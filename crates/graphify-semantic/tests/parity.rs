//! Parity tests against `graphify-py/tests/test_semantic_cleanup.py`.
#![allow(clippy::expect_used)]

use graphify_semantic::{
    MAX_SEMANTIC_FRAGMENT_BYTES, MAX_SEMANTIC_HYPEREDGE_NODES, load_validated_semantic_fragment,
    sanitize_semantic_fragment, validate_semantic_fragment,
};
use serde_json::{Map, Value, json};
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// validate_semantic_fragment — shape + size + ID checks
// ---------------------------------------------------------------------------

#[test]
fn validate_accepts_minimal_fragment() {
    let frag = json!({"nodes": [], "edges": [], "hyperedges": []});
    assert!(validate_semantic_fragment(&frag).is_empty());
}

#[test]
fn validate_rejects_non_object() {
    let errs = validate_semantic_fragment(&json!([1, 2, 3]));
    assert_eq!(errs.len(), 1);
    assert!(errs[0].contains("must be a JSON object"));
}

#[test]
fn validate_rejects_non_list_nodes() {
    let errs = validate_semantic_fragment(&json!({"nodes": "x", "edges": []}));
    assert!(errs.iter().any(|e| e.contains("nodes must be a list")));
}

#[test]
fn validate_rejects_path_separator_in_node_id() {
    let errs = validate_semantic_fragment(&json!({
        "nodes": [{"id": "a/b"}],
        "edges": [],
    }));
    assert!(
        errs.iter()
            .any(|e| e.contains("must not contain path separators"))
    );
}

#[test]
fn validate_rejects_unsupported_characters_in_id() {
    let errs = validate_semantic_fragment(&json!({
        "nodes": [{"id": "a b"}],
        "edges": [],
    }));
    assert!(errs.iter().any(|e| e.contains("unsupported characters")));
}

#[test]
fn validate_rejects_invalid_file_type() {
    let errs = validate_semantic_fragment(&json!({
        "nodes": [{"id": "abc", "file_type": "bogus"}],
        "edges": [],
    }));
    assert!(errs.iter().any(|e| e.contains("file_type")));
}

#[test]
fn validate_accepts_rationale_and_concept_file_types() {
    // `rationale` / `concept` are technically valid (they are sanitised away
    // later) — they must NOT cause validation errors.
    let errs = validate_semantic_fragment(&json!({
        "nodes": [
            {"id": "a", "file_type": "rationale"},
            {"id": "b", "file_type": "concept"},
        ],
        "edges": [],
    }));
    assert!(errs.is_empty(), "errs: {errs:?}");
}

#[test]
fn validate_rejects_oversize_payload() {
    // Build a fragment with a huge string field. We cap at 25 MiB so 30 MiB
    // is definitely over.
    let huge = "x".repeat(30 * 1024 * 1024);
    let frag = json!({"nodes": [{"id": "a", "label": huge}], "edges": []});
    let errs = validate_semantic_fragment(&frag);
    assert!(errs.iter().any(|e| e.contains("max is")));
}

#[test]
fn validate_caps_hyperedge_node_count() {
    let many: Vec<String> = (0..=MAX_SEMANTIC_HYPEREDGE_NODES)
        .map(|i| format!("n{i}"))
        .collect();
    let frag = json!({
        "nodes": [],
        "edges": [],
        "hyperedges": [{"id": "he1", "nodes": many}],
    });
    let errs = validate_semantic_fragment(&frag);
    assert!(errs.iter().any(|e| e.contains("hyperedges[0].nodes has")));
}

// ---------------------------------------------------------------------------
// sanitize_semantic_fragment — node / edge / hyperedge cleanup
// ---------------------------------------------------------------------------

#[test]
fn sanitize_drops_rationale_file_type_node() {
    let mut frag: Map<String, Value> = json!({
        "nodes": [
            {"id": "x", "label": "RealEntity", "file_type": "code"},
            {"id": "r", "label": "Some rationale.", "file_type": "rationale"},
        ],
        "edges": [],
        "hyperedges": [],
    })
    .as_object()
    .expect("test invariant")
    .clone();
    sanitize_semantic_fragment(&mut frag);
    let nodes = frag
        .get("nodes")
        .expect("key present")
        .as_array()
        .expect("array field");
    assert_eq!(nodes.len(), 1);
    assert_eq!(
        nodes[0]
            .as_object()
            .expect("object field")
            .get("id")
            .expect("key present"),
        &json!("x")
    );
}

#[test]
fn sanitize_converts_sentence_rationale_to_attribute() {
    let mut frag: Map<String, Value> = json!({
        "nodes": [
            {"id": "target", "label": "Target", "file_type": "code"},
            {"id": "rationale", "label": "This is a long enough rationale sentence ending in period. Add more.", "file_type": "document"},
        ],
        "edges": [
            {"source": "rationale", "target": "target", "relation": "rationale_for"},
        ],
        "hyperedges": [],
    })
    .as_object()
    .expect("test invariant")
    .clone();
    sanitize_semantic_fragment(&mut frag);
    let nodes = frag
        .get("nodes")
        .expect("key present")
        .as_array()
        .expect("array field");
    let target = nodes
        .iter()
        .find(|n| n.as_object().and_then(|m| m.get("id")) == Some(&json!("target")))
        .expect("target node");
    let rationale = target
        .as_object()
        .expect("test invariant")
        .get("rationale")
        .and_then(Value::as_str)
        .expect("rationale attr");
    assert!(rationale.contains("rationale sentence"));
}

#[test]
fn sanitize_removes_concept_file_type_nodes() {
    // Concept nodes are removed via `file_type` invalid check; this test
    // confirms that a non-sentence concept node is removed cleanly (no
    // rationale propagation).
    let mut frag: Map<String, Value> = json!({
        "nodes": [
            {"id": "x", "label": "Target", "file_type": "code"},
            {"id": "c", "label": "Concept", "file_type": "concept"},
        ],
        "edges": [],
        "hyperedges": [],
    })
    .as_object()
    .expect("test invariant")
    .clone();
    sanitize_semantic_fragment(&mut frag);
    let ids: Vec<&str> = frag
        .get("nodes")
        .expect("test invariant")
        .as_array()
        .expect("test invariant")
        .iter()
        .filter_map(|n| {
            n.as_object()
                .and_then(|m| m.get("id"))
                .and_then(Value::as_str)
        })
        .collect();
    assert_eq!(ids, ["x"]);
}

#[test]
fn sanitize_drops_edges_pointing_at_removed_nodes() {
    let mut frag: Map<String, Value> = json!({
        "nodes": [
            {"id": "x", "label": "X", "file_type": "code"},
            {"id": "c", "label": "C", "file_type": "concept"},
        ],
        "edges": [
            {"source": "x", "target": "c", "relation": "references"},
        ],
        "hyperedges": [],
    })
    .as_object()
    .expect("test invariant")
    .clone();
    sanitize_semantic_fragment(&mut frag);
    assert_eq!(
        frag.get("edges")
            .expect("key present")
            .as_array()
            .expect("array field")
            .len(),
        0
    );
}

#[test]
fn sanitize_filters_hyperedge_members_to_survivors() {
    let mut frag: Map<String, Value> = json!({
        "nodes": [
            {"id": "x", "label": "X", "file_type": "code"},
            {"id": "y", "label": "Y", "file_type": "code"},
            {"id": "c", "label": "C", "file_type": "concept"},
        ],
        "edges": [],
        "hyperedges": [
            {"id": "h1", "nodes": ["x", "y", "c"]},
        ],
    })
    .as_object()
    .expect("test invariant")
    .clone();
    sanitize_semantic_fragment(&mut frag);
    let hyperedges = frag
        .get("hyperedges")
        .expect("key present")
        .as_array()
        .expect("array field");
    assert_eq!(hyperedges.len(), 1);
    let members: Vec<&str> = hyperedges[0]
        .as_object()
        .expect("test invariant")
        .get("nodes")
        .expect("test invariant")
        .as_array()
        .expect("test invariant")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(members, ["x", "y"]);
}

#[test]
fn sanitize_drops_hyperedge_when_under_two_survivors() {
    let mut frag: Map<String, Value> = json!({
        "nodes": [
            {"id": "x", "label": "X", "file_type": "code"},
            {"id": "c", "label": "C", "file_type": "concept"},
        ],
        "edges": [],
        "hyperedges": [
            {"id": "h1", "nodes": ["x", "c"]},
        ],
    })
    .as_object()
    .expect("test invariant")
    .clone();
    sanitize_semantic_fragment(&mut frag);
    assert_eq!(
        frag.get("hyperedges")
            .expect("key present")
            .as_array()
            .expect("array field")
            .len(),
        0
    );
}

#[test]
fn sanitize_drops_nodes_without_id() {
    let mut frag: Map<String, Value> = json!({
        "nodes": [
            {"id": "x", "label": "X", "file_type": "code"},
            {"label": "noid"},
        ],
        "edges": [],
        "hyperedges": [],
    })
    .as_object()
    .expect("test invariant")
    .clone();
    sanitize_semantic_fragment(&mut frag);
    assert_eq!(
        frag.get("nodes")
            .expect("key present")
            .as_array()
            .expect("array field")
            .len(),
        1
    );
}

// ---------------------------------------------------------------------------
// load_validated_semantic_fragment
// ---------------------------------------------------------------------------

#[test]
fn load_validated_valid_file() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("ok.json");
    std::fs::write(
        &path,
        json!({"nodes": [], "edges": [], "hyperedges": []}).to_string(),
    )
    .expect("test invariant");
    let (fragment, errors) = load_validated_semantic_fragment(&path);
    assert!(errors.is_empty(), "errors: {errors:?}");
    assert!(fragment.is_some());
}

#[test]
fn load_validated_rejects_invalid_json() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("bad.json");
    std::fs::write(&path, "{not json").expect("write fixture");
    let (fragment, errors) = load_validated_semantic_fragment(&path);
    assert!(fragment.is_none());
    assert!(errors.iter().any(|e| e.contains("invalid JSON")));
}

#[test]
fn load_validated_rejects_oversize_before_parse() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("big.json");
    let huge =
        "x".repeat(usize::try_from(MAX_SEMANTIC_FRAGMENT_BYTES + 100).expect("test invariant"));
    std::fs::write(&path, huge).expect("write fixture");
    let (fragment, errors) = load_validated_semantic_fragment(&path);
    assert!(fragment.is_none());
    assert!(errors.iter().any(|e| e.contains("max is")));
}

#[test]
fn load_validated_returns_errors_for_invalid_shape() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("invalid.json");
    std::fs::write(&path, json!({"nodes": "x"}).to_string()).expect("test invariant");
    let (fragment, errors) = load_validated_semantic_fragment(&path);
    assert!(fragment.is_none());
    assert!(!errors.is_empty());
}
