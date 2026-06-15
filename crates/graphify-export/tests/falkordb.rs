//! Offline tests for the `FalkorDB` export.
//!
//! The live `push_to_falkordb` is behind the `falkordb` cargo feature and needs
//! a running instance (the Python suite is likewise docker-gated), so it is
//! verified by compilation. Here we cover the always-compiled pieces: connection
//! resolution (`parse_falkordb_uri`) and the `OpenCypher` MERGE/SET statements
//! reused from the Neo4j path (incl. the cypher-injection guard).

#![allow(clippy::expect_used)]

use graphify_build::build_from_json;
use graphify_export::neo4j::{
    build_edge_rows, build_node_rows, merge_edge_cypher, merge_node_cypher,
};
use graphify_export::parse_falkordb_uri;
use indexmap::IndexMap;
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;

// ── parse_falkordb_uri ─────────────────────────────────────────────────────

#[test]
fn bare_host_port_defaults_to_redis_scheme() {
    let c = parse_falkordb_uri("localhost:6379", None, None);
    assert_eq!(c.host, "localhost");
    assert_eq!(c.port, 6379);
    assert_eq!(c.username, None);
    assert_eq!(c.password, None);
}

#[test]
fn falkordb_and_redis_schemes_are_equivalent() {
    let a = parse_falkordb_uri("falkordb://db.example:7000", None, None);
    let b = parse_falkordb_uri("redis://db.example:7000", None, None);
    assert_eq!(a.host, "db.example");
    assert_eq!(a.port, 7000);
    assert_eq!(a, b);
}

#[test]
fn missing_host_and_port_fall_back_to_defaults() {
    // No authority → defaults.
    let c = parse_falkordb_uri("redis://", None, None);
    assert_eq!(c.host, "localhost");
    assert_eq!(c.port, 6379);
}

#[test]
fn uri_credentials_take_precedence_over_args() {
    let c = parse_falkordb_uri("redis://alice:s3cret@host:6379", Some("bob"), Some("other"));
    assert_eq!(c.username.as_deref(), Some("alice"));
    assert_eq!(c.password.as_deref(), Some("s3cret"));
}

#[test]
fn arg_user_applied_only_when_password_given() {
    // Password present → fall back to the arg username.
    let with_pw = parse_falkordb_uri("redis://host:6379", Some("bob"), Some("pw"));
    assert_eq!(with_pw.username.as_deref(), Some("bob"));
    assert_eq!(with_pw.password.as_deref(), Some("pw"));

    // No password → a bare username arg is dropped (FalkorDB rejects an unknown
    // bolt-style default ACL user).
    let no_pw = parse_falkordb_uri("redis://host:6379", Some("neo4j"), None);
    assert_eq!(no_pw.username, None);
    assert_eq!(no_pw.password, None);
}

#[test]
fn empty_arg_credentials_are_treated_as_absent() {
    let c = parse_falkordb_uri("redis://host:6379", Some(""), Some(""));
    assert_eq!(c.username, None);
    assert_eq!(c.password, None);
}

// ── push fail-fast on an unparseable URI (gated) ────────────────────────────

#[cfg(feature = "falkordb")]
#[test]
fn push_rejects_unparseable_uri_before_connecting() {
    // A malformed URI must error out before any connection attempt rather than
    // silently defaulting to localhost:6379 and writing to the wrong database.
    let g = one_node_graph();
    let communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    let err = graphify_export::falkordb::push_to_falkordb(
        "ht!tp://nope",
        None,
        None,
        &g,
        &communities,
        "graphify",
    )
    .expect_err("malformed URI must error");
    assert!(
        matches!(err, graphify_export::falkordb::FalkorDbError::InvalidUri(_)),
        "expected InvalidUri, got {err:?}"
    );
}

// ── OpenCypher MERGE/SET generation (shared with Neo4j) ─────────────────────

fn one_node_graph() -> graphify_build::Graph {
    build_from_json(
        json!({
            "nodes": [{"id": "n0", "label": "Alpha", "file_type": "code", "source_file": "a.py"}],
            "edges": []
        }),
        false,
        None,
    )
    .expect("build graph")
}

#[test]
fn merge_node_cypher_emits_label_and_id() {
    let g = one_node_graph();
    let communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    let rows = build_node_rows(&g, &communities);
    let cypher = merge_node_cypher(&rows[0]);
    // file_type "code" → capitalized label "Code".
    assert!(
        cypher.starts_with("MERGE (n:Code {id: 'n0'}) SET "),
        "{cypher}"
    );
    assert!(cypher.contains("n.id = 'n0'"));
}

#[test]
fn merge_edge_cypher_emits_match_and_rel() -> TestResult {
    let g = build_from_json(
        json!({
            "nodes": [
                {"id": "n0", "label": "A", "file_type": "code", "source_file": "a.py"},
                {"id": "n1", "label": "B", "file_type": "code", "source_file": "b.py"},
            ],
            "edges": [{"source": "n0", "target": "n1", "relation": "calls", "confidence": "EXTRACTED"}]
        }),
        false,
        None,
    )?;
    let rows = build_edge_rows(&g);
    let cypher = merge_edge_cypher(&rows[0]);
    assert!(
        cypher.contains("MATCH (a {id: 'n0'}), (b {id: 'n1'})"),
        "{cypher}"
    );
    // "calls" → uppercased relation type CALLS.
    assert!(cypher.contains("MERGE (a)-[r:CALLS]->(b)"), "{cypher}");
    Ok(())
}

#[test]
fn merge_node_cypher_escapes_injection_in_values() -> TestResult {
    // A label/id containing a quote must be escaped so it cannot break out of
    // the MERGE/SET clause (cypher-injection guard).
    let g = build_from_json(
        json!({
            "nodes": [{"id": "n'); DROP", "label": "x", "file_type": "code", "source_file": "a.py"}],
            "edges": []
        }),
        false,
        None,
    )?;
    let communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    let rows = build_node_rows(&g, &communities);
    let cypher = merge_node_cypher(&rows[0]);
    // The raw unescaped `n'); DROP` sequence must not appear verbatim.
    assert!(
        !cypher.contains("n'); DROP"),
        "unescaped injection survived: {cypher}"
    );
    Ok(())
}
