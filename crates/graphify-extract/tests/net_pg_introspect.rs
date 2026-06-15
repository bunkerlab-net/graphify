//! Live `PostgreSQL` introspection tests (#1271).
//!
//! These exercise `introspect_postgres` against a real `PostgreSQL` reachable at
//! `GRAPHIFY_TEST_POSTGRES_URL` (the CI `integration` job provides a
//! `postgres:18-alpine` service container). They self-skip when the variable is
//! unset, so they are no-ops in the default test runs and for local developers
//! without a database. Gated behind the `postgres` feature — the live connection
//! path is otherwise only verified by compilation, matching `graphify-py`, which
//! mocks the driver.
#![cfg(feature = "postgres")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;

use graphify_extract::pg_introspect::introspect_postgres;
use postgres::{Client, NoTls};

/// The DSN to test against, or `None` when the live database is not configured.
fn dsn() -> Option<String> {
    std::env::var("GRAPHIFY_TEST_POSTGRES_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

#[test]
fn net_postgres_introspects_live_schema() {
    let Some(dsn) = dsn() else {
        eprintln!("skipping: GRAPHIFY_TEST_POSTGRES_URL is not set");
        return;
    };

    // Seed a small schema with a foreign key. Object names are unique to this
    // test so concurrent `net_*` tests never collide on a shared database.
    let mut setup = Client::connect(&dsn, NoTls).expect("connect to seed schema");
    setup
        .batch_execute(
            "DROP TABLE IF EXISTS net_pg_orders;\
             DROP TABLE IF EXISTS net_pg_users;\
             CREATE TABLE net_pg_users (id INT PRIMARY KEY);\
             CREATE TABLE net_pg_orders (id INT PRIMARY KEY, \
                 user_id INT REFERENCES net_pg_users (id));",
        )
        .expect("seed schema");

    let res = introspect_postgres(&dsn).expect("introspect live database");

    let labels: HashSet<&str> = res.nodes.iter().map(|n| n.label.as_str()).collect();
    let node_id = |needle: &str| -> String {
        res.nodes
            .iter()
            .find(|n| n.label.contains(needle))
            .map_or_else(
                || panic!("table {needle} missing from introspection: {labels:?}"),
                |n| n.id.clone(),
            )
    };
    let users_id = node_id("net_pg_users");
    let orders_id = node_id("net_pg_orders");

    // Assert the *specific* seeded relationship rather than any `references`
    // edge, so an unrelated foreign key elsewhere in the catalog cannot mask a
    // regression in this one.
    assert!(
        res.edges.iter().any(|e| {
            e.relation == "references" && e.source == orders_id && e.target == users_id
        }),
        "expected a `references` edge net_pg_orders -> net_pg_users; edges: {:?}",
        res.edges
            .iter()
            .map(|e| (e.source.as_str(), e.relation.as_str(), e.target.as_str()))
            .collect::<Vec<_>>()
    );

    let _ = setup
        .batch_execute("DROP TABLE IF EXISTS net_pg_orders; DROP TABLE IF EXISTS net_pg_users;");
}
