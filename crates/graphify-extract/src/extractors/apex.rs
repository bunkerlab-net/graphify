//! Salesforce Apex extractor (`.cls` / `.trigger`).
//!
//! Apex has no tree-sitter grammar on crates.io, so structure is recovered with
//! line-oriented regexes — classes, interfaces, enums, methods, triggers, SOQL
//! queries, and DML statements. Mirrors `graphify-py` `extract_apex`.

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};

// Reusable Apex declaration-modifier fragments, composed into the declaration
// regexes below. Mirror the `_ACCESS` / `_SHARING` / `_MOD` / `_ANNOTATION`
// fragments in the Python reference.
const AP_ANNOT: &str = r"(?:\s*@\w+(?:\s*\([^)]*\))?\s*)*";
const AP_ACCESS: &str = r"(?:public|private|protected|global|webService)?";
const AP_SHARING: &str = r"(?:\s+(?:with|without|inherited)\s+sharing)?";
const AP_MOD: &str = r"(?:\s+(?:abstract|virtual|override|static|final|transient|testMethod))?";

#[allow(clippy::expect_used)] // composed-from-literal pattern; build cannot fail
static APEX_CLASS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)^{AP_ANNOT}\s*{AP_ACCESS}{AP_SHARING}{AP_MOD}\s*class\s+(\w+)(?:\s+extends\s+(\w+))?(?:\s+implements\s+([\w,\s]+))?\s*\{{?"
    ))
    .expect("apex class regex")
});

#[allow(clippy::expect_used)] // composed-from-literal pattern
static APEX_IFACE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)^{AP_ANNOT}\s*{AP_ACCESS}{AP_SHARING}{AP_MOD}\s*interface\s+(\w+)(?:\s+extends\s+([\w,\s]+))?\s*\{{?"
    ))
    .expect("apex interface regex")
});

#[allow(clippy::expect_used)] // composed-from-literal pattern
static APEX_ENUM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)^{AP_ANNOT}\s*{AP_ACCESS}{AP_SHARING}{AP_MOD}\s*enum\s+(\w+)\s*\{{?"
    ))
    .expect("apex enum regex")
});

#[allow(clippy::expect_used)] // literal pattern
static APEX_TRIGGER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*trigger\s+(\w+)\s+on\s+(\w+)\s*\(").expect("apex trigger regex")
});

#[allow(clippy::expect_used)] // composed-from-literal pattern
static APEX_METHOD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)^{AP_ANNOT}\s*{AP_ACCESS}{AP_MOD}\s*(?:static\s+)?[\w<>\[\]]+\s+(\w+)\s*\([^)]*\)\s*(?:throws\s+\w+\s*)?\{{?"
    ))
    .expect("apex method regex")
});

#[allow(clippy::expect_used)] // literal pattern
static APEX_ANNOTATION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)@(\w+)").expect("apex annotation regex"));

#[allow(clippy::expect_used)] // composed-from-literal pattern
static APEX_ANNOT_ONLY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?i)^{AP_ANNOT}$")).expect("apex annotation-only regex")
});

#[allow(clippy::expect_used)] // literal pattern
static APEX_SOQL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\[\s*SELECT\b[^\]]+FROM\s+(\w+)").expect("apex soql regex"));

#[allow(clippy::expect_used)] // literal pattern
static APEX_DML_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(insert|update|delete|upsert|merge|undelete)\s+\w").expect("apex dml regex")
});

/// Keywords that look like declaration names to the regexes but are control
/// flow, not real symbols — suppressed so they never become nodes.
const APEX_CONTROL_FLOW: &[&str] = &[
    "if",
    "else",
    "for",
    "while",
    "do",
    "switch",
    "try",
    "catch",
    "finally",
    "return",
    "throw",
    "new",
    "void",
    "null",
    "true",
    "false",
    "this",
    "super",
    "class",
    "interface",
    "enum",
    "trigger",
    "on",
];

fn is_control_flow(name: &str) -> bool {
    let lower = name.to_lowercase();
    APEX_CONTROL_FLOW.contains(&lower.as_str())
}

/// Mutable bookkeeping for the Apex line walk.
struct ApexCtx<'a> {
    str_path: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
}

impl ApexCtx<'_> {
    fn add_node(&mut self, nid: &str, label: &str, line: usize) {
        if self.seen_ids.insert(nid.to_string()) {
            self.nodes.push(Node {
                id: nid.to_string(),
                label: label.to_string(),
                file_type: "code".to_string(),
                source_file: self.str_path.to_string(),
                source_location: Some(format!("L{line}")),
                metadata: None,
                origin_file: None,
            });
        }
    }

    fn add_edge(&mut self, src: &str, tgt: &str, relation: &str, line: usize, confidence: &str) {
        self.edges.push(Edge {
            external: false,
            source: src.to_string(),
            target: tgt.to_string(),
            relation: relation.to_string(),
            confidence: confidence.to_string(),
            source_file: self.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: None,
            confidence_score: None,
        });
    }

    /// Resolve a referenced type label to a node id: prefer an existing
    /// stem-qualified node, fall back to an existing bare-id node, otherwise
    /// create a bare-id stub. Mirrors the base/interface resolution in the
    /// Python reference.
    fn resolve_type(&mut self, stem: &str, label: &str, line: usize) -> String {
        let mut nid = make_id(&[stem, label]);
        if !self.seen_ids.contains(&nid) {
            nid = make_id1(label);
        }
        if !self.seen_ids.contains(&nid) {
            self.add_node(&nid, label, line);
        }
        nid
    }
}

/// Extract classes, interfaces, enums, methods, triggers, SOQL/DML usage from an
/// Apex `.cls` / `.trigger` file. Mirrors `graphify-py` `extract_apex`.
#[must_use]
#[allow(clippy::too_many_lines)] // single line-oriented dispatch mirroring the Python reference; splitting fragments the per-construct branches
pub fn extract_apex(path: &Path) -> FileResult {
    let Ok(source) = std::fs::read_to_string(path) else {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: None,
        };
    };

    let str_path = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_nid = make_id1(&str_path);

    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut ctx = ApexCtx {
        str_path: &str_path,
        nodes: &mut nodes,
        edges: &mut edges,
        seen_ids: &mut seen_ids,
    };

    let filename = path
        .file_name()
        .map_or(String::new(), |f| f.to_string_lossy().into_owned());
    ctx.add_node(&file_nid, &filename, 1);

    let mut current_class_nid: Option<String> = None;
    let mut pending_annotations: Vec<String> = Vec::new();

    for (idx, line_text) in source.lines().enumerate() {
        let lineno = idx + 1;
        let stripped = line_text.trim();

        if stripped.starts_with('@') {
            for cap in APEX_ANNOTATION_RE.captures_iter(stripped) {
                pending_annotations.push(cap[1].to_lowercase());
            }
            // An annotation-only line carries its annotations to the next line
            // (the declaration they decorate), so skip the rest of the loop to
            // preserve `pending_annotations` past the fall-through clear below.
            // When a declaration shares the line (inline annotation, e.g.
            // `@AuraEnabled public static String foo()`), fall through so the
            // declaration regexes — which all carry the `AP_ANNOT` prefix
            // precisely to match inline annotations — can still match it.
            //
            // DIVERGENCE from graphify-py (`extract.py` `extract_apex`), which
            // unconditionally `continue`s here and so silently drops every
            // inline-annotated declaration.
            if APEX_ANNOT_ONLY_RE.is_match(stripped) {
                continue;
            }
        }

        if let Some(tm) = APEX_TRIGGER_RE.captures(stripped) {
            let trig_name = &tm[1];
            let sobject = &tm[2];
            let trig_nid = make_id(&[&stem, trig_name]);
            ctx.add_node(&trig_nid, trig_name, lineno);
            ctx.add_edge(&file_nid, &trig_nid, "contains", lineno, "EXTRACTED");
            let sob_nid = make_id1(sobject);
            ctx.add_node(&sob_nid, sobject, lineno);
            ctx.add_edge(&trig_nid, &sob_nid, "uses", lineno, "INFERRED");
            current_class_nid = Some(trig_nid);
            pending_annotations.clear();
            continue;
        }

        if let Some(cm) = APEX_CLASS_RE.captures(stripped) {
            let class_name = cm[1].to_string();
            if is_control_flow(&class_name) {
                pending_annotations.clear();
                continue;
            }
            let class_nid = make_id(&[&stem, &class_name]);
            ctx.add_node(&class_nid, &class_name, lineno);
            ctx.add_edge(&file_nid, &class_nid, "contains", lineno, "EXTRACTED");
            if let Some(base) = cm.get(2) {
                let base = base.as_str().trim();
                let base_nid = ctx.resolve_type(&stem, base, lineno);
                ctx.add_edge(&class_nid, &base_nid, "extends", lineno, "INFERRED");
            }
            if let Some(ifaces) = cm.get(3) {
                for iface in ifaces.as_str().split(',') {
                    let iface = iface.trim();
                    if !iface.is_empty() {
                        let iface_nid = ctx.resolve_type(&stem, iface, lineno);
                        ctx.add_edge(&class_nid, &iface_nid, "implements", lineno, "INFERRED");
                    }
                }
            }
            current_class_nid = Some(class_nid);
            pending_annotations.clear();
            continue;
        }

        if let Some(im) = APEX_IFACE_RE.captures(stripped) {
            let iface_name = im[1].to_string();
            if is_control_flow(&iface_name) {
                pending_annotations.clear();
                continue;
            }
            let iface_nid = make_id(&[&stem, &iface_name]);
            ctx.add_node(&iface_nid, &iface_name, lineno);
            let parent = current_class_nid
                .clone()
                .unwrap_or_else(|| file_nid.clone());
            ctx.add_edge(&parent, &iface_nid, "contains", lineno, "EXTRACTED");
            pending_annotations.clear();
            continue;
        }

        if let Some(em) = APEX_ENUM_RE.captures(stripped) {
            let enum_name = em[1].to_string();
            if is_control_flow(&enum_name) {
                pending_annotations.clear();
                continue;
            }
            let enum_nid = make_id(&[&stem, &enum_name]);
            ctx.add_node(&enum_nid, &enum_name, lineno);
            let parent = current_class_nid
                .clone()
                .unwrap_or_else(|| file_nid.clone());
            ctx.add_edge(&parent, &enum_nid, "contains", lineno, "EXTRACTED");
            pending_annotations.clear();
            continue;
        }

        if let Some(class_nid) = current_class_nid.clone()
            && let Some(mm) = APEX_METHOD_RE.captures(stripped)
        {
            let method_name = mm[1].to_string();
            if !is_control_flow(&method_name) {
                let method_nid = make_id(&[&class_nid, &method_name]);
                ctx.add_node(&method_nid, &format!(".{method_name}()"), lineno);
                ctx.add_edge(&class_nid, &method_nid, "method", lineno, "EXTRACTED");
                if pending_annotations.iter().any(|a| a == "auraenabled")
                    || pending_annotations.iter().any(|a| a == "invocablemethod")
                {
                    ctx.add_edge(&file_nid, &method_nid, "contains", lineno, "INFERRED");
                }
                pending_annotations.clear();
                continue;
            }
        }

        pending_annotations.clear();

        let src = current_class_nid
            .clone()
            .unwrap_or_else(|| file_nid.clone());
        for sm in APEX_SOQL_RE.captures_iter(line_text) {
            let sobject = &sm[1];
            let sob_nid = make_id1(sobject);
            ctx.add_node(&sob_nid, sobject, lineno);
            ctx.add_edge(&src, &sob_nid, "uses", lineno, "INFERRED");
        }
        for dm in APEX_DML_RE.captures_iter(line_text) {
            let dml_op = dm[1].to_lowercase();
            let dml_nid = make_id1(&format!("dml_{dml_op}"));
            ctx.add_node(&dml_nid, &dml_op, lineno);
            ctx.add_edge(&src, &dml_nid, "uses", lineno, "INFERRED");
        }
    }

    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}
