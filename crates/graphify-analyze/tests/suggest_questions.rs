//! Coverage tests for `suggest_questions` exercising the 5 question sections:
//! AMBIGUOUS edges, bridge nodes, god nodes, isolated nodes, low cohesion.

#![allow(clippy::expect_used)]

use graphify_analyze::suggest_questions;
use graphify_build::build_from_json;
use indexmap::IndexMap;
use serde_json::json;

fn build_graph(j: serde_json::Value) -> graphify_build::Graph {
    build_from_json(j, false, None).expect("build_from_json")
}

#[test]
fn suggest_questions_no_signal_truly_empty() {
    let g = build_graph(json!({"nodes": [], "edges": []}));
    let communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    let labels: IndexMap<i64, String> = IndexMap::new();
    let qs = suggest_questions(&g, &communities, &labels, 5);
    assert_eq!(qs.len(), 1);
    assert_eq!(qs[0]["type"], "no_signal");
}

#[test]
fn suggest_questions_ambiguous_edge() {
    let g = build_graph(json!({
        "nodes": [
            {"id": "a", "label": "A", "source_file": "a.py"},
            {"id": "b", "label": "B", "source_file": "b.py"}
        ],
        "edges": [
            {"source": "a", "target": "b", "relation": "calls", "confidence": "AMBIGUOUS"}
        ]
    }));
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec!["a".into(), "b".into()]);
    let labels: IndexMap<i64, String> = IndexMap::new();
    let qs = suggest_questions(&g, &communities, &labels, 10);
    assert!(
        qs.iter().any(|q| q["type"] == "ambiguous_edge"),
        "expected ambiguous_edge question, got {qs:?}"
    );
}

#[test]
fn suggest_questions_god_node() {
    // Build a graph with one well-connected node and many leaves.
    let mut nodes = vec![json!({
        "id": "hub",
        "label": "Hub",
        "source_file": "src/hub.py"
    })];
    let mut edges = vec![];
    for i in 0..15 {
        nodes.push(json!({
            "id": format!("leaf{i}"),
            "label": format!("Leaf{i}"),
            "source_file": format!("src/leaf{i}.py")
        }));
        edges.push(json!({
            "source": "hub",
            "target": format!("leaf{i}"),
            "relation": "calls",
            "confidence": "EXTRACTED"
        }));
    }
    let g = build_graph(json!({"nodes": nodes, "edges": edges}));
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(
        0,
        std::iter::once("hub".to_string())
            .chain((0..15).map(|i| format!("leaf{i}")))
            .collect(),
    );
    let labels: IndexMap<i64, String> = IndexMap::new();
    let qs = suggest_questions(&g, &communities, &labels, 10);
    assert!(!qs.is_empty(), "expected questions for god node");
}

#[test]
fn suggest_questions_isolated_node() {
    let g = build_graph(json!({
        "nodes": [
            {"id": "a", "label": "Used", "source_file": "src/a.py"},
            {"id": "b", "label": "Lonely", "source_file": "src/b.py"},
            {"id": "c", "label": "Friend", "source_file": "src/c.py"}
        ],
        "edges": [
            {"source": "a", "target": "c", "relation": "uses", "confidence": "EXTRACTED"}
        ]
    }));
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec!["a".into(), "b".into(), "c".into()]);
    let labels: IndexMap<i64, String> = IndexMap::new();
    let qs = suggest_questions(&g, &communities, &labels, 10);
    assert!(!qs.is_empty());
}

#[test]
fn suggest_questions_bridge_node() {
    // Two communities connected only through "bridge" node.
    let g = build_graph(json!({
        "nodes": [
            {"id": "c1a", "label": "C1A", "source_file": "src/c1/a.py"},
            {"id": "c1b", "label": "C1B", "source_file": "src/c1/b.py"},
            {"id": "bridge", "label": "Bridge", "source_file": "src/bridge.py"},
            {"id": "c2a", "label": "C2A", "source_file": "src/c2/a.py"},
            {"id": "c2b", "label": "C2B", "source_file": "src/c2/b.py"}
        ],
        "edges": [
            {"source": "c1a", "target": "c1b", "relation": "uses", "confidence": "EXTRACTED"},
            {"source": "c1b", "target": "bridge", "relation": "uses", "confidence": "EXTRACTED"},
            {"source": "bridge", "target": "c2a", "relation": "uses", "confidence": "EXTRACTED"},
            {"source": "c2a", "target": "c2b", "relation": "uses", "confidence": "EXTRACTED"}
        ]
    }));
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec!["c1a".into(), "c1b".into()]);
    communities.insert(1, vec!["c2a".into(), "c2b".into()]);
    communities.insert(2, vec!["bridge".into()]);
    let mut labels: IndexMap<i64, String> = IndexMap::new();
    labels.insert(0, "Group1".into());
    labels.insert(1, "Group2".into());
    let qs = suggest_questions(&g, &communities, &labels, 10);
    assert!(!qs.is_empty());
}

#[test]
fn suggest_questions_respects_top_n() {
    let g = build_graph(json!({
        "nodes": [
            {"id": "a", "label": "A", "source_file": "a.py"},
            {"id": "b", "label": "B", "source_file": "b.py"},
            {"id": "c", "label": "C", "source_file": "c.py"}
        ],
        "edges": [
            {"source": "a", "target": "b", "relation": "x", "confidence": "AMBIGUOUS"},
            {"source": "b", "target": "c", "relation": "y", "confidence": "AMBIGUOUS"}
        ]
    }));
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec!["a".into(), "b".into(), "c".into()]);
    let labels: IndexMap<i64, String> = IndexMap::new();
    let qs = suggest_questions(&g, &communities, &labels, 1);
    assert_eq!(qs.len(), 1);
}

#[test]
fn suggest_questions_excludes_rationale_nodes_from_isolated_count() {
    // #1768: rationale nodes are excluded from the weakly-connected count so it
    // agrees with the report's Knowledge Gaps section (both count the same set).
    let g = build_graph(json!({
        "nodes": [
            {"id": "service", "label": "Service", "file_type": "code", "source_file": "service.py"},
            {"id": "reason", "label": "Explains service", "file_type": "rationale", "source_file": "service.py"}
        ],
        "edges": []
    }));
    let communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    let labels: IndexMap<i64, String> = IndexMap::new();
    let qs = suggest_questions(&g, &communities, &labels, 10);
    let isolated = qs
        .iter()
        .find(|q| q.get("type").and_then(serde_json::Value::as_str) == Some("isolated_nodes"))
        .expect("isolated_nodes question present");
    let why = isolated
        .get("why")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let question = isolated
        .get("question")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    assert!(why.starts_with("1 weakly-connected node"), "why: {why:?}");
    assert!(question.contains("`Service`"), "question: {question:?}");
    assert!(
        !question.contains("Explains service"),
        "rationale node leaked: {question:?}"
    );
}
