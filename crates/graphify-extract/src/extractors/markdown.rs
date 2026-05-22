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
#[allow(clippy::too_many_lines)]
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
    });

    // heading_stack: Vec<(level, nid)>
    let mut heading_stack: Vec<(usize, String)> = Vec::new();
    let mut in_code_block = false;
    let mut code_block_lang: Option<String> = None;
    let mut code_block_start: usize = 0;
    let mut code_block_lines: Vec<String> = Vec::new();
    let mut code_block_count: usize = 0;

    for (line_num_0, line_text) in source.lines().enumerate() {
        let line_num = line_num_0 + 1;
        let stripped = line_text.trim();

        if let Some(after_ticks) = stripped.strip_prefix("```") {
            if in_code_block {
                // End of code block
                in_code_block = false;
                code_block_count += 1;
                let label = if let Some(ref lang) = code_block_lang {
                    let mut lbl = format!("code:{lang}");
                    if let Some(first_line) = code_block_lines.first() {
                        let fl = first_line.trim();
                        if !fl.is_empty() {
                            let fl_clipped: String = fl.chars().take(60).collect();
                            lbl = format!("{lbl} ({fl_clipped})");
                        }
                    }
                    lbl
                } else {
                    let mut lbl = format!("code:block{code_block_count}");
                    if let Some(first_line) = code_block_lines.first() {
                        let fl = first_line.trim();
                        if !fl.is_empty() {
                            let fl_clipped: String = fl.chars().take(60).collect();
                            lbl = format!("{lbl} ({fl_clipped})");
                        }
                    }
                    lbl
                };
                let cb_nid = make_id(&[&stem, &format!("codeblock_{code_block_count}")]);
                if seen_ids.insert(cb_nid.clone()) {
                    nodes.push(Node {
                        id: cb_nid.clone(),
                        label,
                        file_type: "document".to_string(),
                        source_file: str_path.clone(),
                        source_location: Some(format!("L{code_block_start}")),
                    });
                }
                let parent = heading_stack
                    .last()
                    .map_or(file_nid.as_str(), |(_, nid)| nid.as_str());
                edges.push(Edge {
                    source: parent.to_string(),
                    target: cb_nid,
                    relation: "contains".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{code_block_start}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
            } else {
                in_code_block = true;
                code_block_lang = {
                    let lang = after_ticks.split_whitespace().next().unwrap_or("");
                    if lang.is_empty() {
                        None
                    } else {
                        Some(lang.to_string())
                    }
                };
                code_block_start = line_num;
                code_block_lines = Vec::new();
            }
            continue;
        }

        if in_code_block {
            code_block_lines.push(line_text.to_string());
            continue;
        }

        // Detect headings
        if let Some(cap) = HEADING_RE.captures(line_text) {
            let level = cap[1].len();
            let title = cap[2].trim().to_string();
            let mut h_nid = make_id(&[&stem, &title]);
            if seen_ids.contains(&h_nid) {
                h_nid = make_id(&[&stem, &title, &line_num.to_string()]);
            }
            if seen_ids.insert(h_nid.clone()) {
                nodes.push(Node {
                    id: h_nid.clone(),
                    label: title,
                    file_type: "document".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{line_num}")),
                });
            }

            // Pop headings at same or deeper level
            while heading_stack.last().is_some_and(|(lvl, _)| *lvl >= level) {
                heading_stack.pop();
            }

            let parent = heading_stack
                .last()
                .map_or(file_nid.as_str(), |(_, nid)| nid.as_str());
            edges.push(Edge {
                source: parent.to_string(),
                target: h_nid.clone(),
                relation: "contains".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.clone(),
                source_location: Some(format!("L{line_num}")),
                weight: 1.0,
                context: None,
                confidence_score: None,
            });
            heading_stack.push((level, h_nid));
        }
    }

    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}
