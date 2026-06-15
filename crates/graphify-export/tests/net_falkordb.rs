//! Live `FalkorDB` push tests (#1175), ported from
//! `graphify-py/tests/test_falkordb_integration.py`.
//!
//! These run `push_to_falkordb` against a real `FalkorDB` reachable at
//! `GRAPHIFY_TEST_FALKORDB_URL` (the CI `integration` job provides a
//! `falkordb/falkordb:v4.18.9-alpine` service container) and read the result
//! back with `GRAPH.QUERY`. They self-skip when the variable is unset, so they
//! are no-ops in the default test runs and for local developers without a
//! server.
//!
//! `FalkorDB` — not plain Redis/Valkey — is required: the push speaks the
//! `GRAPH.QUERY` graph-module command, which a vanilla key-value server does not
//! implement. Gated behind the `falkordb` feature.
#![cfg(feature = "falkordb")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use graphify_build::{Graph, build_from_json};
use graphify_export::falkordb::push_to_falkordb;
use indexmap::IndexMap;
use serde_json::json;

/// The bare `host:port` to test against, or `None` when unconfigured.
fn target() -> Option<String> {
    std::env::var("GRAPHIFY_TEST_FALKORDB_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// A small 3-node / 2-edge graph.
fn sample_graph() -> Graph {
    build_from_json(
        json!({
            "nodes": [
                {"id": "n0", "label": "A", "file_type": "code", "source_file": "a.py"},
                {"id": "n1", "label": "B", "file_type": "code", "source_file": "b.py"},
                {"id": "n2", "label": "C", "file_type": "code", "source_file": "c.py"},
            ],
            "edges": [
                {"source": "n0", "target": "n1", "relation": "calls", "confidence": "EXTRACTED"},
                {"source": "n1", "target": "n2", "relation": "calls", "confidence": "EXTRACTED"},
            ]
        }),
        false,
        None,
    )
    .expect("build graph")
}

/// Extract the leading scalar of a `GRAPH.QUERY` reply (`[header, rows, stats]`,
/// with `rows[0][0]` holding the value of a single-column / single-row query).
fn first_scalar_int(reply: &redis::Value) -> Option<i64> {
    let redis::Value::Array(top) = reply else {
        return None;
    };
    let redis::Value::Array(rows) = top.get(1)? else {
        return None;
    };
    let redis::Value::Array(first_row) = rows.first()? else {
        return None;
    };
    match first_row.first()? {
        redis::Value::Int(i) => Some(*i),
        redis::Value::BulkString(b) => std::str::from_utf8(b).ok()?.trim().parse().ok(),
        redis::Value::SimpleString(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Open a raw connection for read-back / cleanup, given the bare `host:port`.
fn connect(target: &str) -> redis::Connection {
    redis::Client::open(format!("redis://{target}"))
        .expect("redis client")
        .get_connection()
        .expect("redis connection")
}

fn graph_query(con: &mut redis::Connection, graph: &str, cypher: &str) -> redis::Value {
    redis::cmd("GRAPH.QUERY")
        .arg(graph)
        .arg(cypher)
        .query(con)
        .expect("GRAPH.QUERY")
}

fn count(con: &mut redis::Connection, graph: &str, cypher: &str) -> i64 {
    first_scalar_int(&graph_query(con, graph, cypher)).expect("scalar count in GRAPH.QUERY reply")
}

fn delete_graph(con: &mut redis::Connection, graph: &str) {
    // GRAPH.DELETE errors if the graph does not exist; that is fine for cleanup.
    let _: Result<redis::Value, _> = redis::cmd("GRAPH.DELETE").arg(graph).query(con);
}

#[test]
fn net_falkordb_push_creates_expected_graph() {
    let Some(target) = target() else {
        eprintln!("skipping: GRAPHIFY_TEST_FALKORDB_URL is not set");
        return;
    };
    let graph_name = "graphify_net_create";
    let mut con = connect(&target);
    delete_graph(&mut con, graph_name);

    let g = sample_graph();
    let communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    let (nodes, edges) =
        push_to_falkordb(&target, None, None, &g, &communities, graph_name).expect("push");
    assert_eq!((nodes, edges), (3, 2));

    assert_eq!(count(&mut con, graph_name, "MATCH (n) RETURN count(n)"), 3);
    assert_eq!(
        count(&mut con, graph_name, "MATCH ()-[r]->() RETURN count(r)"),
        2
    );

    delete_graph(&mut con, graph_name);
}

#[test]
fn net_falkordb_push_is_idempotent() {
    let Some(target) = target() else {
        eprintln!("skipping: GRAPHIFY_TEST_FALKORDB_URL is not set");
        return;
    };
    let graph_name = "graphify_net_idem";
    let mut con = connect(&target);
    delete_graph(&mut con, graph_name);

    let g = sample_graph();
    let communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    // MERGE-based push must be safe to re-run: counts must not grow.
    push_to_falkordb(&target, None, None, &g, &communities, graph_name).expect("first push");
    push_to_falkordb(&target, None, None, &g, &communities, graph_name).expect("second push");

    assert_eq!(count(&mut con, graph_name, "MATCH (n) RETURN count(n)"), 3);
    assert_eq!(
        count(&mut con, graph_name, "MATCH ()-[r]->() RETURN count(r)"),
        2
    );

    delete_graph(&mut con, graph_name);
}
