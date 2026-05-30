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
///
/// Emits a node per file and per heading, with `contains` edges nesting
/// headings by level. Fenced code blocks (both backtick and tilde fences) are
/// skipped during parsing so their contents are not misread as headings, but no
/// node is emitted for them — they were always orphans and inflated the
/// disconnected-component count (#1077).
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

    let mut heading_stack: Vec<(usize, String)> = Vec::new();
    // The currently-open fence as `(marker_char, run_length)`, or `None`
    // outside a fenced block. Tracking the marker char (rather than a bool)
    // lets a `~~~` inside a ``` block — or vice versa — not prematurely close
    // the block; tracking the run length enforces the CommonMark rule that a
    // closing fence must repeat the opening marker at least as many times, so a
    // nested ``` inside a ```` block does not close the outer block early.
    let mut fence: Option<(char, usize)> = None;

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
        // Skip over fenced code blocks so their contents are not parsed as
        // headings, but emit no nodes/edges for them (#1077): they were always
        // orphans (a single contains edge to the parent doc) and inflated the
        // disconnected-component count.
        //
        // Divergence from graphify-py: the Python parser only recognises ```
        // fences, so a `~~~` code block leaks its contents as phantom heading
        // nodes. Both ``` and ~~~ are valid CommonMark fences, so the Rust port
        // honours both.
        // CommonMark allows a fence to be indented by at most three spaces; four
        // or more leading spaces make the line an indented code block, not a
        // fence. Count leading spaces explicitly rather than trimming so an
        // over-indented ``` is not mistaken for a fence.
        let leading_spaces = line_text.chars().take_while(|&c| c == ' ').count();
        let trimmed = &line_text[leading_spaces..];
        let marker = if leading_spaces <= 3 && trimmed.starts_with("```") {
            Some('`')
        } else if leading_spaces <= 3 && trimmed.starts_with("~~~") {
            Some('~')
        } else {
            None
        };
        if let Some(marker) = marker {
            let marker_len = trimmed.chars().take_while(|&c| c == marker).count();
            match fence {
                None => fence = Some((marker, marker_len)),
                // Close only on the same marker repeated at least as many times
                // as the opening fence (CommonMark). A shorter or mismatched run
                // inside the block does not close it.
                Some((open_ch, open_len)) if open_ch == marker && marker_len >= open_len => {
                    fence = None;
                }
                Some(_) => {}
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        if let Some(cap) = HEADING_RE.captures(line_text) {
            handle_heading(&mut ctx, &mut heading_stack, &cap, line_num);
        }
    }

    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
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
