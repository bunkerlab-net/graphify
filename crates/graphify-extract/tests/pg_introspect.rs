//! Port of `graphify-py/tests/test_pg_introspect.py`.
//!
//! The Python tests mock the `psycopg` driver; the Rust port exercises the pure
//! catalog → DDL → graph core (`introspect_catalog`) with fixture catalogs, plus
//! the connection-error sanitizer. The live connection (`introspect_postgres`)
//! is feature-gated and not exercised here (it has no offline test path), exactly
//! as the Python live path is never hit under mocks.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;

use graphify_extract::{
    FileResult, PgCatalog, PgForeignKey, PgRoutine, PgTable, PgView, introspect_catalog,
    sanitize_connection_error,
};

fn table(schema: &str, name: &str, ty: &str) -> PgTable {
    PgTable {
        schema: schema.into(),
        name: name.into(),
        table_type: ty.into(),
    }
}

fn fk(
    cname: &str,
    t_schema: &str,
    t_name: &str,
    cols: &[&str],
    r_schema: &str,
    r_name: &str,
    r_cols: &[&str],
) -> PgForeignKey {
    PgForeignKey {
        constraint_name: cname.into(),
        table_schema: t_schema.into(),
        table_name: t_name.into(),
        columns: cols.iter().map(|s| (*s).to_string()).collect(),
        foreign_table_schema: r_schema.into(),
        foreign_table_name: r_name.into(),
        foreign_columns: r_cols.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn labels(r: &FileResult) -> HashSet<&str> {
    r.nodes.iter().map(|n| n.label.as_str()).collect()
}

/// The label form tree-sitter produces for a quoted `"schema"."name"` reference.
fn q(schema: &str, name: &str) -> String {
    format!("\"{schema}\".\"{name}\"")
}

/// `test_pg_introspect_success`
#[test]
fn pg_introspect_success() {
    let catalog = PgCatalog {
        tables: vec![
            table("public", "users", "BASE TABLE"),
            table("public", "orders", "BASE TABLE"),
        ],
        views: vec![PgView {
            schema: "public".into(),
            name: "active_users".into(),
            definition: Some("SELECT * FROM public.users WHERE active = true".into()),
        }],
        routines: vec![
            PgRoutine {
                schema: "public".into(),
                name: "calculate_total".into(),
                routine_type: "FUNCTION".into(),
                definition: Some("SELECT 42;".into()),
                external_language: Some("SQL".into()),
            },
            PgRoutine {
                schema: "public".into(),
                name: "do_nothing".into(),
                routine_type: "PROCEDURE".into(),
                definition: None,
                external_language: Some("PLPGSQL".into()),
            },
        ],
        foreign_keys: vec![fk(
            "fk_orders_user_id",
            "public",
            "orders",
            &["user_id"],
            "public",
            "users",
            &["id"],
        )],
    };

    let res = introspect_catalog(&catalog, "myhost", "mydb");

    // source_file is the sanitized virtual path (no credentials, `//` collapsed).
    for node in &res.nodes {
        assert_eq!(node.source_file, "postgresql:/myhost/mydb");
    }
    for edge in &res.edges {
        assert_eq!(edge.source_file, "postgresql:/myhost/mydb");
    }

    // Node labels: quoted object references for tables, views, and functions.
    let ls = labels(&res);
    assert!(
        ls.contains(q("public", "users").as_str()),
        "users missing: {ls:?}"
    );
    assert!(
        ls.contains(q("public", "orders").as_str()),
        "orders missing: {ls:?}"
    );
    assert!(
        ls.contains(q("public", "active_users").as_str()),
        "active_users missing: {ls:?}"
    );
    assert!(
        ls.contains(format!("{}()", q("public", "calculate_total")).as_str()),
        "calculate_total() missing: {ls:?}"
    );
    assert!(
        ls.contains(format!("{}()", q("public", "do_nothing")).as_str()),
        "do_nothing() missing: {ls:?}"
    );

    // File node label == dbname.
    let file_nodes: Vec<_> = res
        .nodes
        .iter()
        .filter(|n| n.file_type == "code" && n.label == "mydb")
        .collect();
    assert_eq!(
        file_nodes.len(),
        1,
        "expected one file node labelled `mydb`"
    );

    // FK references edge orders → users, exactly once.
    let users_nid = &res
        .nodes
        .iter()
        .find(|n| n.label == q("public", "users"))
        .unwrap()
        .id;
    let orders_nid = &res
        .nodes
        .iter()
        .find(|n| n.label == q("public", "orders"))
        .unwrap()
        .id;
    let ref_edges: Vec<_> = res
        .edges
        .iter()
        .filter(|e| &e.source == orders_nid && &e.target == users_nid && e.relation == "references")
        .collect();
    assert_eq!(ref_edges.len(), 1, "expected exactly 1 references edge");
}

/// `test_pg_introspect_quoted_identifiers`
#[test]
fn pg_introspect_quoted_identifiers() {
    let catalog = PgCatalog {
        tables: vec![
            table("public", "order", "BASE TABLE"),     // reserved word
            table("public", "user-data", "BASE TABLE"), // hyphen
        ],
        foreign_keys: vec![fk(
            "fk_userdata_order",
            "public",
            "user-data",
            &["owner_id"],
            "public",
            "order",
            &["id"],
        )],
        ..Default::default()
    };

    let res = introspect_catalog(&catalog, "myhost", "mydb");
    let ls = labels(&res);
    assert!(
        ls.contains(q("public", "order").as_str()),
        "order missing: {ls:?}"
    );
    assert!(
        ls.contains(q("public", "user-data").as_str()),
        "user-data missing: {ls:?}"
    );

    let order_nid = &res
        .nodes
        .iter()
        .find(|n| n.label == q("public", "order"))
        .unwrap()
        .id;
    let userdata_nid = &res
        .nodes
        .iter()
        .find(|n| n.label == q("public", "user-data"))
        .unwrap()
        .id;
    assert!(
        res.edges.iter().any(|e| &e.source == userdata_nid
            && &e.target == order_nid
            && e.relation == "references"),
        "FK edge user-data -> order missing"
    );
}

/// `test_pg_introspect_composite_fk`: a 2-column composite FK must produce
/// exactly ONE references edge, not two.
#[test]
fn pg_introspect_composite_fk() {
    let catalog = PgCatalog {
        tables: vec![
            table("public", "products", "BASE TABLE"),
            table("public", "order_items", "BASE TABLE"),
        ],
        foreign_keys: vec![fk(
            "fk_order_items_composite",
            "public",
            "order_items",
            &["order_id", "product_id"],
            "public",
            "products",
            &["order_id", "product_id"],
        )],
        ..Default::default()
    };

    let res = introspect_catalog(&catalog, "myhost", "mydb");
    let products_nid = &res
        .nodes
        .iter()
        .find(|n| n.label == q("public", "products"))
        .unwrap()
        .id;
    let order_items_nid = &res
        .nodes
        .iter()
        .find(|n| n.label == q("public", "order_items"))
        .unwrap()
        .id;
    let ref_edges: Vec<_> = res
        .edges
        .iter()
        .filter(|e| {
            &e.source == order_items_nid && &e.target == products_nid && e.relation == "references"
        })
        .collect();
    assert_eq!(ref_edges.len(), 1, "composite FK must yield exactly 1 edge");
}

/// `test_pg_introspect_connection_error`: a driver error is surfaced as a
/// sanitized message — stable prefix, only the first line, no DSN/credentials.
#[test]
fn pg_introspect_connection_error() {
    let raw = "connection to server at \"myhost\" (127.0.0.1), port 5432 failed: \
               FATAL: password authentication failed for user \"myuser\"\n\
               DETAIL: Connection matched pg_hba.conf line 1: …";
    let msg = sanitize_connection_error(raw);
    assert!(msg.contains("could not connect to PostgreSQL"));
    assert!(!msg.contains("secret"), "credentials must not leak: {msg}");
    assert!(
        !msg.contains("DETAIL"),
        "only the first line should survive: {msg}"
    );
}
