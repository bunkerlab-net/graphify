//! BYOND `DreamMaker` extractors.
//!
//! Ports the DM section of `graphify-py/graphify/extract.py`:
//! - [`extract_dm`] — `.dm`/`.dme` source via tree-sitter (types, procs,
//!   includes, calls). DM identity is path-based (`/datum/object/proc/New()`),
//!   so this uses a bespoke walk rather than the generic class-body walker.
//! - [`extract_dmi`] — `.dmi` icon sheets (PNG with a BYOND metadata text chunk).
//! - [`extract_dmm`] — `.dmm` map files (tile dictionary type references).
//! - [`extract_dmf`] — `.dmf` interface forms (windows + controls).

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use tree_sitter::Node;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node as GNode, RawCall};

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

/// Decompress up to 1 MiB of a zTXt zlib stream (best effort).
///
/// graphify-py lets a corrupt zlib stream raise; we degrade gracefully and keep
/// whatever decompressed cleanly. The 1 MiB cap mirrors graphify-py's
/// `max_length` guard against decompression bombs.
fn decompress_capped(compressed: &[u8]) -> String {
    let mut out = Vec::new();
    let mut decoder = flate2::read::ZlibDecoder::new(compressed).take(1024 * 1024);
    let _ = decoder.read_to_end(&mut out);
    String::from_utf8_lossy(&out).into_owned()
}

/// Pull the BYOND metadata text out of a `.dmi` PNG, or `""` on failure.
///
/// Scans PNG chunks for a `tEXt`/`zTXt` chunk keyed `Description`; zTXt payloads
/// are zlib-decompressed (capped). Mirrors graphify-py `_read_dmi_description`.
fn read_dmi_description(data: &[u8]) -> String {
    const PNG_SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !data.starts_with(PNG_SIG) {
        return String::new();
    }
    let mut i = 8usize;
    while i + 8 <= data.len() {
        let length = usize::try_from(u32::from_be_bytes([
            data[i],
            data[i + 1],
            data[i + 2],
            data[i + 3],
        ]))
        .unwrap_or(0);
        let chunk_type = &data[i + 4..i + 8];
        let payload_start = i + 8;
        let payload_end = payload_start.saturating_add(length).min(data.len());
        let payload = &data[payload_start..payload_end];
        if chunk_type == b"tEXt" || chunk_type == b"zTXt" {
            let Some(nul) = payload.iter().position(|&b| b == 0) else {
                return String::new();
            };
            if &payload[..nul] == b"Description" {
                if chunk_type == b"zTXt" {
                    // zTXt: keyword \0 compression_method(1 byte) compressed_data
                    return decompress_capped(payload.get(nul + 2..).unwrap_or(&[]));
                }
                // tEXt: keyword \0 text
                return String::from_utf8_lossy(payload.get(nul + 1..).unwrap_or(&[])).into_owned();
            }
        }
        i = i.saturating_add(8).saturating_add(length).saturating_add(4);
    }
    String::new()
}

/// Extract icon state names from a `.dmi` (BYOND PNG icon sheet).
#[must_use]
pub fn extract_dmi(path: &Path) -> FileResult {
    let data = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return FileResult::error(e.to_string()),
    };
    let str_path = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_nid = make_id1(&str_path);
    let file_label = path
        .file_name()
        .map_or(String::new(), |f| f.to_string_lossy().into_owned());

    let mut nodes = vec![GNode {
        id: file_nid.clone(),
        label: file_label,
        file_type: "code".to_string(),
        source_file: str_path.clone(),
        source_location: Some("L1".to_string()),
        metadata: None,
    }];
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen: HashSet<String> = HashSet::from([file_nid.clone()]);

    let description = read_dmi_description(&data);
    if description.is_empty() {
        return FileResult {
            nodes,
            edges,
            raw_calls: Vec::new(),
            error: None,
        };
    }

    let mut line_no: u32 = 0;
    for raw_line in description.lines() {
        line_no += 1;
        let stripped = raw_line.trim();
        if !stripped.starts_with("state =") {
            continue;
        }
        let value = stripped.split_once('=').map_or("", |(_, v)| v).trim();
        let state_name = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            &value[1..value.len() - 1]
        } else {
            value
        };
        if state_name.is_empty() {
            continue;
        }
        let nid = make_id(&[&stem, "state", state_name]);
        if !seen.insert(nid.clone()) {
            continue;
        }
        nodes.push(GNode {
            id: nid.clone(),
            label: format!("\"{state_name}\""),
            file_type: "code".to_string(),
            source_file: str_path.clone(),
            source_location: Some(format!("L{line_no}")),
            metadata: None,
        });
        edges.push(Edge {
            source: file_nid.clone(),
            target: nid,
            relation: "contains".to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: str_path.clone(),
            source_location: Some(format!("L{line_no}")),
            weight: 1.0,
            context: None,
            confidence_score: None,
            external: false,
        });
    }

    FileResult {
        nodes,
        edges,
        raw_calls: Vec::new(),
        error: None,
    }
}

// ── .dmm (BYOND map files) ─────────────────────────────────────────────────────

/// Matches the start of a `.dmm` grid section: `(x,y,z) = ...`.
#[allow(clippy::expect_used)] // literal pattern; compiles on first use
static DMM_GRID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\(\s*\d+\s*,\s*\d+\s*,\s*\d+\s*\)\s*=").expect("static dmm_grid regex")
});

/// Split a tile-dictionary body on top-level commas, respecting `(){}[]`
/// nesting and string literals. Mirrors graphify-py `_split_dmm_tile`.
fn split_dmm_tile(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    for ch in body.chars() {
        if escape {
            buf.push(ch);
            escape = false;
            continue;
        }
        if in_string {
            buf.push(ch);
            if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                buf.push(ch);
            }
            '(' | '{' | '[' => {
                depth += 1;
                buf.push(ch);
            }
            ')' | '}' | ']' => {
                depth -= 1;
                buf.push(ch);
            }
            ',' if depth == 0 => {
                out.push(buf.trim().to_string());
                buf.clear();
            }
            _ => buf.push(ch),
        }
    }
    let tail = buf.trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

/// Strip a `{var=val; ...}` override suffix off a tile entry, leaving the type path.
fn dmm_type_path(entry: &str) -> String {
    entry
        .find('{')
        .map_or(entry, |b| &entry[..b])
        .trim()
        .to_string()
}

/// Extract type-path references from a `.dmm` map file's tile dictionary.
#[must_use]
pub fn extract_dmm(path: &Path) -> FileResult {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > 50 * 1024 * 1024 => {
            return FileResult::error("file too large (>50 MB)");
        }
        Ok(_) => {}
        Err(e) => return FileResult::error(e.to_string()),
    }
    let data = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return FileResult::error(e.to_string()),
    };
    let text = String::from_utf8_lossy(&data).into_owned();

    let str_path = path.to_string_lossy().into_owned();
    let file_nid = make_id1(&str_path);
    let file_label = path
        .file_name()
        .map_or(String::new(), |f| f.to_string_lossy().into_owned());
    let nodes = vec![GNode {
        id: file_nid.clone(),
        label: file_label,
        file_type: "code".to_string(),
        source_file: str_path.clone(),
        source_location: Some("L1".to_string()),
        metadata: None,
    }];
    let mut edges: Vec<Edge> = Vec::new();

    // Only the dictionary section (before the grid) names type paths.
    let dict_text = match DMM_GRID_RE.find(&text) {
        Some(m) => &text[..m.start()],
        None => &text[..],
    };

    let mut seen_targets: HashSet<String> = HashSet::new();
    let mut buf = String::new();
    let mut open_line: u32 = 0;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    let mut line_idx: u32 = 0;
    for line in dict_text.lines() {
        line_idx += 1;
        for ch in line.chars() {
            if escape {
                escape = false;
            } else if in_string {
                if ch == '\\' {
                    escape = true;
                } else if ch == '"' {
                    in_string = false;
                }
            } else if ch == '"' {
                in_string = true;
            } else if ch == '(' {
                if depth == 0 {
                    open_line = line_idx;
                }
                depth += 1;
            } else if ch == ')' {
                depth -= 1;
            }
            buf.push(ch);
        }
        buf.push('\n');
        if depth == 0 && !buf.is_empty() {
            let chunk = std::mem::take(&mut buf);
            let (Some(lp), Some(rp)) = (chunk.find('('), chunk.rfind(')')) else {
                continue;
            };
            if rp <= lp {
                continue;
            }
            for entry in split_dmm_tile(&chunk[lp + 1..rp]) {
                let tpath = dmm_type_path(&entry);
                if !tpath.starts_with('/') {
                    continue;
                }
                let tgt = make_id1(&tpath);
                if !seen_targets.insert(tgt.clone()) {
                    continue;
                }
                edges.push(Edge {
                    source: file_nid.clone(),
                    target: tgt,
                    relation: "uses".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{open_line}")),
                    weight: 1.0,
                    context: Some("map".to_string()),
                    confidence_score: None,
                    external: false,
                });
            }
        }
    }

    FileResult {
        nodes,
        edges,
        raw_calls: Vec::new(),
        error: None,
    }
}

// ── .dmf (BYOND interface forms) ────────────────────────────────────────────────

#[allow(clippy::expect_used)] // literal pattern; compiles on first use
static DMF_WINDOW_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*window\s+"([^"]+)"\s*$"#).expect("static dmf_window regex"));
#[allow(clippy::expect_used)] // literal pattern; compiles on first use
static DMF_ELEM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*elem\s+"([^"]+)"\s*$"#).expect("static dmf_elem regex"));
#[allow(clippy::expect_used)] // literal pattern; compiles on first use
static DMF_TYPE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*type\s*=\s*(\S+)\s*$").expect("static dmf_type regex"));

/// Extract windows and controls from a `.dmf` interface file.
#[must_use]
#[allow(clippy::too_many_lines)] // linear line scanner; verbose node/edge literals, not real complexity
pub fn extract_dmf(path: &Path) -> FileResult {
    let data = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return FileResult::error(e.to_string()),
    };
    let text = String::from_utf8_lossy(&data).into_owned();

    let str_path = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_nid = make_id1(&str_path);
    let file_label = path
        .file_name()
        .map_or(String::new(), |f| f.to_string_lossy().into_owned());
    let mut nodes = vec![GNode {
        id: file_nid.clone(),
        label: file_label,
        file_type: "code".to_string(),
        source_file: str_path.clone(),
        source_location: Some("L1".to_string()),
        metadata: None,
    }];
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen: HashSet<String> = HashSet::from([file_nid.clone()]);

    let mut current_window_nid: Option<String> = None;
    let mut current_elem_nid: Option<String> = None;
    let mut current_elem_name: Option<String> = None;

    let mut line_idx: u32 = 0;
    for line in text.lines() {
        line_idx += 1;
        if let Some(cap) = DMF_WINDOW_RE.captures(line) {
            let name = &cap[1];
            let nid = make_id(&[&stem, "window", name]);
            if seen.insert(nid.clone()) {
                nodes.push(GNode {
                    id: nid.clone(),
                    label: format!("window \"{name}\""),
                    file_type: "code".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{line_idx}")),
                    metadata: None,
                });
                edges.push(Edge {
                    source: file_nid.clone(),
                    target: nid.clone(),
                    relation: "contains".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{line_idx}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                    external: false,
                });
            }
            current_window_nid = Some(nid);
            current_elem_nid = None;
            current_elem_name = None;
            continue;
        }
        if let Some(cap) = DMF_ELEM_RE.captures(line)
            && let Some(win) = current_window_nid.clone()
        {
            let name = cap[1].to_string();
            let nid = make_id(&[&stem, "elem", &win, &name]);
            if seen.insert(nid.clone()) {
                nodes.push(GNode {
                    id: nid.clone(),
                    label: format!("elem \"{name}\""),
                    file_type: "code".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{line_idx}")),
                    metadata: None,
                });
                edges.push(Edge {
                    source: win,
                    target: nid.clone(),
                    relation: "contains".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{line_idx}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                    external: false,
                });
            }
            current_elem_nid = Some(nid);
            current_elem_name = Some(name);
            continue;
        }
        if let Some(cap) = DMF_TYPE_RE.captures(line)
            && let (Some(elem_nid), Some(elem_name)) =
                (current_elem_nid.as_deref(), current_elem_name.as_deref())
        {
            let ctype = &cap[1];
            for n in &mut nodes {
                if n.id == elem_nid && !n.label.contains(" [") {
                    n.label = format!("elem \"{elem_name}\" [{ctype}]");
                    break;
                }
            }
        }
    }

    FileResult {
        nodes,
        edges,
        raw_calls: Vec::new(),
        error: None,
    }
}
