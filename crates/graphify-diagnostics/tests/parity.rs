//! Parity tests against `graphify-py/tests/test_multigraph_diagnostics.py`.
#![allow(clippy::expect_used)]

use graphify_diagnostics::{
    DiagnoseOptions, diagnose_extraction, diagnose_file, format_diagnostic_json,
    format_diagnostic_report, scan_producer_suppression_sites,
};
use serde_json::{Map, Value, json};
use tempfile::tempdir;

#[allow(clippy::needless_pass_by_value)] // test helper — value-based callers read cleaner
fn extraction(nodes: Value, edges: Value) -> Map<String, Value> {
    json!({"nodes": nodes, "edges": edges})
        .as_object()
        .expect("test invariant")
        .clone()
}

#[test]
fn diagnose_empty_extraction_returns_zero_counts() {
    let ext = extraction(json!([]), json!([]));
    let summary = diagnose_extraction(&ext, &DiagnoseOptions::default());
    assert_eq!(summary["node_count"], json!(0));
    assert_eq!(summary["raw_edge_count"], json!(0));
    assert_eq!(summary["valid_candidate_edges"], json!(0));
    assert_eq!(summary["same_endpoint_group_count"], json!(0));
}

#[test]
fn diagnose_counts_valid_candidate_edges() {
    let ext = extraction(
        json!([{"id": "a"}, {"id": "b"}]),
        json!([{"source": "a", "target": "b", "relation": "calls"}]),
    );
    let summary = diagnose_extraction(&ext, &DiagnoseOptions::default());
    assert_eq!(summary["valid_candidate_edges"], json!(1));
    assert_eq!(summary["missing_endpoint_edges"], json!(0));
    assert_eq!(summary["dangling_endpoint_edges"], json!(0));
}

#[test]
fn diagnose_detects_missing_endpoints() {
    let ext = extraction(
        json!([{"id": "a"}]),
        json!([{"source": "a"}, {"target": "a"}]),
    );
    let summary = diagnose_extraction(&ext, &DiagnoseOptions::default());
    assert_eq!(summary["missing_endpoint_edges"], json!(2));
}

#[test]
fn diagnose_detects_dangling_endpoints() {
    let ext = extraction(
        json!([{"id": "a"}]),
        json!([{"source": "a", "target": "ghost", "relation": "calls"}]),
    );
    let summary = diagnose_extraction(&ext, &DiagnoseOptions::default());
    assert_eq!(summary["dangling_endpoint_edges"], json!(1));
    assert_eq!(summary["valid_candidate_edges"], json!(0));
}

#[test]
fn diagnose_detects_self_loop() {
    let ext = extraction(
        json!([{"id": "a"}]),
        json!([{"source": "a", "target": "a", "relation": "calls"}]),
    );
    let summary = diagnose_extraction(&ext, &DiagnoseOptions::default());
    assert_eq!(summary["self_loop_edges"], json!(1));
}

#[test]
fn diagnose_detects_exact_duplicate_edges() {
    let ext = extraction(
        json!([{"id": "a"}, {"id": "b"}]),
        json!([
            {"source": "a", "target": "b", "relation": "calls"},
            {"source": "a", "target": "b", "relation": "calls"},
            {"source": "a", "target": "b", "relation": "calls"},
        ]),
    );
    let summary = diagnose_extraction(&ext, &DiagnoseOptions::default());
    assert_eq!(summary["exact_duplicate_edges"], json!(2));
    assert_eq!(summary["directed_same_endpoint_collapsed_edges"], json!(2));
}

#[test]
fn diagnose_detects_relation_variant_groups() {
    let ext = extraction(
        json!([{"id": "a"}, {"id": "b"}]),
        json!([
            {"source": "a", "target": "b", "relation": "calls"},
            {"source": "a", "target": "b", "relation": "imports"},
        ]),
    );
    let summary = diagnose_extraction(&ext, &DiagnoseOptions::default());
    assert_eq!(summary["relation_variant_groups"], json!(1));
}

#[test]
fn diagnose_detects_source_file_variant_groups() {
    let ext = extraction(
        json!([{"id": "a"}, {"id": "b"}]),
        json!([
            {"source": "a", "target": "b", "relation": "calls", "source_file": "x.py"},
            {"source": "a", "target": "b", "relation": "calls", "source_file": "y.py"},
        ]),
    );
    let summary = diagnose_extraction(&ext, &DiagnoseOptions::default());
    assert_eq!(summary["source_file_variant_groups"], json!(1));
}

#[test]
fn diagnose_accepts_links_alias_for_edges() {
    let ext = json!({
        "nodes": [{"id": "a"}, {"id": "b"}],
        "links": [{"source": "a", "target": "b", "relation": "calls"}],
    })
    .as_object()
    .expect("test invariant")
    .clone();
    let summary = diagnose_extraction(&ext, &DiagnoseOptions::default());
    assert_eq!(summary["valid_candidate_edges"], json!(1));
}

#[test]
fn diagnose_examples_lists_high_multiplicity_pairs() {
    let ext = extraction(
        json!([{"id": "a"}, {"id": "b"}, {"id": "c"}]),
        json!([
            {"source": "a", "target": "b", "relation": "calls"},
            {"source": "a", "target": "b", "relation": "imports"},
            {"source": "a", "target": "c", "relation": "calls"},
        ]),
    );
    let summary = diagnose_extraction(&ext, &DiagnoseOptions::default());
    let examples = summary["examples"].as_array().expect("array field");
    assert_eq!(examples.len(), 1);
    assert_eq!(examples[0]["source"], json!("a"));
    assert_eq!(examples[0]["target"], json!("b"));
    assert_eq!(examples[0]["edge_count"], json!(2));
}

#[test]
fn diagnose_examples_capped_by_max_examples() {
    let ext = extraction(
        json!([{"id": "a"}, {"id": "b"}, {"id": "c"}]),
        json!([
            {"source": "a", "target": "b", "relation": "calls"},
            {"source": "a", "target": "b", "relation": "imports"},
            {"source": "a", "target": "c", "relation": "calls"},
            {"source": "a", "target": "c", "relation": "imports"},
        ]),
    );
    let opts = DiagnoseOptions {
        max_examples: 1,
        ..DiagnoseOptions::default()
    };
    let summary = diagnose_extraction(&ext, &opts);
    assert_eq!(
        summary["examples"].as_array().expect("array field").len(),
        1
    );
}

#[test]
fn diagnose_examples_disabled_when_max_examples_zero() {
    let ext = extraction(
        json!([{"id": "a"}, {"id": "b"}]),
        json!([
            {"source": "a", "target": "b", "relation": "calls"},
            {"source": "a", "target": "b", "relation": "imports"},
        ]),
    );
    let opts = DiagnoseOptions {
        max_examples: 0,
        ..DiagnoseOptions::default()
    };
    let summary = diagnose_extraction(&ext, &opts);
    assert_eq!(
        summary["examples"].as_array().expect("array field").len(),
        0
    );
}

#[test]
fn diagnose_non_object_edges_counted() {
    let ext = extraction(
        json!([{"id": "a"}]),
        json!(["bad", 42, {"source": "a", "target": "a"}]),
    );
    let summary = diagnose_extraction(&ext, &DiagnoseOptions::default());
    assert_eq!(summary["non_object_edges"], json!(2));
}

#[test]
fn diagnose_does_not_mutate_input() {
    let ext = extraction(
        json!([{"id": "a"}, {"id": "b"}]),
        json!([{"source": "a", "target": "b", "relation": "calls"}]),
    );
    let original = ext.clone();
    let _ = diagnose_extraction(&ext, &DiagnoseOptions::default());
    assert_eq!(ext, original);
}

// ---------------------------------------------------------------------------
// diagnose_file
// ---------------------------------------------------------------------------

#[test]
fn diagnose_file_reads_directed_flag_from_json() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let path = dir.path().join("g.json");
    std::fs::write(
        &path,
        json!({
            "directed": false,
            "nodes": [{"id": "a"}, {"id": "b"}],
            "links": [{"source": "a", "target": "b", "relation": "calls"}],
        })
        .to_string(),
    )?;
    let summary = diagnose_file(&path, None, 5, None)?;
    assert_eq!(summary["effective_directed"], json!(false));
    Ok(())
}

#[test]
fn diagnose_file_directed_override_wins() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let path = dir.path().join("g.json");
    std::fs::write(
        &path,
        json!({
            "directed": false,
            "nodes": [{"id": "a"}],
            "links": [],
        })
        .to_string(),
    )?;
    let summary = diagnose_file(&path, Some(true), 5, None)?;
    assert_eq!(summary["effective_directed"], json!(true));
    Ok(())
}

#[test]
fn diagnose_file_rejects_non_object_input() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let path = dir.path().join("g.json");
    std::fs::write(&path, "[1, 2, 3]")?;
    let err =
        diagnose_file(&path, None, 5, None).expect_err("non-object JSON input must be rejected");
    assert!(
        matches!(err, graphify_diagnostics::DiagnosticsError::NotAnObject),
        "expected NotAnObject, got {err:?}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// format_diagnostic_json / format_diagnostic_report
// ---------------------------------------------------------------------------

#[test]
fn format_diagnostic_json_emits_schema_envelope() {
    let summary = diagnose_extraction(
        &extraction(json!([]), json!([])),
        &DiagnoseOptions::default(),
    );
    let envelope = format_diagnostic_json(&summary);
    assert_eq!(envelope["schema_version"], json!(1));
    assert!(envelope["summary"].is_object());
    assert!(envelope["notes"].is_array());
}

#[test]
fn format_diagnostic_report_includes_node_and_edge_counts() {
    let ext = extraction(
        json!([{"id": "a"}, {"id": "b"}]),
        json!([{"source": "a", "target": "b", "relation": "calls"}]),
    );
    let summary = diagnose_extraction(&ext, &DiagnoseOptions::default());
    let text = format_diagnostic_report(&summary);
    assert!(text.contains("nodes: 2"));
    assert!(text.contains("raw_edges: 1"));
    assert!(text.contains("valid_candidate_edges: 1"));
}

// ---------------------------------------------------------------------------
// scan_producer_suppression_sites
// ---------------------------------------------------------------------------

#[test]
fn scan_producer_suppression_sites_returns_file_not_found_for_missing_path() {
    let dir = tempdir().expect("tempdir");
    let result = scan_producer_suppression_sites(&dir.path().join("missing.py"));
    assert_eq!(result["total_sites"], json!(0));
    assert_eq!(result["error"], json!("file not found"));
}

#[test]
fn scan_producer_suppression_sites_picks_up_seen_decl() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("extract.py");
    std::fs::write(
        &path,
        "seen_calls: set[tuple[str, str]] = set()\nseen_other = set()\n",
    )
    .expect("test invariant");
    let result = scan_producer_suppression_sites(&path);
    let sites = result["sites"].as_array().expect("array field");
    assert!(sites.iter().any(|s| s["name"] == json!("seen_calls")));
    assert!(sites.iter().any(|s| s["name"] == json!("seen_other")));
    let calls_site = sites
        .iter()
        .find(|s| s["name"] == json!("seen_calls"))
        .expect("test invariant");
    assert_eq!(calls_site["tuple_arity"], json!(2));
}
