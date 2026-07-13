//! `.razor` / `.cshtml` component extractor.

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};
use regex::Regex;
use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

#[allow(clippy::expect_used)]
static RAZOR_USING_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^@using\s+([\w.]+)").expect("static razor @using regex"));

#[allow(clippy::expect_used)]
static RAZOR_INJECT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^@inject\s+([\w.<>\[\]]+)\s+(\w+)").expect("static razor @inject regex")
});

#[allow(clippy::expect_used)]
static RAZOR_INHERITS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^@inherits\s+([\w.<>\[\]]+)").expect("static razor @inherits regex")
});

#[allow(clippy::expect_used)]
static RAZOR_MODEL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^@model\s+([\w.<>\[\]]+)").expect("static razor @model regex"));

#[allow(clippy::expect_used)]
static RAZOR_PAGE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^@page\s+"([^"]+)""#).expect("static razor @page regex"));

#[allow(clippy::expect_used)]
static RAZOR_COMPONENT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<([A-Z][A-Za-z0-9]+)[\s/>]").expect("static razor component regex")
});

#[allow(clippy::expect_used)]
static RAZOR_CODE_BLOCK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)@code\s*\{").expect("static razor @code regex"));

#[allow(clippy::expect_used)]
static RAZOR_METHOD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:public|private|protected|internal|static|async|override|virtual|abstract)\s+[\w<>\[\],\s]+\s+(\w+)\s*\(",
    )
    .expect("static razor method regex")
});

const RAZOR_HTML_TAGS: &[&str] = &[
    "DOCTYPE", "Html", "Head", "Body", "Div", "Span", "Table", "Form", "Input", "Button", "Select",
    "Option", "Label", "Textarea", "Script", "Style", "Link", "Meta", "Title", "Header", "Footer",
    "Nav", "Main", "Section", "Article", "Aside",
];

// ── .sln ────────────────────────────────────────────────────────────────────

/// Extract directives, component refs, and `@code` methods from a `.razor` /
/// `.cshtml` file. Mirrors `graphify-py` `extract_razor`.
#[must_use]
#[allow(clippy::too_many_lines)] // linear directive dispatch + component scan + @code body parse
pub fn extract_razor(path: &Path) -> FileResult {
    let Ok(src) = std::fs::read_to_string(path) else {
        return FileResult::error(format!("cannot read {}", path.display()));
    };

    let str_path = path.to_string_lossy().into_owned();
    let file_nid = make_id1(&str_path);

    let mut nodes: Vec<Node> = vec![Node {
        id: file_nid.clone(),
        label: path
            .file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
        file_type: "code".to_string(),
        source_file: str_path.clone(),
        source_location: None,
        metadata: None,
        origin_file: None,
        node_type: None,
    }];
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    seen_ids.insert(file_nid.clone());

    let add_ref = |target_name: &str,
                   relation: &str,
                   line: usize,
                   nodes: &mut Vec<Node>,
                   edges: &mut Vec<Edge>,
                   seen_ids: &mut HashSet<String>| {
        let tgt_nid = make_id1(target_name);
        if tgt_nid.is_empty() {
            return;
        }
        if seen_ids.insert(tgt_nid.clone()) {
            nodes.push(Node {
                id: tgt_nid.clone(),
                label: target_name.to_string(),
                file_type: "code".to_string(),
                source_file: str_path.clone(),
                source_location: Some(format!("L{line}")),
                metadata: None,
                origin_file: None,
                node_type: None,
            });
        }
        edges.push(Edge {
            external: false,
            source: file_nid.clone(),
            target: tgt_nid,
            relation: relation.to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: str_path.clone(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: None,
            confidence_score: None,
            deferred: false,
            metadata: None,
        });
    };

    for (idx, line) in src.lines().enumerate() {
        let i = idx + 1;
        if let Some(cap) = RAZOR_USING_RE.captures(line) {
            if let Some(m) = cap.get(1) {
                add_ref(
                    m.as_str(),
                    "imports",
                    i,
                    &mut nodes,
                    &mut edges,
                    &mut seen_ids,
                );
            }
            continue;
        }
        if let Some(cap) = RAZOR_INJECT_RE.captures(line) {
            if let Some(m) = cap.get(1) {
                add_ref(
                    m.as_str(),
                    "imports",
                    i,
                    &mut nodes,
                    &mut edges,
                    &mut seen_ids,
                );
            }
            continue;
        }
        if let Some(cap) = RAZOR_INHERITS_RE.captures(line) {
            if let Some(m) = cap.get(1) {
                add_ref(
                    m.as_str(),
                    "inherits",
                    i,
                    &mut nodes,
                    &mut edges,
                    &mut seen_ids,
                );
            }
            continue;
        }
        if let Some(cap) = RAZOR_MODEL_RE.captures(line) {
            if let Some(m) = cap.get(1) {
                add_ref(
                    m.as_str(),
                    "references",
                    i,
                    &mut nodes,
                    &mut edges,
                    &mut seen_ids,
                );
            }
            continue;
        }
        if let Some(cap) = RAZOR_PAGE_RE.captures(line)
            && let Some(m) = cap.get(1)
        {
            let route = m.as_str();
            let route_nid = make_id(&["route", route]);
            if !route_nid.is_empty() && seen_ids.insert(route_nid.clone()) {
                nodes.push(Node {
                    id: route_nid.clone(),
                    label: format!("route:{route}"),
                    file_type: "concept".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{i}")),
                    metadata: None,
                    origin_file: None,
                    node_type: None,
                });
                edges.push(Edge {
                    external: false,
                    source: file_nid.clone(),
                    target: route_nid,
                    relation: "references".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.clone(),
                    source_location: None,
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                    deferred: false,
                    metadata: None,
                });
            }
        }
    }

    // Component references: capitalised tag names that aren't known HTML elements.
    for m in RAZOR_COMPONENT_RE.captures_iter(&src) {
        let Some(name_m) = m.get(1) else { continue };
        let comp_name = name_m.as_str();
        if RAZOR_HTML_TAGS.contains(&comp_name) {
            continue;
        }
        let abs_pos = name_m.start();
        let line_num = src[..abs_pos].chars().filter(|&c| c == '\n').count() + 1;
        add_ref(
            comp_name,
            "calls",
            line_num,
            &mut nodes,
            &mut edges,
            &mut seen_ids,
        );
    }

    // @code { ... } method extraction. Find each `@code {` opening, walk
    // braces tracking C# lexical context (line comments, block comments,
    // regular strings, verbatim strings, char literals) so braces inside
    // those don't confuse the depth counter.
    //
    // Divergence from `graphify-py` `extract_razor` (intentional): the
    // Python brace counter is purely structural, which means a method
    // body containing `"}{"` would truncate `block_end` early and
    // silently drop every method below that point. Run-aware scanning
    // costs O(n) extra work but produces the right block boundary.
    let stem = file_stem(path);
    let src_bytes = src.as_bytes();
    for cap in RAZOR_CODE_BLOCK_RE.find_iter(&src) {
        let block_start = cap.end();
        let block_end = find_csharp_block_end(src_bytes, block_start);
        if block_end <= block_start {
            continue;
        }
        let block_body = &src[block_start..block_end];
        for mm in RAZOR_METHOD_RE.captures_iter(block_body) {
            let Some(name_m) = mm.get(1) else { continue };
            let method_name = name_m.as_str();
            let abs_pos = block_start + name_m.start();
            let method_line = src[..abs_pos].chars().filter(|&c| c == '\n').count() + 1;
            let method_nid = make_id(&[&stem, method_name]);
            if method_nid.is_empty() {
                continue;
            }
            if seen_ids.insert(method_nid.clone()) {
                nodes.push(Node {
                    id: method_nid.clone(),
                    label: method_name.to_string(),
                    file_type: "code".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{method_line}")),
                    metadata: None,
                    origin_file: None,
                    node_type: None,
                });
            }
            edges.push(Edge {
                external: false,
                source: file_nid.clone(),
                target: method_nid,
                relation: "contains".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.clone(),
                source_location: None,
                weight: 1.0,
                context: None,
                confidence_score: None,
                deferred: false,
                metadata: None,
            });
        }
    }

    FileResult {
        nodes,
        edges,
        raw_calls: Vec::new(),
        error: None,
    }
}

/// Find the byte index of the closing `}` that matches the opening `{` of
/// an `@code {` block.
///
/// Walks `src` from `start` tracking C# lexical state: line comments,
/// block comments, regular strings (with `\` escape), verbatim strings
/// (with `""` escape), interpolated strings, and char literals. Braces
/// inside any of those don't count toward the depth.
///
/// Returns the byte offset of the matching `}` (the byte one past the
/// last byte of the block body). When the closing `}` is missing,
/// returns `src.len()` so the caller fails open and still scans
/// whatever body it has.
#[allow(clippy::too_many_lines)] // linear state-machine dispatch; splitting per-state would just spread the transition table
fn find_csharp_block_end(src: &[u8], start: usize) -> usize {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Code,
        LineComment,
        BlockComment,
        String,
        VerbatimString,
        Char,
    }
    let mut state = State::Code;
    let mut depth: i32 = 1;
    let mut pos = start;
    while pos < src.len() {
        let b = src[pos];
        match state {
            State::Code => {
                let next = src.get(pos + 1).copied().unwrap_or(0);
                if b == b'/' && next == b'/' {
                    state = State::LineComment;
                    pos += 2;
                    continue;
                }
                if b == b'/' && next == b'*' {
                    state = State::BlockComment;
                    pos += 2;
                    continue;
                }
                if (b == b'@' || b == b'$') && next == b'"' {
                    // `@"..."` is verbatim (no `\` escape, `""` is the
                    // embedded-quote). `$"..."` is interpolated — the
                    // embedded text between holes honours the regular
                    // `\"` escape, so route it through the regular
                    // string state.
                    state = if b == b'@' {
                        State::VerbatimString
                    } else {
                        State::String
                    };
                    pos += 2;
                    continue;
                }
                if b == b'"' {
                    state = State::String;
                    pos += 1;
                    continue;
                }
                if b == b'\'' {
                    state = State::Char;
                    pos += 1;
                    continue;
                }
                if b == b'{' {
                    depth += 1;
                } else if b == b'}' {
                    depth -= 1;
                    if depth == 0 {
                        return pos;
                    }
                }
                pos += 1;
            }
            State::LineComment => {
                if b == b'\n' {
                    state = State::Code;
                }
                pos += 1;
            }
            State::BlockComment => {
                if b == b'*' && src.get(pos + 1).copied() == Some(b'/') {
                    state = State::Code;
                    pos += 2;
                } else {
                    pos += 1;
                }
            }
            State::String => {
                if b == b'\\' && pos + 1 < src.len() {
                    // Skip the escaped char (covers `\"`, `\\`, `\n`, ...).
                    pos += 2;
                } else if b == b'"' {
                    state = State::Code;
                    pos += 1;
                } else {
                    pos += 1;
                }
            }
            State::VerbatimString => {
                if b == b'"' && src.get(pos + 1).copied() == Some(b'"') {
                    pos += 2;
                } else if b == b'"' {
                    state = State::Code;
                    pos += 1;
                } else {
                    pos += 1;
                }
            }
            State::Char => {
                if b == b'\\' && pos + 1 < src.len() {
                    pos += 2;
                } else if b == b'\'' {
                    state = State::Code;
                    pos += 1;
                } else {
                    pos += 1;
                }
            }
        }
    }
    src.len()
}
