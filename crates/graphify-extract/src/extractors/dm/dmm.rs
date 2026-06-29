//! `.dmm` map-file extractor (tile dictionary type references).

use crate::ids::make_id1;
use crate::types::{Edge, FileResult, Node as GNode};
use regex::Regex;
use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

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
        origin_file: None,
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
