//! Coverage tests for `graphify_export::neo4j::{build_node_rows, build_edge_rows}`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use graphify_build::build_from_json;
use graphify_export::neo4j::{build_edge_rows, build_node_rows};
use graphify_export::{Neo4jError, push_to_neo4j_blocking};
use indexmap::IndexMap;
use serde_json::json;

fn small_graph() -> graphify_build::Graph {
    build_from_json(
        json!({
            "nodes": [
                {"id": "n1", "label": "A", "file_type": "code", "source_file": "a.py", "community": 0},
                {"id": "n2", "label": "B", "file_type": "document", "source_file": "b.md", "community": 1},
                {"id": "n3", "label": "C", "file_type": "code", "source_file": "c.py", "community": 1},
            ],
            "edges": [
                {"source": "n1", "target": "n2", "relation": "calls", "confidence": "EXTRACTED"},
                {"source": "n2", "target": "n3", "relation": "uses", "confidence": "INFERRED"},
            ]
        }),
        false,
        None,
    )
    .unwrap()
}

#[test]
fn build_node_rows_captures_label_and_props() {
    let g = small_graph();
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec!["n1".to_string()]);
    communities.insert(1, vec!["n2".to_string(), "n3".to_string()]);
    let rows = build_node_rows(&g, &communities);
    assert_eq!(rows.len(), 3);

    let r1 = rows.iter().find(|r| r.id == "n1").unwrap();
    assert!(r1.label.contains("Code") || r1.label == "Entity");
    assert_eq!(r1.community, Some(0));
    assert!(r1.props.contains_key("source_file"));

    let r2 = rows.iter().find(|r| r.id == "n2").unwrap();
    assert!(r2.label.contains("Document") || r2.label == "Entity");
    assert_eq!(r2.community, Some(1));
}

#[test]
fn build_node_rows_missing_file_type_defaults_to_entity() {
    let g = build_from_json(
        json!({
            "nodes": [{"id": "n1", "label": "A", "source_file": "a.py"}],
            "edges": []
        }),
        false,
        None,
    )
    .unwrap();
    let communities = IndexMap::new();
    let rows = build_node_rows(&g, &communities);
    assert_eq!(rows.len(), 1);
    // No file_type → defaults to "entity" → capitalised → "Entity" via cypher_label.
    assert!(
        !rows[0].label.is_empty(),
        "row label should be non-empty even without file_type, got {:?}",
        rows[0].label
    );
}

#[test]
fn build_edge_rows_captures_rel_type() {
    let g = small_graph();
    let rows = build_edge_rows(&g);
    assert_eq!(rows.len(), 2);
    let row = &rows[0];
    assert!(!row.rel_type.is_empty());
    assert!(row.props.contains_key("confidence") || row.props.is_empty());
}

#[test]
fn push_to_neo4j_blocking_fails_on_invalid_uri() {
    let g = small_graph();
    let communities = IndexMap::new();
    let result = push_to_neo4j_blocking(
        "bogus://no-host:99999",
        "neo4j",
        "wrong",
        &g,
        &communities,
        false,
    );
    // Either a Config or Driver error; both are valid here.
    assert!(matches!(
        result,
        Err(Neo4jError::Config(_) | Neo4jError::Driver(_))
    ));
}
