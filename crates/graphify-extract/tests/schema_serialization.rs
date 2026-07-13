//! Serialization contract for the optional `Node.node_type` (`"type"`) and
//! `Node.metadata` / `Edge.metadata` fields (#1562). These optional keys must be
//! omitted when absent and, for a namespace node, `"type"` must serialize before
//! `"metadata"`. Guards the wide `metadata: None` / `node_type: None` codemod
//! against a silent serde regression.

#![allow(clippy::expect_used)]

use graphify_extract::types::{Edge, Node};
use indexmap::IndexMap;
use serde_json::{Value, json};

#[test]
fn ordinary_node_omits_type_and_metadata() {
    let n = Node {
        id: "a".to_string(),
        label: "A".to_string(),
        file_type: "code".to_string(),
        source_file: "a.cs".to_string(),
        source_location: Some("L1".to_string()),
        origin_file: None,
        node_type: None,
        metadata: None,
    };
    let v = serde_json::to_value(&n).expect("serialize node");
    let obj = v.as_object().expect("node object");
    assert!(
        !obj.contains_key("type"),
        "absent node_type must omit `type`: {obj:?}"
    );
    assert!(
        !obj.contains_key("metadata"),
        "absent metadata must be omitted: {obj:?}"
    );
}

#[test]
fn ordinary_edge_omits_metadata() {
    let e = Edge {
        source: "a".to_string(),
        target: "b".to_string(),
        relation: "calls".to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: "a.cs".to_string(),
        source_location: Some("L1".to_string()),
        weight: 1.0,
        context: Some("call".to_string()),
        confidence_score: Some(1.0),
        external: false,
        deferred: false,
        metadata: None,
    };
    let v = serde_json::to_value(&e).expect("serialize edge");
    assert!(
        !v.as_object().expect("edge object").contains_key("metadata"),
        "absent edge metadata must be omitted"
    );
}

#[test]
fn namespace_node_serializes_type_before_metadata() {
    let mut md: IndexMap<String, Value> = IndexMap::new();
    md.insert(
        "kind".to_string(),
        Value::String("csharp_namespace".to_string()),
    );
    let n = Node {
        id: "csharp_namespace:deadbeefdeadbeef".to_string(),
        label: "Game.Core".to_string(),
        file_type: "code".to_string(),
        source_file: "block.cs".to_string(),
        source_location: Some("L1".to_string()),
        origin_file: None,
        node_type: Some("namespace".to_string()),
        metadata: Some(md),
    };
    // Serialize to a string to assert key ORDER (`type` before `metadata`).
    let s = serde_json::to_string(&n).expect("serialize namespace node");
    assert_eq!(
        serde_json::from_str::<Value>(&s)
            .expect("reparse")
            .get("type")
            .and_then(Value::as_str),
        Some("namespace")
    );
    let type_at = s.find("\"type\"").expect("type key present");
    let meta_at = s.find("\"metadata\"").expect("metadata key present");
    assert!(
        type_at < meta_at,
        "`type` must serialize before `metadata`: {s}"
    );
}

#[test]
fn edge_metadata_serializes_when_present() {
    let expected =
        json!({"using_kind": "namespace", "target_fqn": "Game.Core", "alias": Value::Null});
    let md: IndexMap<String, Value> = expected
        .as_object()
        .expect("obj")
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let e = Edge {
        source: "f.cs".to_string(),
        target: "csharp_namespace:deadbeefdeadbeef".to_string(),
        relation: "imports".to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: "f.cs".to_string(),
        source_location: Some("L1".to_string()),
        weight: 1.0,
        context: Some("import".to_string()),
        confidence_score: Some(1.0),
        external: false,
        deferred: false,
        metadata: Some(md),
    };
    let v = serde_json::to_value(&e).expect("serialize edge");
    assert_eq!(
        v.get("metadata")
            .and_then(|m| m.get("using_kind"))
            .and_then(Value::as_str),
        Some("namespace")
    );
}
