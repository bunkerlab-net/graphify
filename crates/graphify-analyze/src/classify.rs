//! Node classification helpers and static extension/label tables.
//!
//! Extracted from `lib.rs` to isolate the `LANG_FAMILY` / `JSON_NOISE_LABELS`
//! statics and the four `is_*` / `file_category` / `top_level_dir` predicates
//! that are used across multiple sibling modules.

use graphify_build::Graph;
use graphify_detect::{FileType, classify_file};
use indexmap::{IndexMap, IndexSet};
use serde_json::Value;
use std::path::Path;

// ── Language family table ─────────────────────────────────────────────────────

/// Extension → language family, for cross-language suppression logic.
/// Mirrors Python's `_LANG_FAMILY`.
pub(crate) static LANG_FAMILY: std::sync::LazyLock<IndexMap<&'static str, &'static str>> =
    std::sync::LazyLock::new(|| {
        let mut m = IndexMap::new();
        for ext in &[".py", ".pyw"] {
            m.insert(*ext, "python");
        }
        for ext in &[
            ".js", ".jsx", ".mjs", ".ejs", ".ts", ".tsx", ".vue", ".svelte",
        ] {
            m.insert(*ext, "js");
        }
        m.insert(".go", "go");
        m.insert(".rs", "rust");
        for ext in &[".java", ".kt", ".kts", ".scala"] {
            m.insert(*ext, "jvm");
        }
        for ext in &[".c", ".h", ".cpp", ".cc", ".cxx", ".hpp"] {
            m.insert(*ext, "c");
        }
        m.insert(".rb", "ruby");
        m.insert(".swift", "swift");
        m.insert(".cs", "dotnet");
        m.insert(".php", "php");
        m.insert(".r", "r");
        m
    });

/// JSON key labels that indicate a noise node extracted from a JSON schema.
pub(crate) static JSON_NOISE_LABELS: std::sync::LazyLock<IndexSet<&'static str>> =
    std::sync::LazyLock::new(|| {
        [
            "start",
            "end",
            "name",
            "id",
            "type",
            "properties",
            "value",
            "key",
            "data",
            "items",
            "title",
            "description",
            "version",
            "dependencies",
            "devdependencies",
            "peerdependencies",
            "optionaldependencies",
            "bundleddependencies",
            "bundledependencies",
        ]
        .into_iter()
        .collect()
    });

// ── Predicates ────────────────────────────────────────────────────────────────

/// Return true if a node is a file-level hub or AST method stub.
///
/// Requires a precomputed `degrees` map (use [`crate::centrality::all_degrees`]).
/// The previous version recomputed `all_degrees(graph)` *inside this function*
/// every time it was called — at 25k callers per build, that O(C × (N+E))
/// shape dominated the entire `update` pipeline.
///
/// Mirrors Python `_is_file_node`.
pub(crate) fn is_file_node(
    graph: &Graph,
    node_id: &str,
    degrees: &indexmap::IndexMap<String, usize>,
) -> bool {
    let Some(attrs) = graph.node_data(node_id) else {
        return false;
    };
    let label = attrs.get("label").and_then(Value::as_str).unwrap_or("");
    if label.is_empty() {
        return false;
    }
    // File-level hub: label matches the actual source filename
    let source_file = attrs
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !source_file.is_empty() {
        let file_name = Path::new(source_file)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if label == file_name {
            return true;
        }
    }
    // Method stub: ".method_name()"
    if label.starts_with('.') && label.ends_with("()") {
        return true;
    }
    // Module-level function stub: "function_name()" with degree <= 1
    if label.ends_with("()") && degrees.get(node_id).copied().unwrap_or(0) <= 1 {
        return true;
    }
    false
}

/// Return true if the node is a manually-injected semantic concept node.
///
/// Signals: empty `source_file`, or `source_file` has no extension.
///
/// Mirrors Python `_is_concept_node`.
#[must_use]
pub fn is_concept_node(graph: &Graph, node_id: &str) -> bool {
    let Some(attrs) = graph.node_data(node_id) else {
        return true;
    };
    let source = attrs
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or("");
    if source.is_empty() {
        return true;
    }
    // No file extension in the last path component
    let last = source.rsplit('/').next().unwrap_or(source);
    !last.contains('.')
}

/// Classify a file path as "code", "paper", "image", or "doc".
///
/// Uses graphify-detect's `classify_file` (same extension list as Python).
///
/// Mirrors Python `_file_category`.
#[must_use]
pub fn file_category(path: &str) -> &'static str {
    match classify_file(Path::new(path)) {
        Some(FileType::Code) => "code",
        Some(FileType::Paper) => "paper",
        Some(FileType::Image) => "image",
        _ => "doc",
    }
}

/// Return true if this is a noise JSON key node that should be excluded.
///
/// Mirrors Python `_is_json_key_node`.
#[must_use]
pub fn is_json_key_node(graph: &Graph, node_id: &str) -> bool {
    let Some(attrs) = graph.node_data(node_id) else {
        return false;
    };
    let src = attrs
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    if !std::path::Path::new(&src)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        return false;
    }
    let label = attrs
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    JSON_NOISE_LABELS.contains(label.as_str())
}

/// Return the first path component (for cross-repo detection).
///
/// Used to identify which top-level repository directory a file belongs to.
/// Two nodes from different top-level directories are presumed to be in
/// different repos, which contributes a surprise-score bonus in `surprises.rs`.
/// Mirrors Python `_top_level_dir`.
pub(crate) fn top_level_dir(path: &str) -> &str {
    path.split('/').next().unwrap_or(path)
}
