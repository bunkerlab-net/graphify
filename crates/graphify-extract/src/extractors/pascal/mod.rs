//! Pascal / Delphi / Lazarus extractors.
//!
//! `extract_pascal` — regex-based (tree-sitter-pascal is not on crates.io).
//! `extract_lazarus_form` — .lfm component hierarchy.
//! `extract_delphi_form` — .dfm component hierarchy.
//! `extract_lazarus_package` — .lpk XML package metadata.

mod forms;
mod package;

pub use forms::{extract_delphi_form, extract_lazarus_form};
pub use package::extract_lazarus_package;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node, RawCall};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

#[allow(clippy::expect_used)] // literal pattern; build cannot panic
static PAS_TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)'(?:''|[^'])*'|\{[^}]*\}|\(\*.*?\*\)|//[^\n]*")
        .expect("static pascal token regex")
});

#[allow(clippy::expect_used)]
static PAS_MODULE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(unit|program|library)\s+([A-Za-z_][\w.]*)\s*;")
        .expect("static pascal module regex")
});

#[allow(clippy::expect_used)]
static PAS_USES_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)\buses\b\s*([^;]+);").expect("static pascal uses regex"));

#[allow(clippy::expect_used)]
static PAS_TYPE_HEADER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?P<name>[A-Za-z_]\w*)(?:\s*<[^>]+>)?\s*=\s*(?:packed\s+)?(?P<kind>class|interface)\b(?:\s*\(\s*(?P<bases>[^)]*)\s*\))?",
    )
    .expect("static pascal type header regex")
});

#[allow(clippy::expect_used)]
static PAS_END_SEMI_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bend\s*;").expect("static pascal end-semi regex"));

#[allow(clippy::expect_used)]
static PAS_METHOD_DECL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:procedure|function|constructor|destructor)\s+(?P<name>[A-Za-z_]\w*)(?:\s*\([^)]*\))?(?:\s*:\s*[\w<>,\s.]+)?\s*;",
    )
    .expect("static pascal method decl regex")
});

#[allow(clippy::expect_used)]
static PAS_IMPL_HEADER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:procedure|function|constructor|destructor)\s+(?P<qual>[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)?)(?:\s*<[^>]+>)?(?:\s*\([^)]*\))?(?:\s*:\s*[\w<>,\s.]+)?\s*;",
    )
    .expect("static pascal impl header regex")
});

#[allow(clippy::expect_used)]
static PAS_BEGIN_END_TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(begin|end|case|try|asm|record)\b")
        .expect("static pascal begin-end token regex")
});

#[allow(clippy::expect_used)]
static PAS_CALL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b([A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*)\s*[(;]").expect("static pascal call regex")
});

static PAS_KEYWORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "begin",
        "end",
        "if",
        "then",
        "else",
        "while",
        "do",
        "for",
        "to",
        "downto",
        "repeat",
        "until",
        "case",
        "of",
        "try",
        "finally",
        "except",
        "with",
        "inherited",
        "result",
        "var",
        "const",
        "type",
        "nil",
        "true",
        "false",
        "exit",
        "break",
        "continue",
        "uses",
        "unit",
        "program",
        "library",
        "interface",
        "implementation",
        "initialization",
        "finalization",
        "procedure",
        "function",
        "constructor",
        "destructor",
        "class",
        "record",
        "object",
        "array",
        "string",
        "integer",
        "boolean",
        "real",
        "char",
        "writeln",
        "write",
        "readln",
        "read",
        "assigned",
        "length",
        "high",
        "low",
        "inc",
        "dec",
        "new",
        "dispose",
        "setlength",
        "copy",
        "pos",
        "trim",
        "format",
        "inttostr",
        "strtoint",
        "ord",
        "chr",
        "sizeof",
        "create",
        "free",
        "destroy",
    ]
    .into_iter()
    .collect()
});

// ── Project-root and unit-name resolution ─────────────────────────────────────

/// Heuristically determine the Pascal project root by walking up until `.pas` files become sparse.
///
/// Travels up to 12 directories up from `from_path`, picking the deepest ancestor that contains
/// at least two `.pas` files. Falls back to `from_path`'s parent if no better candidate is found.
fn pascal_project_root(from_path: &Path) -> std::path::PathBuf {
    let mut best = from_path.parent().unwrap_or(from_path).to_path_buf();
    let mut current: std::path::PathBuf = best.clone();
    for _ in 0..12 {
        if current.components().count() <= 1 {
            break;
        }
        let pas_count = current.read_dir().map_or(0, |rd| {
            rd.filter_map(std::result::Result::ok)
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case("pas"))
                })
                .count()
        });
        let dpr_count = current.read_dir().map_or(0, |rd| {
            rd.filter_map(std::result::Result::ok)
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case("dpr"))
                })
                .count()
        });
        if pas_count >= 2 || dpr_count >= 1 {
            best.clone_from(&current);
        }
        let parent = current.parent().map(std::path::Path::to_path_buf);
        match parent {
            Some(p) if p != current => current = p,
            _ => break,
        }
    }
    best
}

/// Resolve a Pascal unit name to a project-relative NID string.
///
/// Searches sibling directories of `from_path` for a `.pas`/`.pp` file whose stem matches
/// `unit_name` (case-insensitive). Falls back to a bare NID derived from `unit_name` if no
/// file is found. Mirrors Python `_pascal_resolve_unit`.
fn pascal_resolve_unit(from_path: &Path, unit_name: &str) -> String {
    let root = pascal_project_root(from_path);
    let lower = unit_name.to_lowercase();
    for ext in &[".pas", ".pp", ".dpr", ".dpk", ".inc"] {
        if let Ok(rd) = root.read_dir() {
            for entry in rd.filter_map(std::result::Result::ok) {
                let p = entry.path();
                if p.extension()
                    .is_some_and(|e| format!(".{}", e.to_string_lossy()).to_lowercase() == *ext)
                    && p.file_stem()
                        .is_some_and(|s| s.to_string_lossy().to_lowercase() == lower)
                {
                    return make_id1(&p.to_string_lossy());
                }
            }
        }
        // Also rglob-style search via walkdir
        for entry in walkdir::WalkDir::new(&root).min_depth(1).max_depth(8) {
            let Ok(entry) = entry else { continue };
            let p = entry.path();
            if p.extension()
                .is_some_and(|e| format!(".{}", e.to_string_lossy()).to_lowercase() == *ext)
                && p.file_stem()
                    .is_some_and(|s| s.to_string_lossy().to_lowercase() == lower)
            {
                return make_id1(&p.to_string_lossy());
            }
        }
    }
    make_id1(unit_name)
}

/// Try to find a `.pas`/`.pp` file that defines `class_name` and return its NID.
///
/// Scans files in the same project tree for a case-insensitive match of the class name.
/// Returns `None` when no file can be found, allowing the caller to skip the edge.
fn pascal_resolve_class(from_path: &Path, class_name: &str) -> Option<String> {
    let prefix = class_name.chars().next().unwrap_or(' ');
    let unit_name = if prefix == 'T' || prefix == 'I' {
        &class_name[1..]
    } else {
        class_name
    };
    let lower = unit_name.to_lowercase();
    let root = pascal_project_root(from_path);
    for ext in &[".pas", ".pp", ".dpr", ".dpk"] {
        for entry in walkdir::WalkDir::new(&root).min_depth(1).max_depth(8) {
            let Ok(entry) = entry else { continue };
            let p = entry.path();
            if p.extension()
                .is_some_and(|e| format!(".{}", e.to_string_lossy()).to_lowercase() == *ext)
                && p.file_stem()
                    .is_some_and(|s| s.to_string_lossy().to_lowercase() == lower)
            {
                let found_stem = file_stem(p);
                return Some(make_id(&[&found_stem, class_name]));
            }
        }
    }
    None
}

// ── Regex helpers ─────────────────────────────────────────────────────────────

/// Remove Pascal comments (`{...}`, `(*...*)`), `//` line comments, and string literals from text.
///
/// Necessary pre-processing step before regex-based section and identifier extraction, so that
/// comment text does not accidentally match code patterns.
fn pascal_strip_comments(text: &str) -> String {
    PAS_TOKEN_RE
        .replace_all(text, |caps: &regex::Captures<'_>| {
            let tok = caps.get(0).map_or("", |m| m.as_str());
            if tok.starts_with('\'') {
                tok.to_string()
            } else {
                // preserve newlines, blank out everything else
                tok.chars()
                    .map(|c| if c == '\n' { '\n' } else { ' ' })
                    .collect()
            }
        })
        .into_owned()
}

/// Split stripped Pascal source into `(interface_section, impl_start_line, impl_section, impl_line)`.
///
/// Returns byte-slice views of the `interface` and `implementation` sections, together with the
/// 1-based line numbers of where each section starts. Used to separate declaration from definition
/// for class and method extraction.
fn pascal_split_sections(text: &str) -> (&str, usize, &str, usize) {
    #[allow(clippy::expect_used)] // literal patterns
    let iface_re = Regex::new(r"(?i)\binterface\b").expect("literal pattern is valid");
    #[allow(clippy::expect_used)]
    let impl_re = Regex::new(r"(?i)\bimplementation\b").expect("literal pattern is valid");

    let iface_m = iface_re.find(text);
    let impl_m = impl_re.find(text);
    if let (Some(im), Some(mm)) = (iface_m, impl_m) {
        let iface_off = im.end();
        let impl_off = mm.end();
        #[allow(clippy::expect_used)]
        let end_re =
            Regex::new(r"(?i)\b(initialization|finalization)\b").expect("literal pattern is valid");
        let impl_end = end_re
            .find(&text[impl_off..])
            .map_or(text.len(), |m| impl_off + m.start());
        (
            &text[iface_off..mm.start()],
            iface_off,
            &text[impl_off..impl_end],
            impl_off,
        )
    } else {
        ("", 0, text, 0)
    }
}

/// Parse a Pascal `uses` clause, returning a list of unit names.
///
/// Handles multi-line clauses and strips trailing semicolons/commas. Returns an empty `Vec`
/// when no `uses` keyword is found.
fn pascal_split_uses(s: &str) -> Vec<String> {
    #[allow(clippy::expect_used)] // literal pattern; build cannot panic
    static IN_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\s+in\s+").expect("static pascal uses-in regex"));
    #[allow(clippy::expect_used)] // literal pattern; build cannot panic
    static VALID_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[A-Za-z_][\w.]*$").expect("static pascal uses-valid regex"));
    let mut out = Vec::new();
    for chunk in s.split(',') {
        let part = IN_RE.splitn(chunk.trim(), 2).next().unwrap_or("").trim();
        let part = part.trim_end_matches(';').trim();
        if !part.is_empty() && VALID_RE.is_match(part) {
            out.push(part.to_string());
        }
    }
    out
}

/// Parse the base-class list from a Pascal class declaration string.
///
/// Handles `TClass = class(TBase1, IFace2)` syntax, returning the parent names as a `Vec`.
/// Returns an empty `Vec` when no parenthesised base list is present.
fn pascal_split_bases(s: &str) -> Vec<String> {
    #[allow(clippy::expect_used)]
    let strip_re = Regex::new(r"<.*$").expect("literal pattern is valid");
    #[allow(clippy::expect_used)]
    let valid_re = Regex::new(r"^[A-Za-z_]\w*$").expect("literal pattern is valid");
    let mut out: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let mut buf = String::new();
    for ch in s.chars() {
        match ch {
            '<' => {
                depth += 1;
                buf.push(ch);
            }
            '>' => {
                depth = depth.saturating_sub(1);
                buf.push(ch);
            }
            ',' if depth == 0 => {
                let name = strip_re.replace(&buf, "").trim().to_string();
                if !name.is_empty() {
                    out.push(name);
                }
                buf.clear();
            }
            _ => buf.push(ch),
        }
    }
    let name = strip_re.replace(&buf, "").trim().to_string();
    if !name.is_empty() {
        out.push(name);
    }
    out.into_iter().filter(|n| valid_re.is_match(n)).collect()
}

/// Locate the `begin`…`end` body of a Pascal routine starting at `start` byte offset.
///
/// Counts nested `begin`/`end` pairs to handle compound statements. Returns `(body_start,
/// body_end)` as byte offsets, or `(start, start)` when no body is found.
fn pascal_find_body(text: &str, start: usize) -> (usize, usize) {
    #[allow(clippy::expect_used)]
    let begin_re = Regex::new(r"(?i)\bbegin\b").expect("literal pattern is valid");
    let Some(m) = begin_re.find(&text[start..]) else {
        return (0, 0);
    };
    let body_start = start + m.end();
    let mut depth = 1usize;
    for tok in PAS_BEGIN_END_TOKEN_RE.find_iter(&text[body_start..]) {
        let kw = tok.as_str().to_lowercase();
        if matches!(kw.as_str(), "begin" | "case" | "try" | "asm" | "record") {
            depth += 1;
        } else if kw == "end" {
            depth -= 1;
            if depth == 0 {
                return (body_start, body_start + tok.start());
            }
        }
    }
    (body_start, text.len())
}

/// Return the 1-based line number corresponding to `offset` bytes into `text`.
fn lineno(text: &str, offset: usize) -> usize {
    text[..offset].chars().filter(|&c| c == '\n').count() + 1
}

// ── Regex-based Pascal extractor ──────────────────────────────────────────────

/// A per-file scoped call resolver for Pascal/Delphi, mirroring Python
/// `_resolve_pascal_callee_factory`. Resolution order for a call to `name_lower`
/// from `caller_nid`: (1) a method on the caller's own class, (2) a method on an
/// ancestor class (BFS up `inherits` edges), (3) a file-level free function,
/// (4) an unambiguous file-wide match. `None` when ambiguous at every level —
/// the god-node guard also used by `resolve_ruby_member_calls`.
struct PascalScopeResolver {
    class_bases: HashMap<String, Vec<String>>,
    class_procs: HashMap<String, HashMap<String, Vec<String>>>,
    module_procs: HashMap<String, Vec<String>>,
    global_procs: HashMap<String, Vec<String>>,
    proc_owner: HashMap<String, String>,
}

impl PascalScopeResolver {
    fn build(
        records: &[(String, usize, String, String, String)],
        edges: &[Edge],
        module_nid: &str,
    ) -> Self {
        let mut class_bases: HashMap<String, Vec<String>> = HashMap::new();
        for e in edges {
            if e.relation == "inherits" {
                class_bases
                    .entry(e.source.clone())
                    .or_default()
                    .push(e.target.clone());
            }
        }
        let mut class_procs: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
        let mut module_procs: HashMap<String, Vec<String>> = HashMap::new();
        let mut global_procs: HashMap<String, Vec<String>> = HashMap::new();
        let mut proc_owner: HashMap<String, String> = HashMap::new();
        for (proc_nid, _line, _body, container, name_lower) in records {
            proc_owner.insert(proc_nid.clone(), container.clone());
            global_procs
                .entry(name_lower.clone())
                .or_default()
                .push(proc_nid.clone());
            if container == module_nid {
                module_procs
                    .entry(name_lower.clone())
                    .or_default()
                    .push(proc_nid.clone());
            } else {
                class_procs
                    .entry(container.clone())
                    .or_default()
                    .entry(name_lower.clone())
                    .or_default()
                    .push(proc_nid.clone());
            }
        }
        Self {
            class_bases,
            class_procs,
            module_procs,
            global_procs,
            proc_owner,
        }
    }

    /// The single node id `name_lower` resolves to from `caller_nid`, or `None`
    /// when unresolvable or ambiguous at every scope level.
    fn resolve(&self, caller_nid: &str, name_lower: &str) -> Option<&str> {
        if let Some(owner) = self.proc_owner.get(caller_nid) {
            match lookup_proc(self.class_procs.get(owner), name_lower) {
                ProcLookup::Unique(nid) => return Some(nid),
                ProcLookup::Ambiguous => return None,
                ProcLookup::Absent => {}
            }
            let mut seen_bases: HashSet<&str> = HashSet::new();
            let mut queue: Vec<&str> = self
                .class_bases
                .get(owner)
                .map(|v| v.iter().map(String::as_str).collect())
                .unwrap_or_default();
            let mut head = 0;
            while head < queue.len() {
                let base = queue[head];
                head += 1;
                if !seen_bases.insert(base) {
                    continue;
                }
                match lookup_proc(self.class_procs.get(base), name_lower) {
                    ProcLookup::Unique(nid) => return Some(nid),
                    ProcLookup::Ambiguous => return None,
                    ProcLookup::Absent => {}
                }
                if let Some(bases) = self.class_bases.get(base) {
                    queue.extend(bases.iter().map(String::as_str));
                }
            }
        }
        match lookup_flat(self.module_procs.get(name_lower)) {
            ProcLookup::Unique(nid) => return Some(nid),
            ProcLookup::Ambiguous => return None,
            ProcLookup::Absent => {}
        }
        // Global tier: only when exactly one candidate anywhere in the file.
        match self.global_procs.get(name_lower) {
            Some(v) if v.len() == 1 => Some(v[0].as_str()),
            _ => None,
        }
    }
}

/// Outcome of looking a proc name up in one scope level.
enum ProcLookup<'a> {
    /// Exactly one candidate — resolve to it.
    Unique(&'a str),
    /// Name present but owned by multiple procs — bail (god-node guard).
    Ambiguous,
    /// Name absent at this level — keep searching outward.
    Absent,
}

/// Look `name_lower` up in a class's `name -> [nid]` proc map.
fn lookup_proc<'a>(
    class_map: Option<&'a HashMap<String, Vec<String>>>,
    name_lower: &str,
) -> ProcLookup<'a> {
    lookup_flat(class_map.and_then(|m| m.get(name_lower)))
}

/// Look up in a flat `[nid]` candidate list (module free functions / a class bucket).
fn lookup_flat(candidates: Option<&Vec<String>>) -> ProcLookup<'_> {
    match candidates {
        Some(v) if v.len() == 1 => ProcLookup::Unique(v[0].as_str()),
        Some(_) => ProcLookup::Ambiguous,
        None => ProcLookup::Absent,
    }
}
#[allow(clippy::too_many_lines)]
/// Extract Pascal classes, methods, uses, and inheritance using regex scanning.
///
/// Strips comments, splits interface/implementation sections, then applies regex patterns to
/// find class declarations, method definitions, `uses` clauses, and constructor calls. Used as
/// the primary extraction path since there is no tree-sitter grammar for Pascal on crates.io.
/// Mirrors Python `_extract_pascal_regex`.
fn extract_pascal_regex(path: &Path) -> FileResult {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return FileResult::error(e.to_string()),
    };

    let str_path = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut seen_call_pairs: HashSet<(String, String)> = HashSet::new();

    let add_node = |nodes: &mut Vec<Node>,
                    seen_ids: &mut HashSet<String>,
                    nid: &str,
                    label: &str,
                    line: usize,
                    str_path: &str| {
        if seen_ids.insert(nid.to_string()) {
            nodes.push(Node {
                id: nid.to_string(),
                label: label.to_string(),
                file_type: "code".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                metadata: None,
                origin_file: None,
                node_type: None,
            });
        }
    };

    let make_edge = |src: &str,
                     tgt: &str,
                     relation: &str,
                     line: usize,
                     context: Option<&str>,
                     str_path: &str|
     -> Edge {
        Edge {
            external: false,
            source: src.to_string(),
            target: tgt.to_string(),
            relation: relation.to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: context.map(str::to_string),
            confidence_score: None,
            deferred: false,
            metadata: None,
        }
    };

    let file_nid = make_id1(&str_path);
    add_node(
        &mut nodes,
        &mut seen_ids,
        &file_nid,
        &path
            .file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
        1,
        &str_path,
    );

    let stripped = pascal_strip_comments(&raw);

    let mut module_nid = file_nid.clone();
    if let Some(mod_m) = PAS_MODULE_RE.find(&stripped)
        && let Some(cap) = PAS_MODULE_RE.captures(&stripped)
    {
        let mod_name = cap.get(2).map_or("", |m| m.as_str());
        let nid = make_id(&[&stem, mod_name]);
        let line = lineno(&stripped, mod_m.start());
        add_node(&mut nodes, &mut seen_ids, &nid, mod_name, line, &str_path);
        edges.push(make_edge(
            &file_nid, &nid, "contains", line, None, &str_path,
        ));
        module_nid = nid;
    }

    let (iface_text, iface_off, impl_text, impl_off) = pascal_split_sections(&stripped);

    // Uses clauses
    for (section_text, section_off) in &[(iface_text, iface_off), (impl_text, impl_off)] {
        for um in PAS_USES_RE.find_iter(section_text) {
            if let Some(cap) = PAS_USES_RE.captures(&section_text[um.start()..]) {
                let line = lineno(&stripped, section_off + um.start());
                let uses_list = cap.get(1).map_or("", |m| m.as_str());
                for unit_name in pascal_split_uses(uses_list) {
                    let tgt_nid = pascal_resolve_unit(path, &unit_name);
                    edges.push(make_edge(
                        &module_nid,
                        &tgt_nid,
                        "imports",
                        line,
                        Some("import"),
                        &str_path,
                    ));
                }
            }
        }
    }

    // Type declarations
    let (search_text, search_off) = if iface_text.is_empty() {
        (stripped.as_str(), 0)
    } else {
        (iface_text, iface_off)
    };
    let mut pos = 0;
    while pos < search_text.len() {
        let Some(hm) = PAS_TYPE_HEADER_RE.find(&search_text[pos..]) else {
            break;
        };
        let abs_start = pos + hm.start();
        let Some(cap) = PAS_TYPE_HEADER_RE.captures(&search_text[pos..]) else {
            pos += hm.end();
            continue;
        };
        let type_name = cap.name("name").map_or("", |m| m.as_str());
        let bases_raw = cap.name("bases").map_or("", |m| m.as_str());
        let line = lineno(&stripped, search_off + abs_start);

        let cls_nid = make_id(&[&stem, type_name]);
        add_node(
            &mut nodes,
            &mut seen_ids,
            &cls_nid,
            type_name,
            line,
            &str_path,
        );
        edges.push(make_edge(
            &module_nid,
            &cls_nid,
            "contains",
            line,
            None,
            &str_path,
        ));

        for base_name in pascal_split_bases(bases_raw) {
            let same_file_nid = make_id(&[&stem, &base_name]);
            let base_nid = if seen_ids.contains(&same_file_nid) {
                // Base class already declared earlier in THIS file — reuse its real
                // node instead of the cross-file/stub lookup below (which assumes
                // one-class-per-file and would duplicate a same-file base) (#1739).
                same_file_nid
            } else if let Some(resolved) = pascal_resolve_class(path, &base_name) {
                // Cross-file base found on disk — its real node arrives via THAT
                // file's own extraction. Do NOT add a stub here: it would carry this
                // file's source_file (wrong) and collide with the real node under
                // cross-file id disambiguation, forking one class into two salted ids
                // and breaking the cross-file `inherits`-chain resolver (#1739).
                resolved
            } else {
                let stub = make_id1(&base_name);
                if seen_ids.insert(stub.clone()) {
                    nodes.push(Node {
                        id: stub.clone(),
                        label: base_name.clone(),
                        file_type: "code".to_string(),
                        source_file: str_path.clone(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                        origin_file: None,
                        node_type: None,
                    });
                }
                stub
            };
            edges.push(make_edge(
                &cls_nid, &base_nid, "inherits", line, None, &str_path,
            ));
        }

        // Find class body up to next end;
        let rel_start = pos + hm.end();
        let body_text = if let Some(end_m) = PAS_END_SEMI_RE.find(&search_text[rel_start..]) {
            &search_text[rel_start..rel_start + end_m.start()]
        } else {
            &search_text[rel_start..]
        };
        let body_off = search_off + rel_start;

        for mm in PAS_METHOD_DECL_RE.find_iter(body_text) {
            if let Some(mcap) = PAS_METHOD_DECL_RE.captures(&body_text[mm.start()..]) {
                let mname = mcap.name("name").map_or("", |m| m.as_str());
                let mline = lineno(&stripped, body_off + mm.start());
                let method_nid = make_id(&[&cls_nid, mname]);
                add_node(
                    &mut nodes,
                    &mut seen_ids,
                    &method_nid,
                    &format!("{mname}()"),
                    mline,
                    &str_path,
                );
                edges.push(make_edge(
                    &cls_nid,
                    &method_nid,
                    "method",
                    mline,
                    None,
                    &str_path,
                ));
            }
        }

        pos = rel_start
            + PAS_END_SEMI_RE
                .find(&search_text[rel_start..])
                .map_or(search_text.len() - rel_start, |m| m.end());
    }

    // Implementation headers
    let mut impl_records: Vec<(String, usize, String, String, String)> = Vec::new();
    let mut impl_pos = 0;
    while impl_pos < impl_text.len() {
        let Some(fm) = PAS_IMPL_HEADER_RE.find(&impl_text[impl_pos..]) else {
            break;
        };
        let Some(cap) = PAS_IMPL_HEADER_RE.captures(&impl_text[impl_pos..]) else {
            impl_pos += fm.end();
            continue;
        };
        let qualified = cap.name("qual").map_or("", |m| m.as_str());
        let line = lineno(&stripped, impl_off + impl_pos + fm.start());

        // Qualified `TClass.Method` impl headers match the class by exact-case id,
        // porting graphify-py (`_make_id(stem, cls_part)` + `cls_nid in seen_ids`).
        // Pascal is case-insensitive, so a differently-cased `tclass.Method` would
        // fall back to the module; that matches the reference and consistent casing
        // is the norm, so it is left as-is rather than diverging with a folded index.
        let (container, relation, label, name_lower) = if let Some(dot) = qualified.find('.') {
            let cls_part = &qualified[..dot];
            let method_part = &qualified[dot + 1..];
            let cls_nid = make_id(&[&stem, cls_part]);
            if seen_ids.contains(&cls_nid) {
                (
                    cls_nid,
                    "method",
                    format!("{method_part}()"),
                    method_part.to_lowercase(),
                )
            } else {
                (
                    module_nid.clone(),
                    "contains",
                    format!("{qualified}()"),
                    qualified.to_lowercase(),
                )
            }
        } else {
            (
                module_nid.clone(),
                "contains",
                format!("{qualified}()"),
                qualified.to_lowercase(),
            )
        };

        let proc_nid = make_id(&[&stem, qualified]);
        add_node(
            &mut nodes,
            &mut seen_ids,
            &proc_nid,
            &label,
            line,
            &str_path,
        );
        edges.push(make_edge(
            &container, &proc_nid, relation, line, None, &str_path,
        ));

        let after = impl_pos + fm.end();
        let (body_start, body_end) = pascal_find_body(impl_text, after);
        let body_text = if body_start > 0 {
            impl_text[body_start..body_end].to_string()
        } else {
            String::new()
        };
        impl_records.push((proc_nid, line, body_text, container, name_lower));
        impl_pos += fm.end();
    }

    // Intra-file call edges, scoped by the caller's own class, then its ancestor
    // chain (via `inherits` edges already emitted above), then file-level free
    // functions, then an unambiguous file-wide match. Same-named methods on
    // unrelated classes (property accessors, generated TLB wrapper classes — a
    // common Pascal/Delphi pattern) no longer collapse onto an arbitrary
    // cross-class edge; an unresolvable name is reported as a `raw_call` for the
    // cross-file inherited-call resolver instead of guessed or dropped (#1739).
    let resolver = PascalScopeResolver::build(&impl_records, &edges, &module_nid);
    let mut raw_calls: Vec<RawCall> = Vec::new();
    for (caller_nid, caller_line, body_text, _container, _name_lower) in &impl_records {
        for cm in PAS_CALL_RE.find_iter(body_text) {
            let Some(cap) = PAS_CALL_RE.captures(&body_text[cm.start()..]) else {
                continue;
            };
            let callee_full = cap.get(1).map_or("", |m| m.as_str());
            let callee_name = callee_full
                .split('.')
                .next_back()
                .unwrap_or("")
                .to_lowercase();
            if PAS_KEYWORDS.contains(callee_name.as_str()) {
                continue;
            }
            let call_line = caller_line
                + body_text[..cm.start()]
                    .chars()
                    .filter(|&c| c == '\n')
                    .count();
            let Some(target_nid) = resolver.resolve(caller_nid, &callee_name) else {
                // Not resolvable within this file (e.g. inherited from a base class
                // declared in another file) — report for the cross-file resolver
                // (`resolve_pascal_inherited_calls`) rather than guess or drop it.
                raw_calls.push(RawCall {
                    caller_nid: caller_nid.clone(),
                    callee: callee_name,
                    is_member_call: false,
                    source_file: str_path.clone(),
                    source_location: format!("L{call_line}"),
                    receiver: None,
                    receiver_type: None,
                    lang: None,
                    ..Default::default()
                });
                continue;
            };
            if target_nid == caller_nid {
                continue;
            }
            let pair = (caller_nid.clone(), target_nid.to_string());
            if seen_call_pairs.insert(pair) {
                edges.push(make_edge(
                    caller_nid,
                    target_nid,
                    "calls",
                    call_line,
                    Some("call"),
                    &str_path,
                ));
            }
        }
    }

    // A class method declared in the interface section and defined in the
    // implementation section both emit a `method` edge to the same node id, so
    // dedup on (source, target, relation) — keeping the first occurrence — to stop
    // the graph carrying doubled method/contains/inherits edges (d2d1f68). Mirrors
    // the `seen_ids` node guard.
    let mut seen_edges: HashSet<(String, String, String)> = HashSet::new();
    edges.retain(|e| seen_edges.insert((e.source.clone(), e.target.clone(), e.relation.clone())));

    FileResult {
        nodes,
        edges,
        raw_calls,
        error: None,
    }
}

// ── Public extractor: extract_pascal ─────────────────────────────────────────

/// Extract units, classes, procedures, uses-imports, and calls from Pascal/Delphi files.
#[must_use]
pub fn extract_pascal(path: &Path) -> FileResult {
    extract_pascal_regex(path)
}

// ── Shared form parser for .lfm / .dfm ───────────────────────────────────────
