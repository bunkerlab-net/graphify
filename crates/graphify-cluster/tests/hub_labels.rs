//! Parity tests against `graphify-py/tests/test_community_hub_labels.py`:
//! deterministic, LLM-free community labels + membership fingerprints.

#![allow(clippy::expect_used)]

use graphify_build::{Graph, GraphKind};
use graphify_cluster::{community_member_sigs, label_communities_by_hub};
use indexmap::IndexMap;
use serde_json::json;

/// Build a graph from `(node_id, optional_label)` pairs and undirected edges.
fn build(node_labels: &[(&str, Option<&str>)], edges: &[(&str, &str)]) -> Graph {
    let mut g = Graph::new(GraphKind::Graph);
    for (nid, label) in node_labels {
        let mut attrs = IndexMap::new();
        if let Some(l) = label {
            attrs.insert("label".to_string(), json!(l));
        }
        g.add_node(nid, attrs);
    }
    for (u, v) in edges {
        g.add_edge(u, v, IndexMap::new());
    }
    g
}

fn comms(pairs: &[(i64, &[&str])]) -> IndexMap<i64, Vec<String>> {
    pairs
        .iter()
        .map(|(cid, members)| (*cid, members.iter().map(|s| (*s).to_string()).collect()))
        .collect()
}

#[test]
fn test_labels_by_highest_degree_hub() {
    // 'a' is the hub (degree 3); the community is named after it, "()" stripped.
    let g = build(
        &[
            ("a", Some("log_action()")),
            ("b", Some("b()")),
            ("c", Some("c()")),
            ("d", Some("d()")),
        ],
        &[("a", "b"), ("a", "c"), ("a", "d")],
    );
    let labels = label_communities_by_hub(&g, &comms(&[(0, &["a", "b", "c", "d"])]));
    assert_eq!(labels[&0], "log_action");
}

#[test]
fn test_not_a_placeholder_for_a_real_community() {
    let g = build(
        &[("a", Some("handler()")), ("b", Some("b()"))],
        &[("a", "b")],
    );
    let labels = label_communities_by_hub(&g, &comms(&[(0, &["a", "b"])]));
    assert_eq!(labels[&0], "handler");
    assert_ne!(labels[&0], "Community 0");
}

#[test]
fn test_tie_breaks_deterministically_by_node_id() {
    // both nodes degree 1 → the lexicographically smaller id wins, regardless of order.
    let g = build(&[("z", Some("z()")), ("a", Some("a()"))], &[("z", "a")]);
    assert_eq!(
        label_communities_by_hub(&g, &comms(&[(0, &["z", "a"])]))[&0],
        "a"
    );
    assert_eq!(
        label_communities_by_hub(&g, &comms(&[(0, &["a", "z"])]))[&0],
        "a"
    );
}

#[test]
fn test_absent_members_fall_back_to_placeholder() {
    // no member of community 5 is in the graph → keep the "Community N" placeholder.
    let g = build(&[("a", Some("a()"))], &[]);
    let labels = label_communities_by_hub(&g, &comms(&[(5, &["ghost1", "ghost2"])]));
    assert_eq!(labels[&5], "Community 5");
}

#[test]
fn test_node_without_label_attr_uses_id() {
    // hub degree 2, no label attrs → falls back to the node id.
    let g = build(
        &[("hub", None), ("x", None), ("y", None)],
        &[("hub", "x"), ("hub", "y")],
    );
    let labels = label_communities_by_hub(&g, &comms(&[(0, &["hub", "x", "y"])]));
    assert_eq!(labels[&0], "hub");
}

#[test]
fn test_multiple_communities_each_get_their_own_hub() {
    let g = build(
        &[
            ("h1", Some("auth()")),
            ("a1", Some("a1()")),
            ("a2", Some("a2()")),
            ("h2", Some("billing()")),
            ("b1", Some("b1()")),
            ("b2", Some("b2()")),
        ],
        &[("h1", "a1"), ("h1", "a2"), ("h2", "b1"), ("h2", "b2")],
    );
    let labels = label_communities_by_hub(
        &g,
        &comms(&[(0, &["h1", "a1", "a2"]), (1, &["h2", "b1", "b2"])]),
    );
    assert_eq!(labels[&0], "auth");
    assert_eq!(labels[&1], "billing");
}

#[test]
fn test_community_member_sigs_are_deterministic_and_order_independent() {
    let a = community_member_sigs(&comms(&[(0, &["x", "y", "z"]), (1, &["a"])]));
    let b = community_member_sigs(&comms(&[(0, &["z", "x", "y"]), (1, &["a"])]));
    assert_eq!(a, b);
    assert_ne!(a[&0], a[&1]);
}

#[test]
fn test_community_member_sigs_change_when_membership_changes() {
    let before = community_member_sigs(&comms(&[(0, &["x", "y", "z"])]));
    let after = community_member_sigs(&comms(&[(0, &["x", "y"])]));
    assert_ne!(
        before[&0], after[&0],
        "signature must change when a community's members change"
    );
}
