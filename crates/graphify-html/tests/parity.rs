//! Parity tests for `graphify-html`.
//!
//! Ports `graphify-py/tests/test_callflow_html.py`.
//!
//! The two CLI subprocess tests (`test_export_callflow_html_cli_*`) are not
//! ported here because they exercise the Python CLI binary — they have no
//! direct Rust equivalent at the crate level.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashMap;

use graphify_html::callflow::{
    CallflowOptions, Node, derive_sections_from_communities, normalize_node, write_callflow_html,
};

// ── helpers ──────────────────────────────────────────────────────────────────

/// Mirrors `_make_graphify_out` from the Python test fixture.
fn make_graphify_out(tmp: &tempfile::TempDir) -> std::path::PathBuf {
    let out = tmp.path().join("graphify-out");
    std::fs::create_dir_all(&out).unwrap();

    let graph = serde_json::json!({
        "directed": false,
        "multigraph": false,
        "graph": {},
        "nodes": [
            {"id": "api",    "label": "ApiClient",              "source_file": "src/api.py",    "file_type": "code", "community": 0},
            {"id": "run",    "label": "run()",                  "source_file": "src/main.py",   "file_type": "code", "community": 0},
            {"id": "export", "label": "write_html()",           "source_file": "src/export.py", "file_type": "code", "community": 1},
            {"id": "evil",   "label": "<script>alert(1)</script>", "source_file": "src/evil.py","file_type": "code", "community": 1},
        ],
        "links": [
            {"source": "run",    "target": "api",    "relation": "calls", "confidence": "EXTRACTED", "confidence_score": 1.0},
            {"source": "api",    "target": "export", "relation": "uses",  "confidence": "EXTRACTED", "confidence_score": 1.0},
            {"source": "export", "target": "evil",   "relation": "calls", "confidence": "EXTRACTED", "confidence_score": 1.0},
        ],
        "hyperedges": [],
        "built_at_commit": "abcdef123456",
    });
    std::fs::write(
        out.join("graph.json"),
        serde_json::to_string(&graph).unwrap(),
    )
    .unwrap();
    std::fs::write(
        out.join(".graphify_labels.json"),
        r#"{"0": "Runtime", "1": "Export"}"#,
    )
    .unwrap();
    std::fs::write(
        out.join("GRAPH_REPORT.md"),
        "# Graph Report - sample\n\n## Summary\n- 3 nodes · 2 edges · 1 communities detected\n\n## God Nodes (most connected - your core abstractions)\n1. `Transformer` - 2 edges\n",
    )
    .unwrap();
    out
}

// ── test_write_callflow_html_creates_file_and_uses_report ────────────────────

/// Ports `test_write_callflow_html_creates_file_and_uses_report`.
#[test]
fn test_write_callflow_html_creates_file_and_uses_report() {
    let tmp = tempfile::tempdir().unwrap();
    let out = make_graphify_out(&tmp);

    let opts = CallflowOptions {
        project: Some(tmp.path().to_path_buf()),
        output: Some(out.join("callflow.html")),
        max_sections: 4,
        ..Default::default()
    };

    let html_path = write_callflow_html(&opts).expect("write_callflow_html should succeed");

    assert_eq!(html_path, out.join("callflow.html"));
    let content = std::fs::read_to_string(&html_path).unwrap();

    assert!(
        content.contains("mermaid"),
        "output should contain 'mermaid'"
    );
    assert!(
        content.contains("Graph Report Highlights"),
        "output should contain report highlights section"
    );
    assert!(
        content.contains("Transformer"),
        "output should include god-node from report"
    );
    assert!(
        content.contains("ApiClient"),
        "output should include node label 'ApiClient'"
    );
    // XSS content must be HTML-escaped.
    assert!(
        content.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "XSS content must be escaped"
    );
    assert!(
        !content.contains("<script>alert(1)</script>"),
        "raw XSS content must NOT appear verbatim"
    );
}

// ── test_derive_sections_groups_by_architecture_keywords ─────────────────────

/// Ports `test_derive_sections_groups_by_architecture_keywords`.
#[test]
fn test_derive_sections_groups_by_architecture_keywords() {
    let raw_nodes: &[serde_json::Value] = &[
        serde_json::json!({"id": "extract_py", "label": "extract_python", "source_file": "graphify/extract.py", "community": 0}),
        serde_json::json!({"id": "extract_js", "label": "extract_js",     "source_file": "graphify/extract.py", "community": 0}),
        serde_json::json!({"id": "to_html",    "label": "to_html",        "source_file": "graphify/export.py",  "community": 1}),
        serde_json::json!({"id": "test_html",  "label": "test_export_html","source_file": "tests/test_export.py","community": 2}),
    ];

    let nodes: Vec<Node> = raw_nodes
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.as_object().map(|m| normalize_node(m, i)))
        .collect();

    let labels: HashMap<String, String> = HashMap::new();
    let sections = derive_sections_from_communities(&nodes, &labels, "en", 6);

    let ids: std::collections::HashSet<&str> = sections.iter().map(|s| s.id.as_str()).collect();

    assert!(
        ids.contains("extract-pipeline"),
        "expected 'extract-pipeline' section, got: {ids:?}"
    );
    assert!(
        ids.contains("outputs-docs"),
        "expected 'outputs-docs' section, got: {ids:?}"
    );
    assert!(
        ids.contains("tests-fixtures"),
        "expected 'tests-fixtures' section, got: {ids:?}"
    );
}
