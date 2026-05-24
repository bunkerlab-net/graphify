//! Parity tests for graphify-report, mirroring
//! `graphify-py/tests/test_report.py`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use graphify_build::{Graph, GraphKind, build_from_json};
use graphify_report::{render_report, write_report};
use serde_json::{Value, json};
use std::path::Path;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn extraction() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../graphify-py/tests/fixtures/extraction.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("fixture not found: {}", path.display()));
    serde_json::from_str(&text).expect("fixture is valid JSON")
}

fn make_graph() -> Graph {
    let ext = extraction();
    build_from_json(ext, false, None).expect("build_from_json should succeed")
}

fn make_analysis() -> Value {
    let ext = extraction();
    let input_tokens = ext.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
    let output_tokens = ext
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    json!({
        "communities": {
            "0": ["n_transformer", "n_attention"],
            "1": ["n_layernorm", "n_concept_attn"]
        },
        "cohesion_scores": { "0": 0.75, "1": 0.5 },
        "community_labels": {
            "0": "Community 0",
            "1": "Community 1"
        },
        "god_nodes": [
            { "id": "n_transformer", "label": "Transformer", "degree": 2 },
            { "id": "n_attention", "label": "MultiHeadAttention", "degree": 2 }
        ],
        "surprising_connections": [
            {
                "source": "LayerNorm",
                "target": "attention mechanism",
                "relation": "referenced",
                "confidence": "AMBIGUOUS",
                "source_files": ["model.py", "paper.md"],
                "note": ""
            }
        ],
        "detection_result": {
            "total_files": 4,
            "total_words": 62400,
            "warning": null
        },
        "token_cost": { "input": input_tokens, "output": output_tokens },
        "root": "./project",
        "suggested_questions": null,
        "min_community_size": 3,
        "built_at_commit": null
    })
}

// ---------------------------------------------------------------------------
// Tests — mirrors test_report.py
// ---------------------------------------------------------------------------

#[test]
fn test_report_contains_header() {
    let graph = make_graph();
    let analysis = make_analysis();
    let report = render_report(&graph, &analysis);
    assert!(report.contains("# Graph Report"), "header missing");
}

#[test]
fn test_report_contains_corpus_check() {
    let graph = make_graph();
    let analysis = make_analysis();
    let report = render_report(&graph, &analysis);
    assert!(
        report.contains("## Corpus Check"),
        "corpus check section missing"
    );
}

#[test]
fn test_report_contains_god_nodes() {
    let graph = make_graph();
    let analysis = make_analysis();
    let report = render_report(&graph, &analysis);
    assert!(report.contains("## God Nodes"), "god nodes section missing");
}

#[test]
fn test_report_contains_surprising_connections() {
    let graph = make_graph();
    let analysis = make_analysis();
    let report = render_report(&graph, &analysis);
    assert!(
        report.contains("## Surprising Connections"),
        "surprising connections section missing"
    );
}

#[test]
fn test_report_contains_communities() {
    let graph = make_graph();
    let analysis = make_analysis();
    let report = render_report(&graph, &analysis);
    assert!(
        report.contains("## Communities"),
        "communities section missing"
    );
}

#[test]
fn test_report_contains_ambiguous_section() {
    let graph = make_graph();
    let analysis = make_analysis();
    let report = render_report(&graph, &analysis);
    assert!(
        report.contains("## Ambiguous Edges"),
        "ambiguous edges section missing"
    );
}

#[test]
fn test_report_shows_token_cost() {
    let graph = make_graph();
    let analysis = make_analysis();
    let report = render_report(&graph, &analysis);
    assert!(report.contains("Token cost"), "token cost line missing");
    assert!(
        report.contains("1,200"),
        "input token count missing (expected '1,200')"
    );
}

#[test]
fn test_report_shows_raw_cohesion_scores() {
    let graph = make_graph();
    // min_community_size=1 to show all communities including thin ones
    let mut analysis = make_analysis();
    analysis
        .as_object_mut()
        .unwrap()
        .insert("min_community_size".to_string(), json!(1));
    let report = render_report(&graph, &analysis);
    assert!(report.contains("Cohesion:"), "cohesion score missing");
    assert!(!report.contains('\u{2713}'), "unexpected ✓ in report");
    assert!(!report.contains('\u{26a0}'), "unexpected ⚠ in report");
}

// ---------------------------------------------------------------------------
// Extra Rust-specific tests
// ---------------------------------------------------------------------------

#[test]
fn test_write_report_creates_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("GRAPH_REPORT.md");
    let graph = make_graph();
    let analysis = make_analysis();
    write_report(&graph, &analysis, &path).expect("write_report should succeed");
    let content = std::fs::read_to_string(&path).expect("file should exist");
    assert!(
        content.contains("# Graph Report"),
        "written file should have header"
    );
}

#[test]
fn test_report_with_warning_detection() {
    let graph = make_graph();
    let mut analysis = make_analysis();
    analysis.as_object_mut().unwrap().insert(
        "detection_result".to_string(),
        json!({ "warning": "Corpus is too small — graph structure may not add value." }),
    );
    let report = render_report(&graph, &analysis);
    assert!(
        report.contains("Corpus is too small"),
        "warning should appear in report"
    );
    assert!(
        !report.contains("files ·"),
        "file count line should not appear when warning is set"
    );
}

#[test]
fn test_report_with_freshness_commit() {
    let graph = make_graph();
    let mut analysis = make_analysis();
    analysis
        .as_object_mut()
        .unwrap()
        .insert("built_at_commit".to_string(), json!("abcdef1234567890"));
    let report = render_report(&graph, &analysis);
    assert!(
        report.contains("## Graph Freshness"),
        "freshness section missing"
    );
    assert!(
        report.contains("`abcdef12`"),
        "short commit hash should appear"
    );
}

#[test]
fn test_report_community_navigation() {
    let graph = make_graph();
    let mut analysis = make_analysis();
    // Use min_community_size=1 so communities show up
    analysis
        .as_object_mut()
        .unwrap()
        .insert("min_community_size".to_string(), json!(1));
    let report = render_report(&graph, &analysis);
    assert!(
        report.contains("## Community Hubs (Navigation)"),
        "community hubs section missing"
    );
    assert!(
        report.contains("[[_COMMUNITY_"),
        "community wikilinks missing"
    );
}

#[test]
fn test_report_hyperedges() {
    use indexmap::IndexMap;
    // Build a graph with hyperedges in graph_attrs
    let mut graph = Graph::new(GraphKind::Graph);
    let mut attrs = IndexMap::new();
    attrs.insert(
        "label".to_string(),
        serde_json::Value::String("Foo".to_string()),
    );
    attrs.insert(
        "file_type".to_string(),
        serde_json::Value::String("code".to_string()),
    );
    attrs.insert(
        "source_file".to_string(),
        serde_json::Value::String("foo.py".to_string()),
    );
    graph.add_node("n1", attrs);
    graph.graph_attrs.insert(
        "hyperedges".to_string(),
        json!([{
            "label": "MyGroup",
            "nodes": ["Foo", "Bar"],
            "confidence": "INFERRED",
            "confidence_score": 0.85
        }]),
    );
    let analysis = json!({
        "communities": {},
        "cohesion_scores": {},
        "community_labels": {},
        "god_nodes": [],
        "surprising_connections": [],
        "detection_result": { "total_files": 1, "total_words": 100, "warning": null },
        "token_cost": { "input": 0, "output": 0 },
        "root": "./test"
    });
    let report = render_report(&graph, &analysis);
    assert!(
        report.contains("## Hyperedges"),
        "hyperedges section missing"
    );
    assert!(report.contains("**MyGroup**"), "hyperedge label missing");
    assert!(
        report.contains("INFERRED 0.85"),
        "hyperedge confidence missing"
    );
}

#[test]
fn test_report_suggested_questions() {
    let graph = make_graph();
    let mut analysis = make_analysis();
    analysis.as_object_mut().unwrap().insert(
        "suggested_questions".to_string(),
        json!([
            {
                "type": "ambiguous_edge",
                "question": "What is the relationship between A and B?",
                "why": "Edge tagged AMBIGUOUS."
            }
        ]),
    );
    let report = render_report(&graph, &analysis);
    assert!(
        report.contains("## Suggested Questions"),
        "suggested questions section missing"
    );
    assert!(
        report.contains("What is the relationship between A and B?"),
        "question text missing"
    );
}

#[test]
fn test_report_no_signal_question() {
    let graph = make_graph();
    let mut analysis = make_analysis();
    analysis.as_object_mut().unwrap().insert(
        "suggested_questions".to_string(),
        json!([
            {
                "type": "no_signal",
                "question": null,
                "why": "Not enough signal."
            }
        ]),
    );
    let report = render_report(&graph, &analysis);
    assert!(report.contains("## Suggested Questions"), "section missing");
    assert!(
        report.contains("_Not enough signal._"),
        "no_signal why missing"
    );
}

#[test]
fn test_fmt_comma_values() {
    // Indirectly tested via token cost display — 1200 → "1,200"
    let graph = make_graph();
    let mut analysis = make_analysis();
    analysis.as_object_mut().unwrap().insert(
        "token_cost".to_string(),
        json!({ "input": 1_234_567u64, "output": 999 }),
    );
    let report = render_report(&graph, &analysis);
    assert!(
        report.contains("1,234,567"),
        "large number formatting failed"
    );
}

#[test]
fn test_empty_graph_renders() {
    let graph = Graph::new(GraphKind::Graph);
    let analysis = json!({
        "communities": {},
        "cohesion_scores": {},
        "community_labels": {},
        "god_nodes": [],
        "surprising_connections": [],
        "detection_result": { "total_files": 0, "total_words": 0, "warning": null },
        "token_cost": { "input": 0, "output": 0 },
        "root": "./empty"
    });
    let report = render_report(&graph, &analysis);
    assert!(
        report.contains("# Graph Report"),
        "header present on empty graph"
    );
    assert!(
        report.contains("- None detected"),
        "no surprises message present"
    );
}
