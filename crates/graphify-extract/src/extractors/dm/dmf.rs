//! `.dmf` interface-form extractor (windows + controls).

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node as GNode};
use regex::Regex;
use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

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
        origin_file: None,
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
                    origin_file: None,
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
                    origin_file: None,
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
