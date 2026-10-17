//! `.dm` / `.dme` `DreamMaker` source extractor (tree-sitter).

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node as GNode, RawCall};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tree_sitter::Node;

/// Byte range covered by `node`, decoded as UTF-8 (`""` on bad UTF-8).
fn read_text<'b>(node: Node<'_>, source: &'b [u8]) -> &'b str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// 1-based source line of a node's start position.
fn line_of(node: Node<'_>) -> u32 {
    u32::try_from(node.start_position().row)
        .unwrap_or(u32::MAX)
        .saturating_add(1)
}

/// First direct child of `node` whose kind is `kind`.
fn find_child<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cur = node.walk();
    node.children(&mut cur).find(|c| c.kind() == kind)
}

/// Read the include target from a `preproc_include`'s `path` (`file_literal`) node.
fn read_include_path(file_node: Option<Node<'_>>, source: &[u8]) -> String {
    file_node.map_or_else(String::new, |node| {
        read_text(node, source)
            .trim_matches(|c| c == '\'' || c == '"')
            .to_string()
    })
}

/// Node kinds that define a proc body: `name` identifiers each followed by an
/// optional `block`. `verb_definition` is grouped here because the previous
/// grammar parsed `verb` through the same `proc_keyword` rule.
const PROC_KINDS: [&str; 3] = ["proc_definition", "verb_definition", "proc_override"];

/// Statements whose presence directly under a type-path segment makes that
/// segment a declared type (it owns procs or vars).
const TYPE_STATEMENT_KINDS: [&str; 6] = [
    "proc_definition",
    "verb_definition",
    "proc_override",
    "operator_override",
    "var_definition",
    "var_assignment",
];

// ── .dm / .dme ────────────────────────────────────────────────────────────────

/// State threaded through the structural walk of a `.dm` AST.
struct DmCtx<'a, 'tree> {
    source: &'a [u8],
    str_path: &'a str,
    stem: &'a str,
    file_nid: &'a str,
    path: &'a Path,
    nodes: Vec<GNode>,
    edges: Vec<Edge>,
    seen_ids: HashSet<String>,
    /// `(proc_nid, body_block)` pairs collected for the later call-resolution pass.
    function_bodies: Vec<(String, Node<'tree>)>,
}

impl<'tree> DmCtx<'_, 'tree> {
    fn add_node(&mut self, nid: &str, label: &str, line: u32) {
        if !nid.is_empty() && self.seen_ids.insert(nid.to_string()) {
            self.nodes.push(GNode {
                id: nid.to_string(),
                label: label.to_string(),
                file_type: "code".to_string(),
                source_file: self.str_path.to_string(),
                source_location: Some(format!("L{line}")),
                metadata: None,
                origin_file: None,
                node_type: None,
            });
        }
    }

    fn add_edge(&mut self, src: &str, tgt: &str, relation: &str, line: u32, context: Option<&str>) {
        if src.is_empty() || tgt.is_empty() || src == tgt {
            return;
        }
        self.edges.push(Edge {
            source: src.to_string(),
            target: tgt.to_string(),
            relation: relation.to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: self.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: context.map(str::to_string),
            confidence_score: None,
            external: false,
            deferred: false,
            metadata: None,
        });
    }

    /// Ensure a type-path node exists and return its id.
    fn ensure_type(&mut self, path_text: &str, line: u32) -> String {
        let nid = make_id(&[self.stem, path_text]);
        self.add_node(&nid, path_text, line);
        nid
    }

    fn walk(&mut self, node: Node<'tree>, owner: Option<(&str, &str)>, prefix: &str) {
        match node.kind() {
            "preproc_include" => self.include(node),
            "type_definition" | "subtype_definition" => self.walk_type(node, prefix),
            kind if PROC_KINDS.contains(&kind) => self.walk_procs(node, owner),
            "operator_override" => {}
            // Everything else (including ERROR recovery nodes) recurses with
            // the current type context propagated.
            _ => {
                let mut cur = node.walk();
                for c in node.children(&mut cur) {
                    self.walk(c, owner, prefix);
                }
            }
        }
    }

    fn include(&mut self, node: Node<'tree>) {
        let raw = read_include_path(node.child_by_field_name("path"), self.source);
        if raw.is_empty() {
            return;
        }
        // graphify-py normalises with `raw.lstrip("./")`, but Python's
        // str.lstrip treats "./" as a *character set*, so it silently
        // eats the leading `..` of a parent-relative include
        // (`../shared.dm` -> `shared.dm`) and mis-resolves it. Strip only
        // a single leading `/` plus any leading `./` segments, preserving
        // `../` so parent-relative includes resolve correctly. Divergence
        // from graphify-py, which carries this lstrip bug.
        let replaced = raw.replace('\\', "/");
        let mut norm_slice: &str = replaced.strip_prefix('/').unwrap_or(&replaced);
        while let Some(rest) = norm_slice.strip_prefix("./") {
            norm_slice = rest;
        }
        let norm = norm_slice.to_string();
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        let joined = parent.join(&norm);
        let (target, relation, external) = if joined.exists() {
            let resolved = joined.canonicalize().unwrap_or(joined);
            (make_id1(&resolved.to_string_lossy()), "imports_from", false)
        } else {
            (make_id1(&norm), "imports", true)
        };
        let line = line_of(node);
        self.edges.push(Edge {
            source: self.file_nid.to_string(),
            target,
            relation: relation.to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: self.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: Some("import".to_string()),
            confidence_score: None,
            external,
            deferred: false,
            metadata: None,
        });
    }

    /// Declare a type node for `path_text` and link it from the file.
    fn declare_type(&mut self, path_text: &str, line: u32) -> String {
        let nid = self.ensure_type(path_text, line);
        let file_nid = self.file_nid;
        self.add_edge(file_nid, &nid, "contains", line, None);
        nid
    }

    /// Walk one segment of a DM path tree. `type_definition` carries the root
    /// segment and `subtype_definition` each nested one; `name` identifiers
    /// are leaf subtypes. The grammar treats `/` like an indent, so
    /// `/datum/weapon/sword` nests three segments.
    ///
    /// A segment becomes a type node only when the source declares it: it owns
    /// statements, has no children (a bare `/datum`), or opens an indented
    /// block on a later line. Pure path prefixes such as `/datum` in
    /// `/datum/weapon/proc/attack()` get no node, matching the previous
    /// grammar, where the whole `type_path` named a single type.
    fn walk_type(&mut self, node: Node<'tree>, prefix: &str) {
        let mut cur = node.walk();
        let children: Vec<(Node<'tree>, Option<&str>)> = node
            .children(&mut cur)
            .enumerate()
            .filter(|(_, c)| c.is_named())
            .map(|(i, c)| {
                let field = u32::try_from(i)
                    .ok()
                    .and_then(|i| node.field_name_for_child(i));
                (c, field)
            })
            .collect();
        let Some(seg_idx) = children
            .iter()
            .position(|(c, f)| c.kind() == "identifier" && *f != Some("name"))
        else {
            return;
        };
        let seg = children[seg_idx].0;
        let path_text = format!("{prefix}/{}", read_text(seg, self.source).trim());
        let rest = &children[seg_idx + 1..];

        let seg_row = seg.start_position().row;
        let declared = rest.is_empty()
            || rest.iter().any(|(c, _)| {
                TYPE_STATEMENT_KINDS.contains(&c.kind())
                    || (matches!(c.kind(), "identifier" | "subtype_definition")
                        && c.start_position().row > seg_row)
            });
        let type_nid = declared.then(|| self.declare_type(&path_text, line_of(seg)));
        let owner = type_nid.as_deref().map(|nid| (path_text.as_str(), nid));

        for &(c, field) in rest {
            if c.kind() == "identifier" && field == Some("name") {
                let leaf = format!("{path_text}/{}", read_text(c, self.source).trim());
                self.declare_type(&leaf, line_of(c));
            } else {
                self.walk(c, owner, &path_text);
            }
        }
    }

    /// Emit one proc node per `name` in a proc/verb definition or override,
    /// pairing each with the `block` that follows it.
    fn walk_procs(&mut self, node: Node<'tree>, owner: Option<(&str, &str)>) {
        let file_nid = self.file_nid;
        let mut current: Option<String> = None;
        let mut cur = node.walk();
        for (i, c) in node.children(&mut cur).enumerate() {
            let field = u32::try_from(i)
                .ok()
                .and_then(|i| node.field_name_for_child(i));
            if field == Some("name") {
                let proc_name = read_text(c, self.source);
                let line = line_of(c);
                let nid = if let Some((op, on)) = owner {
                    let nid = make_id(&[self.stem, op, proc_name]);
                    self.add_node(&nid, &format!("{op}/{proc_name}()"), line);
                    self.add_edge(on, &nid, "method", line, None);
                    nid
                } else {
                    let nid = make_id(&[self.stem, proc_name]);
                    self.add_node(&nid, &format!("{proc_name}()"), line);
                    self.add_edge(file_nid, &nid, "contains", line, None);
                    nid
                };
                current = Some(nid);
            } else if c.kind() == "block"
                && let Some(nid) = current.take()
            {
                self.function_bodies.push((nid, c));
            }
        }
    }
}

/// State threaded through the call-resolution pass over collected proc bodies.
struct CallCtx<'a> {
    source: &'a [u8],
    str_path: &'a str,
    label_to_nids: &'a HashMap<String, Vec<String>>,
    path_to_nids: &'a HashMap<String, Vec<String>>,
    edges: &'a mut Vec<Edge>,
    raw_calls: &'a mut Vec<RawCall>,
    seen_call_pairs: &'a mut HashSet<(String, String)>,
}

impl CallCtx<'_> {
    /// Resolve a callee name to a single node id and emit a `calls` edge, or
    /// stash it in `raw_calls` when unresolved / ambiguous / self-referential.
    fn emit_call(&mut self, caller_nid: &str, callee: &str, line: u32, is_member: bool) {
        let single = self
            .label_to_nids
            .get(&callee.to_lowercase())
            .filter(|v| v.len() == 1)
            .map(|v| v[0].clone());
        if let Some(tgt) = single
            && tgt != caller_nid
        {
            let pair = (caller_nid.to_string(), tgt.clone());
            if !self.seen_call_pairs.insert(pair) {
                return;
            }
            self.edges.push(Edge {
                source: caller_nid.to_string(),
                target: tgt,
                relation: "calls".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: self.str_path.to_string(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                context: Some("call".to_string()),
                confidence_score: None,
                external: false,
                deferred: false,
                metadata: None,
            });
            return;
        }
        self.raw_calls.push(RawCall {
            caller_nid: caller_nid.to_string(),
            callee: callee.to_string(),
            is_member_call: is_member,
            source_file: self.str_path.to_string(),
            source_location: format!("L{line}"),
            receiver: None,
            receiver_type: None,
            lang: None,
            ..Default::default()
        });
    }

    fn walk_calls(&mut self, node: Node<'_>, caller_nid: &str) {
        // Never descend into a nested definition — those bodies are walked from
        // their own `function_bodies` entry with their own caller id.
        let kind = node.kind();
        if PROC_KINDS.contains(&kind)
            || matches!(
                kind,
                "operator_override" | "type_definition" | "subtype_definition"
            )
        {
            return;
        }
        match kind {
            "call_expression" => {
                // `..()` (BYOND's `super` call) has a `parent_proc` function
                // node and falls through here unemitted; so do computed
                // callees such as `call(o, "x")()`.
                if let Some(func) = node.child_by_field_name("function") {
                    match func.kind() {
                        "identifier" => {
                            let callee = read_text(func, self.source);
                            self.emit_call(caller_nid, callee, line_of(node), false);
                        }
                        "field_expression" => {
                            if let Some(field) = func.child_by_field_name("field") {
                                let callee = read_text(field, self.source);
                                self.emit_call(caller_nid, callee, line_of(node), true);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "new_expression" => {
                if let Some(tp_node) = find_child(node, "type_literal") {
                    let target = read_text(tp_node, self.source).trim().to_lowercase();
                    let single = self
                        .path_to_nids
                        .get(&target)
                        .filter(|v| v.len() == 1)
                        .map(|v| v[0].clone());
                    if let Some(tgt) = single
                        && tgt != caller_nid
                    {
                        let pair = (caller_nid.to_string(), tgt.clone());
                        if self.seen_call_pairs.insert(pair) {
                            self.edges.push(Edge {
                                source: caller_nid.to_string(),
                                target: tgt,
                                relation: "instantiates".to_string(),
                                confidence: "EXTRACTED".to_string(),
                                source_file: self.str_path.to_string(),
                                source_location: Some(format!("L{}", line_of(node))),
                                weight: 1.0,
                                context: Some("call".to_string()),
                                confidence_score: None,
                                external: false,
                                deferred: false,
                                metadata: None,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
        let mut cur = node.walk();
        for child in node.children(&mut cur) {
            self.walk_calls(child, caller_nid);
        }
    }
}

/// Extract types, procs, includes, and calls from a `.dm`/`.dme` file.
#[must_use]
pub fn extract_dm(path: &Path) -> FileResult {
    let source = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return FileResult::error(e.to_string()),
    };
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_dm::LANGUAGE.into())
        .is_err()
    {
        return FileResult::error("failed to set dm language");
    }
    let Some(tree) = parser.parse(&source, None) else {
        return FileResult::error("parse failed");
    };

    let stem = file_stem(path);
    let str_path = path.to_string_lossy().into_owned();
    let file_nid = make_id1(&str_path);
    let file_label = path
        .file_name()
        .map_or(String::new(), |f| f.to_string_lossy().into_owned());

    let mut ctx = DmCtx {
        source: &source,
        str_path: &str_path,
        stem: &stem,
        file_nid: &file_nid,
        path,
        nodes: Vec::new(),
        edges: Vec::new(),
        seen_ids: HashSet::new(),
        function_bodies: Vec::new(),
    };
    ctx.add_node(&file_nid, &file_label, 1);
    ctx.walk(tree.root_node(), None, "");

    let DmCtx {
        nodes,
        mut edges,
        function_bodies,
        ..
    } = ctx;

    // Index *callable* nodes by last path segment (for `proc()` calls) and full
    // type paths by their complete path (for `new /type` instantiation). Only
    // proc nodes are callable — their labels end in "()" — so a bare call never
    // resolves to a non-callable type node (e.g. `widget()` matching the type
    // `/datum/widget`). graphify-py indexes *every* label here, a latent bug we
    // fix rather than replicate. Only unambiguous (single-candidate) names
    // resolve; the rest become `raw_calls` for cross-file resolution.
    let mut label_to_nids: HashMap<String, Vec<String>> = HashMap::new();
    let mut path_to_nids: HashMap<String, Vec<String>> = HashMap::new();
    for n in &nodes {
        let label = n.label.trim_matches(|c| c == '(' || c == ')');
        if n.label.ends_with("()") {
            let last = label.rsplit_once('/').map_or(label, |(_, tail)| tail);
            if !last.is_empty() {
                label_to_nids
                    .entry(last.to_lowercase())
                    .or_default()
                    .push(n.id.clone());
            }
        }
        if label.starts_with('/') {
            path_to_nids
                .entry(label.to_lowercase())
                .or_default()
                .push(n.id.clone());
        }
    }

    let mut raw_calls: Vec<RawCall> = Vec::new();
    let mut seen_call_pairs: HashSet<(String, String)> = HashSet::new();
    {
        let mut cc = CallCtx {
            source: &source,
            str_path: &str_path,
            label_to_nids: &label_to_nids,
            path_to_nids: &path_to_nids,
            edges: &mut edges,
            raw_calls: &mut raw_calls,
            seen_call_pairs: &mut seen_call_pairs,
        };
        for (proc_nid, block) in &function_bodies {
            cc.walk_calls(*block, proc_nid);
        }
    }

    FileResult {
        nodes,
        edges,
        raw_calls,
        error: None,
    }
}

// ── .dmi (BYOND icon sheets) ───────────────────────────────────────────────────
