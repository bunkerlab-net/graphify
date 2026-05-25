//! Parity tests against `graphify-py/tests/test_scip_ingest.py`.
#![allow(clippy::expect_used)]

use graphify_scip::{ingest_scip_json, make_scip_node_id};
use serde_json::{Value, json};

fn nodes(result: &Value) -> &Vec<Value> {
    result["nodes"].as_array().expect("array field")
}
fn edges(result: &Value) -> &Vec<Value> {
    result["edges"].as_array().expect("array field")
}

// ---------------------------------------------------------------------------
// Empty / malformed input
// ---------------------------------------------------------------------------

#[test]
fn empty_doc_returns_empty_result() {
    let result = ingest_scip_json(&json!({}), "", "python");
    assert_eq!(nodes(&result).len(), 0);
    assert_eq!(edges(&result).len(), 0);
}

#[test]
fn non_object_input_returns_empty_result() {
    let result = ingest_scip_json(&json!([1, 2, 3]), "", "python");
    assert_eq!(nodes(&result).len(), 0);
    assert_eq!(edges(&result).len(), 0);
}

#[test]
fn documents_must_be_a_list() {
    let result = ingest_scip_json(&json!({"documents": "nope"}), "", "python");
    assert_eq!(nodes(&result).len(), 0);
}

#[test]
fn missing_documents_returns_empty_result() {
    let result = ingest_scip_json(&json!({"other": []}), "", "python");
    assert_eq!(nodes(&result).len(), 0);
}

// ---------------------------------------------------------------------------
// Node emission
// ---------------------------------------------------------------------------

#[test]
fn emits_one_node_per_symbol() {
    let result = ingest_scip_json(
        &json!({
            "documents": [{
                "relative_path": "pkg/foo.py",
                "language": "python",
                "symbols": [{
                    "symbol": "scip-python python pkg/foo/__init__.py `Foo`#",
                    "kind": "Class",
                    "display_name": "Foo",
                }],
            }],
        }),
        "",
        "python",
    );
    assert_eq!(nodes(&result).len(), 1);
    let n = &nodes(&result)[0];
    assert_eq!(n["label"], json!("Foo"));
    assert_eq!(n["file_type"], json!("code"));
    assert_eq!(n["source_file"], json!("pkg/foo.py"));
}

#[test]
fn node_label_falls_back_to_symbol_suffix() {
    let result = ingest_scip_json(
        &json!({
            "documents": [{
                "relative_path": "a.py",
                "symbols": [{"symbol": "scip foo#bar", "kind": "Method"}],
            }],
        }),
        "",
        "python",
    );
    assert_eq!(nodes(&result)[0]["label"], json!("bar"));
}

#[test]
fn node_source_location_from_first_occurrence() {
    let result = ingest_scip_json(
        &json!({
            "documents": [{
                "relative_path": "a.py",
                "symbols": [{
                    "symbol": "scip foo#bar",
                    "kind": "Method",
                    "occurrences": [{"range": [42, 0, 42, 3]}],
                }],
            }],
        }),
        "",
        "python",
    );
    assert_eq!(nodes(&result)[0]["source_location"], json!("L42"));
}

#[test]
fn malformed_range_does_not_corrupt_source_location() {
    let result = ingest_scip_json(
        &json!({
            "documents": [{
                "relative_path": "a.py",
                "symbols": [{
                    "symbol": "scip foo#bar",
                    "kind": "Method",
                    "occurrences": [{"range": [true, 0, 0]}],
                }],
            }],
        }),
        "",
        "python",
    );
    assert_eq!(nodes(&result)[0]["source_location"], json!(""));
}

#[test]
fn symbol_without_id_is_skipped() {
    let result = ingest_scip_json(
        &json!({
            "documents": [{
                "relative_path": "a.py",
                "symbols": [{"kind": "Method"}],
            }],
        }),
        "",
        "python",
    );
    assert_eq!(nodes(&result).len(), 0);
}

#[test]
fn non_dict_symbol_is_skipped() {
    let result = ingest_scip_json(
        &json!({
            "documents": [{
                "relative_path": "a.py",
                "symbols": [42, "bogus"],
            }],
        }),
        "",
        "python",
    );
    assert_eq!(nodes(&result).len(), 0);
}

// ---------------------------------------------------------------------------
// Relationships
// ---------------------------------------------------------------------------

#[test]
fn relationship_resolves_within_same_document() {
    let result = ingest_scip_json(
        &json!({
            "documents": [{
                "relative_path": "a.py",
                "symbols": [
                    {"symbol": "src", "kind": "Method", "relationships": [
                        {"symbol": "tgt", "is_reference": true},
                    ]},
                    {"symbol": "tgt", "kind": "Method"},
                ],
            }],
        }),
        "",
        "python",
    );
    // src + tgt nodes, no stub for tgt because it's resolved.
    assert_eq!(nodes(&result).len(), 2);
    assert_eq!(edges(&result).len(), 1);
    assert_eq!(edges(&result)[0]["relation"], json!("scip_ref"));
}

#[test]
fn relationship_creates_stub_node_when_target_unresolved() {
    let result = ingest_scip_json(
        &json!({
            "documents": [{
                "relative_path": "a.py",
                "symbols": [
                    {"symbol": "src", "kind": "Method", "relationships": [
                        {"symbol": "external-tgt", "is_reference": true},
                    ]},
                ],
            }],
        }),
        "",
        "python",
    );
    // src + stub for external-tgt.
    assert_eq!(nodes(&result).len(), 2);
    let stub_kind = nodes(&result)[1]["metadata"]["scip_kind"]
        .as_str()
        .expect("string field");
    assert_eq!(stub_kind, "external");
}

#[test]
fn relationship_is_implementation_yields_scip_impl() {
    let result = ingest_scip_json(
        &json!({
            "documents": [{
                "relative_path": "a.py",
                "symbols": [
                    {"symbol": "src", "relationships": [
                        {"symbol": "tgt", "is_implementation": true},
                    ]},
                    {"symbol": "tgt"},
                ],
            }],
        }),
        "",
        "python",
    );
    assert_eq!(edges(&result)[0]["relation"], json!("scip_impl"));
}

#[test]
fn relationship_is_type_definition_yields_scip_typed() {
    let result = ingest_scip_json(
        &json!({
            "documents": [{
                "relative_path": "a.py",
                "symbols": [
                    {"symbol": "src", "relationships": [
                        {"symbol": "tgt", "is_type_definition": true},
                    ]},
                    {"symbol": "tgt"},
                ],
            }],
        }),
        "",
        "python",
    );
    assert_eq!(edges(&result)[0]["relation"], json!("scip_typed"));
}

#[test]
fn relationship_is_definition_yields_scip_def() {
    let result = ingest_scip_json(
        &json!({
            "documents": [{
                "relative_path": "a.py",
                "symbols": [
                    {"symbol": "src", "relationships": [
                        {"symbol": "tgt", "is_definition": true},
                    ]},
                    {"symbol": "tgt"},
                ],
            }],
        }),
        "",
        "python",
    );
    assert_eq!(edges(&result)[0]["relation"], json!("scip_def"));
}

#[test]
fn relationship_priority_order_impl_typed_def_ref() {
    // When multiple flags are set, the priority is impl > typed > def > ref.
    let result = ingest_scip_json(
        &json!({
            "documents": [{
                "relative_path": "a.py",
                "symbols": [
                    {"symbol": "src", "relationships": [
                        {"symbol": "tgt", "is_implementation": true, "is_definition": true,
                         "is_type_definition": true, "is_reference": true},
                    ]},
                    {"symbol": "tgt"},
                ],
            }],
        }),
        "",
        "python",
    );
    assert_eq!(edges(&result)[0]["relation"], json!("scip_impl"));
}

#[test]
fn truthy_string_flag_does_not_count_as_set() {
    // External JSON may contain `"is_definition": "false"` — a truthy string
    // that must NOT count as a definition flag.
    let result = ingest_scip_json(
        &json!({
            "documents": [{
                "relative_path": "a.py",
                "symbols": [
                    {"symbol": "src", "relationships": [
                        {"symbol": "tgt", "is_definition": "false", "is_reference": true},
                    ]},
                    {"symbol": "tgt"},
                ],
            }],
        }),
        "",
        "python",
    );
    assert_eq!(edges(&result)[0]["relation"], json!("scip_ref"));
}

#[test]
fn missing_target_symbol_is_skipped() {
    let result = ingest_scip_json(
        &json!({
            "documents": [{
                "relative_path": "a.py",
                "symbols": [
                    {"symbol": "src", "relationships": [{"is_reference": true}]},
                ],
            }],
        }),
        "",
        "python",
    );
    assert_eq!(edges(&result).len(), 0);
}

#[test]
fn duplicate_relationship_emits_one_edge() {
    let result = ingest_scip_json(
        &json!({
            "documents": [{
                "relative_path": "a.py",
                "symbols": [
                    {"symbol": "src", "relationships": [
                        {"symbol": "tgt", "is_reference": true},
                        {"symbol": "tgt", "is_reference": true},
                    ]},
                    {"symbol": "tgt"},
                ],
            }],
        }),
        "",
        "python",
    );
    assert_eq!(edges(&result).len(), 1);
}

// ---------------------------------------------------------------------------
// Cross-document resolution
// ---------------------------------------------------------------------------

#[test]
fn cross_document_unique_match_resolves() {
    let result = ingest_scip_json(
        &json!({
            "documents": [
                {
                    "relative_path": "a.py",
                    "symbols": [{"symbol": "src", "relationships": [
                        {"symbol": "lib", "is_reference": true},
                    ]}],
                },
                {
                    "relative_path": "lib.py",
                    "symbols": [{"symbol": "lib"}],
                },
            ],
        }),
        "",
        "python",
    );
    // No stub created — the cross-doc target was found uniquely.
    assert_eq!(nodes(&result).len(), 2);
}

#[test]
fn cross_document_ambiguous_falls_back_to_stub() {
    let result = ingest_scip_json(
        &json!({
            "documents": [
                {
                    "relative_path": "a.py",
                    "symbols": [{"symbol": "src", "relationships": [
                        {"symbol": "shared", "is_reference": true},
                    ]}],
                },
                {
                    "relative_path": "lib1.py",
                    "symbols": [{"symbol": "shared"}],
                },
                {
                    "relative_path": "lib2.py",
                    "symbols": [{"symbol": "shared"}],
                },
            ],
        }),
        "",
        "python",
    );
    // src + 2 declarations + 1 stub = 4 nodes
    assert_eq!(nodes(&result).len(), 4);
}

// ---------------------------------------------------------------------------
// Node ID derivation
// ---------------------------------------------------------------------------

#[test]
fn node_id_is_deterministic() {
    let a = make_scip_node_id("foo#bar", "a.py");
    let b = make_scip_node_id("foo#bar", "a.py");
    assert_eq!(a, b);
}

#[test]
fn node_id_carries_safe_suffix() {
    let id = make_scip_node_id("scip-python python `pkg/foo`#Bar.baz()", "src.py");
    assert!(id.starts_with("scip_"));
    assert!(id.contains("baz"));
}

#[test]
fn node_id_empty_suffix_falls_back() {
    // A symbol whose last `#` segment contains no identifier characters
    // (e.g. `scip foo#` — trailing `#` leaves the suffix empty) collapses
    // to the bare `scip_<12hex>` form. Verify the exact format so a
    // refactor that changes the digest length or prefix is caught
    // immediately.
    let id = make_scip_node_id("scip foo#", "a.py");
    assert!(id.starts_with("scip_"));
    assert_eq!(id.len(), 17, "expected scip_ + 12 hex chars, got {id}");
    assert!(id.chars().skip(5).all(|c| c.is_ascii_hexdigit()));
}
