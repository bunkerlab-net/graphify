//! Parity tests against `graphify-py/tests/test_validate.py`.
// reason: tests intentionally panic on broken invariants so failures surface loudly
#![allow(clippy::expect_used)]

use graphify_validate::{assert_valid, validate_extraction};
use serde_json::json;

fn valid() -> serde_json::Value {
    json!({
        "nodes": [
            {"id": "n1", "label": "Foo", "file_type": "code", "source_file": "foo.py"},
            {"id": "n2", "label": "Bar", "file_type": "document", "source_file": "bar.md"},
        ],
        "edges": [
            {"source": "n1", "target": "n2", "relation": "references",
             "confidence": "EXTRACTED", "source_file": "foo.py", "weight": 1.0},
        ],
    })
}

#[test]
fn valid_passes() {
    assert_eq!(validate_extraction(&valid()), Vec::<String>::new());
}

#[test]
fn missing_nodes_key() {
    let errors = validate_extraction(&json!({"edges": []}));
    assert!(errors.iter().any(|e| e.contains("nodes")));
}

#[test]
fn missing_edges_key() {
    let errors = validate_extraction(&json!({"nodes": []}));
    assert!(errors.iter().any(|e| e.contains("edges")));
}

#[test]
fn not_a_dict() {
    let errors = validate_extraction(&json!([]));
    assert_eq!(errors.len(), 1);
}

#[test]
fn invalid_file_type() {
    let data = json!({
        "nodes": [{"id": "n1", "label": "X", "file_type": "video", "source_file": "x.mp4"}],
        "edges": [],
    });
    let errors = validate_extraction(&data);
    assert!(errors.iter().any(|e| e.contains("file_type")));
}

#[test]
fn invalid_confidence() {
    let data = json!({
        "nodes": [
            {"id": "n1", "label": "A", "file_type": "code", "source_file": "a.py"},
            {"id": "n2", "label": "B", "file_type": "code", "source_file": "b.py"},
        ],
        "edges": [
            {"source": "n1", "target": "n2", "relation": "calls",
             "confidence": "CERTAIN", "source_file": "a.py"},
        ],
    });
    let errors = validate_extraction(&data);
    assert!(errors.iter().any(|e| e.contains("confidence")));
}

#[test]
fn dangling_edge_source() {
    let data = json!({
        "nodes": [{"id": "n1", "label": "A", "file_type": "code", "source_file": "a.py"}],
        "edges": [
            {"source": "missing_id", "target": "n1", "relation": "calls",
             "confidence": "EXTRACTED", "source_file": "a.py"},
        ],
    });
    let errors = validate_extraction(&data);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("source") && e.contains("missing_id"))
    );
}

#[test]
fn dangling_edge_target() {
    let data = json!({
        "nodes": [{"id": "n1", "label": "A", "file_type": "code", "source_file": "a.py"}],
        "edges": [
            {"source": "n1", "target": "ghost", "relation": "calls",
             "confidence": "EXTRACTED", "source_file": "a.py"},
        ],
    });
    let errors = validate_extraction(&data);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("target") && e.contains("ghost"))
    );
}

#[test]
fn missing_node_field() {
    let data = json!({
        "nodes": [{"id": "n1", "label": "A", "source_file": "a.py"}],
        "edges": [],
    });
    let errors = validate_extraction(&data);
    assert!(errors.iter().any(|e| e.contains("file_type")));
}

#[test]
fn assert_valid_raises_on_errors() {
    let result = assert_valid(&json!({"nodes": "bad", "edges": []}));
    let err = result.expect_err("assert_valid should reject a non-array nodes field");
    assert!(format!("{err}").contains("error"));
}

#[test]
fn assert_valid_passes_silently() {
    assert_valid(&valid()).expect("VALID should pass");
}

#[test]
fn edges_accepts_links_alias() {
    // NetworkX <= 3.1 fallback: "links" should be accepted in place of "edges".
    let data = json!({
        "nodes": [
            {"id": "n1", "label": "A", "file_type": "code", "source_file": "a.py"},
            {"id": "n2", "label": "B", "file_type": "code", "source_file": "b.py"},
        ],
        "links": [
            {"source": "n1", "target": "n2", "relation": "calls",
             "confidence": "EXTRACTED", "source_file": "a.py"},
        ],
    });
    assert_eq!(validate_extraction(&data), Vec::<String>::new());
}

#[test]
fn nodes_not_a_list() {
    let errors = validate_extraction(&json!({"nodes": "bad", "edges": []}));
    assert!(errors.iter().any(|e| e.contains("'nodes' must be a list")));
}

#[test]
fn edges_not_a_list() {
    let errors = validate_extraction(&json!({"nodes": [], "edges": "bad"}));
    assert!(errors.iter().any(|e| e.contains("'edges' must be a list")));
}

#[test]
fn node_must_be_object() {
    let errors = validate_extraction(&json!({"nodes": ["not-an-object"], "edges": []}));
    assert!(errors.iter().any(|e| e.contains("must be an object")));
}

#[test]
fn edge_must_be_object() {
    let errors = validate_extraction(&json!({
        "nodes": [{"id": "n1", "label": "A", "file_type": "code", "source_file": "a.py"}],
        "edges": ["not-an-object"],
    }));
    assert!(errors.iter().any(|e| e.contains("must be an object")));
}

#[test]
fn non_hashable_node_id_reported_not_raised() {
    // A list-valued id must be reported as an error, not crash the validator.
    let data = json!({
        "nodes": [
            {"id": "n1", "label": "A", "file_type": "code", "source_file": "a.py"},
            {"id": ["x", "y"], "label": "B", "file_type": "code", "source_file": "b.py"},
        ],
        "edges": [],
    });
    let errors = validate_extraction(&data);
    assert!(errors.iter().any(|e| e.contains("non-hashable id")));
}

#[test]
fn non_hashable_edge_endpoint_reported_not_raised() {
    // A list-valued endpoint must be reported, not crash the membership test.
    let data = json!({
        "nodes": [
            {"id": "n1", "label": "A", "file_type": "code", "source_file": "a.py"},
            {"id": "n2", "label": "B", "file_type": "code", "source_file": "b.py"},
        ],
        "edges": [
            {"source": "n1", "target": ["n2", "n3"], "relation": "calls",
             "confidence": "INFERRED", "source_file": "a.py"},
        ],
    });
    let errors = validate_extraction(&data);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("target") && e.contains("non-hashable"))
    );
}

#[test]
fn non_hashable_node_id_does_not_mask_valid_ids() {
    // The valid node id must still be collected so a legitimately-dangling edge
    // is still flagged even when a sibling node has a bad id.
    let data = json!({
        "nodes": [
            {"id": "n1", "label": "A", "file_type": "code", "source_file": "a.py"},
            {"id": {"oops": 1}, "label": "B", "file_type": "code", "source_file": "b.py"},
        ],
        "edges": [
            {"source": "n1", "target": "ghost", "relation": "calls",
             "confidence": "EXTRACTED", "source_file": "a.py"},
        ],
    });
    let errors = validate_extraction(&data);
    assert!(errors.iter().any(|e| e.contains("non-hashable id")));
    assert!(
        errors
            .iter()
            .any(|e| e.contains("target") && e.contains("ghost"))
    );
}
