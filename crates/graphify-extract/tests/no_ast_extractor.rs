//! #1689 — a file classified as code but with no AST extractor (e.g. `.r`/`.R`)
//! contributes nothing but must not crash, and mustn't disturb files that do have
//! an extractor.
//!
//! Ports the observable contract of `graphify-py/tests/test_extract.py`'s
//! `test_extract_warns_on_code_files_with_no_ast_extractor`. The stderr warning
//! itself is a diagnostic not capturable from an in-process Rust test, so this
//! asserts the behaviour: the Python file still extracts; the R files produce no
//! nodes and no panic.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::case_sensitive_file_extension_comparisons
)]
use serde_json::Value;
use tempfile::tempdir;

#[test]
fn code_files_without_extractor_contribute_nothing_but_dont_break_extraction() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(root.join("analysis.R"), "f <- function(x) x + 1\n").expect("write R");
    std::fs::write(root.join("helper.r"), "g <- function(y) y * 2\n").expect("write r");
    std::fs::write(root.join("main.py"), "def main():\n    return 1\n").expect("write py");

    let out = graphify_extract::extract(
        &[
            root.join("analysis.R"),
            root.join("helper.r"),
            root.join("main.py"),
        ],
        Some(root),
    );

    let labels: Vec<&str> = out
        .nodes
        .iter()
        .filter_map(|n| n.get("label").and_then(Value::as_str))
        .collect();
    assert!(
        labels.iter().any(|l| l.starts_with("main")),
        "the Python file must still extract: {labels:?}"
    );
    // The R files have no extractor, so they contribute no source_file nodes.
    let r_nodes = out.nodes.iter().any(|n| {
        n.get("source_file")
            .and_then(Value::as_str)
            .is_some_and(|s| s.ends_with(".R") || s.ends_with(".r"))
    });
    assert!(!r_nodes, "files with no extractor must contribute no nodes");
}
