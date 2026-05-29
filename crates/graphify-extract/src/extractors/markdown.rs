//! Markdown extractor — pure line-by-line parsing (no tree-sitter).

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};
use regex::Regex;

#[allow(clippy::expect_used)] // literal regex pattern; cannot fail
static HEADING_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(#{1,6})\s+(.+)").expect("static heading regex"));

/// Extract structural nodes and edges from a Markdown file.
#[must_use]
pub fn extract_markdown(path: &Path) -> FileResult {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            return FileResult {
                nodes: vec![],
                edges: vec![],
                raw_calls: vec![],
                error: Some(e.to_string()),
            };
        }
    };

    let stem = file_stem(path);
    let str_path = path.to_string_lossy().into_owned();
    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    let file_nid = make_id1(&str_path);
    seen_ids.insert(file_nid.clone());
    nodes.push(Node {
        id: file_nid.clone(),
        label: path
            .file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
        file_type: "document".to_string(),
        source_file: str_path.clone(),
        source_location: Some("L1".to_string()),
        metadata: None,
    });

    let mut state = MarkdownState {
        heading_stack: Vec::new(),
        in_code_block: false,
        code_block_lang: None,
        code_block_start: 0,
        code_block_lines: Vec::new(),
        code_block_count: 0,
    };

    let mut ctx = LineCtx {
        stem: &stem,
        file_nid: &file_nid,
        str_path: &str_path,
        nodes: &mut nodes,
        edges: &mut edges,
        seen_ids: &mut seen_ids,
    };
    for (line_num_0, line_text) in source.lines().enumerate() {
        let line_num = line_num_0 + 1;
        let stripped = line_text.trim();
        if let Some(after_ticks) = stripped.strip_prefix("```") {
            handle_fence(&mut state, &mut ctx, after_ticks, line_num);
            continue;
        }
        if state.in_code_block {
            state.code_block_lines.push(line_text.to_string());
            continue;
        }
        if let Some(cap) = HEADING_RE.captures(line_text) {
            handle_heading(&mut ctx, &mut state.heading_stack, &cap, line_num);
        }
    }

    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}

/// Mutable parser state threaded through the markdown line walker.
struct MarkdownState {
    heading_stack: Vec<(usize, String)>,
    in_code_block: bool,
    code_block_lang: Option<String>,
    code_block_start: usize,
    code_block_lines: Vec<String>,
    code_block_count: usize,
}

/// Per-file context passed to the line handlers.
struct LineCtx<'a> {
    stem: &'a str,
    file_nid: &'a str,
    str_path: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
}

/// Toggle the code-block state on a `` ``` `` fence. On close, emit the node + edge.
fn handle_fence(
    state: &mut MarkdownState,
    ctx: &mut LineCtx<'_>,
    after_ticks: &str,
    line_num: usize,
) {
    if state.in_code_block {
        state.in_code_block = false;
        state.code_block_count += 1;
        let label = code_block_label(
            state.code_block_lang.as_deref(),
            state.code_block_count,
            &state.code_block_lines,
        );
        let cb_nid = make_id(&[ctx.stem, &format!("codeblock_{}", state.code_block_count)]);
        if ctx.seen_ids.insert(cb_nid.clone()) {
            ctx.nodes.push(Node {
                id: cb_nid.clone(),
                label,
                file_type: "document".to_string(),
                source_file: ctx.str_path.to_string(),
                source_location: Some(format!("L{}", state.code_block_start)),
                metadata: None,
            });
        }
        let parent = state
            .heading_stack
            .last()
            .map_or(ctx.file_nid, |(_, nid)| nid.as_str());
        ctx.edges.push(Edge {
            external: false,
            source: parent.to_string(),
            target: cb_nid,
            relation: "contains".to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: ctx.str_path.to_string(),
            source_location: Some(format!("L{}", state.code_block_start)),
            weight: 1.0,
            context: None,
            confidence_score: None,
        });
    } else {
        state.in_code_block = true;
        state.code_block_lang = {
            let lang = after_ticks.split_whitespace().next().unwrap_or("");
            if lang.is_empty() {
                None
            } else {
                Some(lang.to_string())
            }
        };
        state.code_block_start = line_num;
        state.code_block_lines = Vec::new();
    }
}

/// Build the `"code:<lang> (<first line>)"` label for a closed code block.
fn code_block_label(lang: Option<&str>, count: usize, lines: &[String]) -> String {
    let mut lbl = lang.map_or_else(|| format!("code:block{count}"), |l| format!("code:{l}"));
    if let Some(first_line) = lines.first() {
        let fl = first_line.trim();
        if !fl.is_empty() {
            let fl_clipped: String = fl.chars().take(60).collect();
            lbl = format!("{lbl} ({fl_clipped})");
        }
    }
    lbl
}

/// Emit a heading node, attach it to its parent, and update the heading stack.
fn handle_heading(
    ctx: &mut LineCtx<'_>,
    heading_stack: &mut Vec<(usize, String)>,
    cap: &regex::Captures<'_>,
    line_num: usize,
) {
    let level = cap[1].len();
    let title = cap[2].trim().to_string();
    let mut h_nid = make_id(&[ctx.stem, &title]);
    if ctx.seen_ids.contains(&h_nid) {
        h_nid = make_id(&[ctx.stem, &title, &line_num.to_string()]);
    }
    if ctx.seen_ids.insert(h_nid.clone()) {
        ctx.nodes.push(Node {
            id: h_nid.clone(),
            label: title,
            file_type: "document".to_string(),
            source_file: ctx.str_path.to_string(),
            source_location: Some(format!("L{line_num}")),
            metadata: None,
        });
    }
    while heading_stack.last().is_some_and(|(lvl, _)| *lvl >= level) {
        heading_stack.pop();
    }
    let parent = heading_stack
        .last()
        .map_or(ctx.file_nid, |(_, nid)| nid.as_str());
    ctx.edges.push(Edge {
        external: false,
        source: parent.to_string(),
        target: h_nid.clone(),
        relation: "contains".to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: ctx.str_path.to_string(),
        source_location: Some(format!("L{line_num}")),
        weight: 1.0,
        context: None,
        confidence_score: None,
    });
    heading_stack.push((level, h_nid));
}
