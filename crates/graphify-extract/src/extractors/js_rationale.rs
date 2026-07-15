//! JS/TS rationale-comment + doc-reference extraction (6d3a6f1).
//!
//! Parity with [`super::python_rationale`]: Python files get rationale nodes from
//! docstrings and `# NOTE:`-style comments, but JS/TS comments were discarded
//! entirely. This post-pass recovers two high-value signals in mixed corpora:
//!   1. rationale comments (`// NOTE:`, `// WHY:`, block-comment `* NOTE:`
//!      variants) as `file_type = "rationale"` nodes with `rationale_for` edges;
//!   2. architecture-decision references (`ADR-0011`, `RFC 793`) that teams cite
//!      in file/function headers, as `file_type = "doc_ref"` nodes with `cites`
//!      edges — the natural join points between code and design docs.

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};

/// Comment prefixes (line `//` and block `*` forms) that mark a rationale note.
const JS_RATIONALE_PREFIXES: [&str; 14] = [
    "// NOTE:",
    "// IMPORTANT:",
    "// HACK:",
    "// WHY:",
    "// RATIONALE:",
    "// TODO:",
    "// FIXME:",
    "* NOTE:",
    "* IMPORTANT:",
    "* HACK:",
    "* WHY:",
    "* RATIONALE:",
    "* TODO:",
    "* FIXME:",
];

// Doc-reference tokens worth first-classing: ADR-NNNN (any zero padding) and
// RFC NNNN / RFC-NNNN. Case-insensitive.
#[allow(clippy::unwrap_used)] // literal pattern; build cannot panic
static JS_DOC_REF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(ADR[- ]?\d{1,5}|RFC[- ]?\d{1,5})\b").unwrap());

// Only scan for doc references inside comments, not string literals or code.
#[allow(clippy::unwrap_used)] // literal pattern; build cannot panic
static JS_COMMENT_LINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(//|/\*|\*)").unwrap());

// Splits a doc-ref token into its kind (`ADR`/`RFC`) and number for canonicalisation.
#[allow(clippy::unwrap_used)] // literal pattern; build cannot panic
static JS_DOC_REF_PARTS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z]+)[- ]?(\d+)").unwrap());

/// Post-pass: append rationale-comment and doc-reference nodes/edges to `result`
/// from the JS/TS source at `path`. Mirrors Python `_extract_js_rationale`.
pub(super) fn extract_js_rationale(path: &Path, result: &mut FileResult) {
    let Ok(source_text) = std::fs::read_to_string(path) else {
        return;
    };
    let stem = file_stem(path);
    let str_path = path.to_string_lossy().into_owned();
    let file_nid = make_id1(&str_path);
    let mut seen_ids: HashSet<String> = result.nodes.iter().map(|n| n.id.clone()).collect();
    let mut seen_doc_refs: HashSet<String> = HashSet::new();

    for (idx, line_text) in source_text.lines().enumerate() {
        let line = idx + 1;
        let stripped = line_text.trim();
        if JS_RATIONALE_PREFIXES
            .iter()
            .any(|p| stripped.starts_with(p))
        {
            let text = stripped.trim_start_matches(['/', '*', ' ']);
            add_rationale(
                result,
                &mut seen_ids,
                &stem,
                &str_path,
                &file_nid,
                text,
                line,
            );
        }
        if JS_COMMENT_LINE_RE.is_match(line_text) {
            for m in JS_DOC_REF_RE.captures_iter(stripped) {
                if let Some(tok) = m.get(1) {
                    add_doc_ref(
                        result,
                        &mut seen_ids,
                        &mut seen_doc_refs,
                        &str_path,
                        &file_nid,
                        tok.as_str(),
                        line,
                    );
                }
            }
        }
    }
}

/// Append a rationale node (deduped by id) + a `rationale_for` edge to the file.
fn add_rationale(
    result: &mut FileResult,
    seen_ids: &mut HashSet<String>,
    stem: &str,
    str_path: &str,
    file_nid: &str,
    text: &str,
    line: usize,
) {
    let label: String = text
        .replace(['\r', '\n'], " ")
        .trim()
        .chars()
        .take(80)
        .collect::<String>()
        .trim()
        .to_string();
    let rid = make_id(&[stem, "rationale", &line.to_string()]);
    if seen_ids.insert(rid.clone()) {
        result.nodes.push(Node {
            id: rid.clone(),
            label,
            file_type: "rationale".to_string(),
            source_file: str_path.to_string(),
            source_location: Some(format!("L{line}")),
            metadata: None,
            origin_file: None,
            node_type: None,
        });
    }
    result.edges.push(rationale_edge(
        &rid,
        file_nid,
        "rationale_for",
        str_path,
        line,
    ));
}

/// Append a doc-ref node + a `cites` edge (file → ref), deduped by canonical label.
fn add_doc_ref(
    result: &mut FileResult,
    seen_ids: &mut HashSet<String>,
    seen_doc_refs: &mut HashSet<String>,
    str_path: &str,
    file_nid: &str,
    token: &str,
    line: usize,
) {
    let Some(caps) = JS_DOC_REF_PARTS_RE.captures(token) else {
        return;
    };
    let kind = caps.get(1).map_or("", |m| m.as_str()).to_uppercase();
    let num = caps.get(2).map_or("", |m| m.as_str());
    // Normalise "adr 11" / "ADR-0011" to a canonical label so references to the
    // same document collapse to one node. ADR is zero-padded to 4 digits.
    let label = if kind == "ADR" {
        format!("ADR-{num:0>4}")
    } else {
        format!("{kind}-{num}")
    };
    if !seen_doc_refs.insert(label.clone()) {
        return;
    }
    let rid = make_id(&["docref", &label]);
    if seen_ids.insert(rid.clone()) {
        result.nodes.push(Node {
            id: rid.clone(),
            label,
            file_type: "doc_ref".to_string(),
            source_file: str_path.to_string(),
            source_location: Some(format!("L{line}")),
            metadata: None,
            origin_file: None,
            node_type: None,
        });
    }
    result
        .edges
        .push(rationale_edge(file_nid, &rid, "cites", str_path, line));
}

/// A weight-1.0 EXTRACTED edge with the given endpoints/relation.
fn rationale_edge(source: &str, target: &str, relation: &str, str_path: &str, line: usize) -> Edge {
    Edge {
        external: false,
        source: source.to_string(),
        target: target.to_string(),
        relation: relation.to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: str_path.to_string(),
        source_location: Some(format!("L{line}")),
        weight: 1.0,
        context: None,
        confidence_score: None,
        deferred: false,
        metadata: None,
    }
}
