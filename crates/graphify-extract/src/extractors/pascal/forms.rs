//! Lazarus `.lfm` / Delphi `.dfm` text-form extractors.

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};
use regex::Regex;
use std::collections::HashSet;
use std::path::Path;

/// Parse a Lazarus `.lfm` or Delphi `.dfm` text-form file, emitting component and event nodes.
///
/// Scans line-by-line for `object Name : ClassName` declarations and `OnXxx = Handler` event
/// bindings. Component nodes are connected via `contains` edges; event handlers produce `handles`
/// edges. Shared by `extract_lazarus_form` and `extract_delphi_form`.
#[allow(clippy::too_many_lines)]
fn parse_form_text(text: &str, path: &Path) -> FileResult {
    #[allow(clippy::expect_used)]
    let obj_re = Regex::new(r"(?i)^\s*object\s+\w+\s*:\s*(\w+)").expect("static lfm object regex");
    #[allow(clippy::expect_used)]
    let event_re = Regex::new(r"(?i)^\s*On\w+\s*=\s*(\w+)").expect("static lfm event regex");
    #[allow(clippy::expect_used)]
    let end_re = Regex::new(r"(?i)^\s*end\s*$").expect("static lfm end regex");

    let str_path = path.to_string_lossy().into_owned();
    let stem = file_stem(path);

    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut seen_edge_pairs: HashSet<(String, String, String)> = HashSet::new();

    let add_node = |nodes: &mut Vec<Node>,
                    seen_ids: &mut HashSet<String>,
                    nid: String,
                    label: String,
                    line: usize,
                    str_path: &str| {
        if seen_ids.insert(nid.clone()) {
            nodes.push(Node {
                id: nid,
                label,
                file_type: "code".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                metadata: None,
                origin_file: None,
            });
        }
    };

    let add_edge = |edges: &mut Vec<Edge>,
                    seen_edge_pairs: &mut HashSet<(String, String, String)>,
                    src: String,
                    tgt: String,
                    relation: String,
                    line: usize,
                    context: Option<String>,
                    str_path: &str| {
        let key = (src.clone(), tgt.clone(), relation.clone());
        if seen_edge_pairs.insert(key) {
            edges.push(Edge {
                external: false,
                source: src,
                target: tgt,
                relation,
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                context,
                confidence_score: None,
            });
        }
    };

    let file_nid = make_id1(&str_path);
    add_node(
        &mut nodes,
        &mut seen_ids,
        file_nid.clone(),
        path.file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
        1,
        &str_path,
    );

    let mut stack: Vec<String> = vec![file_nid];

    for (lineno, line) in text.lines().enumerate() {
        let lineno = lineno + 1;
        if let Some(cap) = obj_re.captures(line) {
            let class_name = cap.get(1).map_or("", |m| m.as_str());
            let nid = make_id(&[&stem, class_name]);
            add_node(
                &mut nodes,
                &mut seen_ids,
                nid.clone(),
                class_name.to_string(),
                lineno,
                &str_path,
            );
            let parent = stack.last().cloned().unwrap_or_default();
            add_edge(
                &mut edges,
                &mut seen_edge_pairs,
                parent,
                nid.clone(),
                "contains".to_string(),
                lineno,
                None,
                &str_path,
            );
            stack.push(nid);
            continue;
        }
        if let Some(cap) = event_re.captures(line) {
            if stack.len() > 1 {
                let handler = cap.get(1).map_or("", |m| m.as_str());
                let handler_nid = make_id(&[&stem, handler]);
                add_node(
                    &mut nodes,
                    &mut seen_ids,
                    handler_nid.clone(),
                    format!("{handler}()"),
                    lineno,
                    &str_path,
                );
                let parent = stack.last().cloned().unwrap_or_default();
                add_edge(
                    &mut edges,
                    &mut seen_edge_pairs,
                    parent,
                    handler_nid,
                    "references".to_string(),
                    lineno,
                    Some("event".to_string()),
                    &str_path,
                );
            }
            continue;
        }
        if end_re.is_match(line) && stack.len() > 1 {
            stack.pop();
        }
    }

    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}

// ── extract_lazarus_form (.lfm) ───────────────────────────────────────────────

/// Extract component hierarchy from a Lazarus `.lfm` form file.
#[must_use]
pub fn extract_lazarus_form(path: &Path) -> FileResult {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_form_text(&text, path),
        Err(e) => FileResult::error(e.to_string()),
    }
}

// ── extract_delphi_form (.dfm) ────────────────────────────────────────────────

/// Extract component hierarchy from a Delphi `.dfm` form file.
///
/// Binary DFM files (magic bytes `FF 0A`) are returned as an error.
#[must_use]
pub fn extract_delphi_form(path: &Path) -> FileResult {
    let raw = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return FileResult::error(e.to_string()),
    };
    // Binary DFM detection
    if raw.starts_with(b"\xff\x0a") {
        return FileResult::error(format!(
            "binary DFM (convert to text in Delphi IDE to index): {}",
            path.file_name()
                .map_or(String::new(), |f| f.to_string_lossy().into_owned())
        ));
    }
    let text = String::from_utf8_lossy(&raw).into_owned();
    parse_form_text(&text, path)
}

// ── extract_lazarus_package (.lpk) ───────────────────────────────────────────
