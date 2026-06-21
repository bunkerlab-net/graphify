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

/// Read the include target string from a `preproc_include`'s `file` node.
fn read_include_path(file_node: Option<Node<'_>>, source: &[u8]) -> String {
    let Some(node) = file_node else {
        return String::new();
    };
    if node.kind() == "string_literal" {
        let mut cur = node.walk();
        let mut parts = String::new();
        for c in node.children(&mut cur) {
            if c.kind() == "string_content" {
                parts.push_str(read_text(c, source));
            }
        }
        parts
    } else {
        read_text(node, source)
            .trim_matches(|c| c == '\'' || c == '"')
            .to_string()
    }
}

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
        });
    }

    /// Ensure a type-path node exists and return its id.
    fn ensure_type(&mut self, path_text: &str, line: u32) -> String {
        let nid = make_id(&[self.stem, path_text]);
        self.add_node(&nid, path_text, line);
        nid
    }

    #[allow(clippy::too_many_lines)] // linear AST dispatch; splitting fragments the type/proc shape
    fn walk(
        &mut self,
        node: Node<'tree>,
        parent_type_path: Option<&str>,
        parent_type_nid: Option<&str>,
    ) {
        let t = node.kind();
        let line = line_of(node);
        let file_nid = self.file_nid;
        let source = self.source;

        match t {
            "preproc_include" => {
                let raw = read_include_path(node.child_by_field_name("file"), source);
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
                self.edges.push(Edge {
                    source: file_nid.to_string(),
                    target,
                    relation: relation.to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: self.str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: Some("import".to_string()),
                    confidence_score: None,
                    external,
                });
            }
            "type_definition" => {
                let Some(tp_node) = find_child(node, "type_path") else {
                    return;
                };
                let type_path_str = read_text(tp_node, source).trim().to_string();
                let type_nid = self.ensure_type(&type_path_str, line);
                self.add_edge(file_nid, &type_nid, "contains", line, None);
                if let Some(body) = find_child(node, "type_body") {
                    let mut cur = body.walk();
                    for c in body.children(&mut cur) {
                        self.walk(c, Some(&type_path_str), Some(&type_nid));
                    }
                }
            }
            "type_proc_definition" | "type_proc_override" => {
                let (Some(ptp), Some(ptn)) = (parent_type_path, parent_type_nid) else {
                    return;
                };
                let Some(name_node) = node.child_by_field_name("name") else {
                    return;
                };
                let proc_name = read_text(name_node, source);
                let proc_nid = make_id(&[self.stem, ptp, proc_name]);
                self.add_node(&proc_nid, &format!("{ptp}/{proc_name}()"), line);
                self.add_edge(ptn, &proc_nid, "method", line, None);
                if let Some(block) = find_child(node, "block") {
                    self.function_bodies.push((proc_nid, block));
                }
            }
            "proc_definition" | "proc_override" => {
                let (owner_path, owner_nid) = match find_child(node, "type_path") {
                    Some(tp) => {
                        let op = read_text(tp, source).trim().to_string();
                        let on = self.ensure_type(&op, line);
                        self.add_edge(file_nid, &on, "contains", line, None);
                        (Some(op), Some(on))
                    }
                    None => (None, None),
                };
                let Some(name_node) = node.child_by_field_name("name") else {
                    return;
                };
                let proc_name = read_text(name_node, source).to_string();
                let block = find_child(node, "block");
                let proc_nid = if let (Some(op), Some(on)) = (&owner_path, &owner_nid) {
                    let nid = make_id(&[self.stem, op, &proc_name]);
                    self.add_node(&nid, &format!("{op}/{proc_name}()"), line);
                    self.add_edge(on, &nid, "method", line, None);
                    nid
                } else {
                    let nid = make_id(&[self.stem, &proc_name]);
                    self.add_node(&nid, &format!("{proc_name}()"), line);
                    self.add_edge(file_nid, &nid, "contains", line, None);
                    nid
                };
                if let Some(block) = block {
                    self.function_bodies.push((proc_nid, block));
                }
            }
            "operator_override" | "type_operator_override" => {}
            // Transparent containers (`type_body`, `type_body_intended`,
            // `type_body_braced`, etc.) and everything else recurse with the
            // current type context propagated.
            _ => {
                let mut cur = node.walk();
                for c in node.children(&mut cur) {
                    self.walk(c, parent_type_path, parent_type_nid);
                }
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
        });
    }

    fn walk_calls(&mut self, node: Node<'_>, caller_nid: &str) {
        // Never descend into a nested definition — those bodies are walked from
        // their own `function_bodies` entry with their own caller id.
        if matches!(
            node.kind(),
            "proc_definition"
                | "proc_override"
                | "type_proc_definition"
                | "type_proc_override"
                | "type_definition"
        ) {
            return;
        }
        match node.kind() {
            "call_expression" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let callee = read_text(name_node, self.source);
                    // `..` is BYOND's `super` call — not a real callee.
                    if !callee.is_empty() && callee != ".." {
                        self.emit_call(caller_nid, callee, line_of(node), false);
                    }
                }
            }
            "field_proc_expression" => {
                if let Some(proc_field) = node.child_by_field_name("proc") {
                    let callee = read_text(proc_field, self.source);
                    if !callee.is_empty() {
                        self.emit_call(caller_nid, callee, line_of(node), true);
                    }
                }
            }
            "new_expression" => {
                if let Some(tp_node) = find_child(node, "type_path") {
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
    ctx.walk(tree.root_node(), None, None);

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
