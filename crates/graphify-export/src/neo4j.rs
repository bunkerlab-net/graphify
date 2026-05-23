//! Neo4j push export — `push_to_neo4j` / `push_to_neo4j_blocking`.
//!
//! Mirrors Python `push_to_neo4j` from `graphify-py/graphify/export.py`.
//!
//! Uses `neo4rs` (Bolt/async driver) to connect to a running Neo4j instance.
//! The function streams nodes and edges in batches of 200 via UNWIND.
//!
//! # Cypher generation
//!
//! `build_node_statements` and `build_edge_statements` are factored out so they
//! can be unit-tested without a live Neo4j connection.

use graphify_build::Graph;
use indexmap::IndexMap;
use neo4rs::{ConfigBuilder, Graph as Neo4jGraph, query};
use serde_json::Value;
use thiserror::Error;

use crate::{cypher_escape, cypher_label, node_community_map};

const BATCH_SIZE: usize = 200;

/// Errors produced by the Neo4j push operation.
#[derive(Debug, Error)]
pub enum Neo4jError {
    /// A `neo4rs` driver error.
    #[error("neo4j driver error: {0}")]
    Driver(#[from] neo4rs::Error),

    /// URI / configuration error.
    #[error("neo4j config error: {0}")]
    Config(String),
}

// ── Cypher row types ─────────────────────────────────────────────────────────

/// One node row for UNWIND batching.
#[derive(Debug, Clone)]
pub struct NodeRow {
    /// Primary node ID.
    pub id: String,
    /// Neo4j node label (safe identifier).
    pub label: String,
    /// Community ID (if any).
    pub community: Option<i64>,
    /// String properties to attach.
    pub props: IndexMap<String, String>,
}

/// One edge row for UNWIND batching.
#[derive(Debug, Clone)]
pub struct EdgeRow {
    pub src: String,
    pub tgt: String,
    /// Relationship type (safe identifier).
    pub rel_type: String,
    /// String properties to attach.
    pub props: IndexMap<String, String>,
}

// ── Cypher statement builders (pure, testable) ───────────────────────────────

/// Build the list of [`NodeRow`]s from a graph + communities map.
///
/// Mirrors the per-node loop in Python `push_to_neo4j`.
#[must_use]
pub fn build_node_rows(graph: &Graph, communities: &IndexMap<i64, Vec<String>>) -> Vec<NodeRow> {
    let node_community = node_community_map(communities);
    graph
        .nodes()
        .map(|(node_id, data)| {
            let ftype = data
                .get("file_type")
                .and_then(Value::as_str)
                .unwrap_or("entity");
            let capitalized: String = {
                let mut chars = ftype.chars();
                match chars.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
                }
            };
            let label = cypher_label(&capitalized, "Entity");
            let community = node_community.get(node_id).copied();

            // `data` is already the per-node attribute map; iterate it directly.
            let mut props: IndexMap<String, String> = IndexMap::new();
            for (k, v) in data {
                match v {
                    Value::String(s) => {
                        props.insert(k.clone(), s.clone());
                    }
                    Value::Number(n) => {
                        props.insert(k.clone(), n.to_string());
                    }
                    Value::Bool(b) => {
                        props.insert(k.clone(), b.to_string());
                    }
                    _ => {}
                }
            }

            NodeRow {
                id: node_id.clone(),
                label,
                community,
                props,
            }
        })
        .collect()
}

/// Build the list of [`EdgeRow`]s from a graph.
///
/// Mirrors the per-edge loop in Python `push_to_neo4j`.
#[must_use]
pub fn build_edge_rows(graph: &Graph) -> Vec<EdgeRow> {
    graph
        .edges()
        .map(|edge| {
            let relation_raw = edge
                .attrs
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("RELATED_TO");
            let rel_type = cypher_label(
                &relation_raw.to_uppercase().replace([' ', '-'], "_"),
                "RELATED_TO",
            );

            // `edge.attrs` is already an attribute `IndexMap`; iterate directly.
            let mut props: IndexMap<String, String> = IndexMap::new();
            for (k, v) in &edge.attrs {
                match v {
                    Value::String(s) => {
                        props.insert(k.clone(), s.clone());
                    }
                    Value::Number(n) => {
                        props.insert(k.clone(), n.to_string());
                    }
                    Value::Bool(b) => {
                        props.insert(k.clone(), b.to_string());
                    }
                    _ => {}
                }
            }

            EdgeRow {
                src: edge.source.clone(),
                tgt: edge.target.clone(),
                rel_type,
                props,
            }
        })
        .collect()
}

// ── Neo4j push ────────────────────────────────────────────────────────────────

/// Push a graph to a running Neo4j instance via the Bolt protocol.
///
/// Nodes and edges are streamed in batches of 200 using UNWIND. The operation
/// uses MERGE so re-running is safe (nodes / edges are upserted, not duplicated),
/// matching Python behaviour.
///
/// If `overwrite` is `true`, all existing nodes and relationships are deleted
/// before the push (`MATCH (n) DETACH DELETE n`).
///
/// Returns `(nodes_written, rels_written)`.
///
/// # Errors
///
/// Returns [`Neo4jError::Driver`] on Bolt connection or query failure, or
/// [`Neo4jError::Config`] if the URI / credentials are invalid.
pub async fn push_to_neo4j(
    uri: &str,
    user: &str,
    password: &str,
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    overwrite: bool,
) -> Result<(usize, usize), Neo4jError> {
    let config = ConfigBuilder::default()
        .uri(uri)
        .user(user)
        .password(password)
        .build()
        .map_err(|e| Neo4jError::Config(e.to_string()))?;
    let db = Neo4jGraph::connect(config).await?;

    if overwrite {
        db.run(query("MATCH (n) DETACH DELETE n")).await?;
    }

    let node_rows = build_node_rows(graph, communities);
    let edge_rows = build_edge_rows(graph);

    let nodes_written = push_nodes(&db, &node_rows).await?;
    let rels_written = push_edges(&db, &edge_rows).await?;

    Ok((nodes_written, rels_written))
}

/// Streams node rows to Neo4j in batches using MERGE, returning the count written.
async fn push_nodes(db: &Neo4jGraph, rows: &[NodeRow]) -> Result<usize, Neo4jError> {
    let mut written = 0usize;
    for chunk in rows.chunks(BATCH_SIZE) {
        for row in chunk {
            // Build a Cypher SET clause from props + community.
            let mut set_parts: Vec<String> = vec![format!("n.id = '{}'", cypher_escape(&row.id))];
            for (k, v) in &row.props {
                set_parts.push(format!("{k} = '{}'", cypher_escape(v)));
            }
            if let Some(cid) = row.community {
                set_parts.push(format!("n.community = {cid}"));
            }
            let set_clause = set_parts.join(", ");
            let cypher = format!(
                "MERGE (n:{label} {{id: '{id}'}}) SET {set_clause}",
                label = row.label,
                id = cypher_escape(&row.id),
            );
            db.run(query(&cypher)).await?;
            written += 1;
        }
    }
    Ok(written)
}

/// Streams edge rows to Neo4j in batches using MERGE, returning the count written.
async fn push_edges(db: &Neo4jGraph, rows: &[EdgeRow]) -> Result<usize, Neo4jError> {
    let mut written = 0usize;
    for chunk in rows.chunks(BATCH_SIZE) {
        for row in chunk {
            let mut set_parts: Vec<String> = vec![];
            for (k, v) in &row.props {
                set_parts.push(format!("r.{k} = '{}'", cypher_escape(v)));
            }
            let set_clause = if set_parts.is_empty() {
                String::new()
            } else {
                format!(" SET {}", set_parts.join(", "))
            };
            let cypher = format!(
                "MATCH (a {{id: '{src}'}}), (b {{id: '{tgt}'}}) MERGE (a)-[r:{rel}]->(b){set_clause}",
                src = cypher_escape(&row.src),
                tgt = cypher_escape(&row.tgt),
                rel = row.rel_type,
            );
            db.run(query(&cypher)).await?;
            written += 1;
        }
    }
    Ok(written)
}

// ── Blocking wrapper ──────────────────────────────────────────────────────────

/// Synchronous wrapper around [`push_to_neo4j`] for non-async callers.
///
/// Creates a single-threaded Tokio runtime internally. This avoids callers
/// needing to manage an executor themselves.
///
/// # Errors
///
/// Propagates [`Neo4jError`] from the async implementation, plus a
/// [`Neo4jError::Config`] if the Tokio runtime cannot be created.
pub fn push_to_neo4j_blocking(
    uri: &str,
    user: &str,
    password: &str,
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    overwrite: bool,
) -> Result<(usize, usize), Neo4jError> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| Neo4jError::Config(format!("failed to create tokio runtime: {e}")))?;
    rt.block_on(push_to_neo4j(
        uri,
        user,
        password,
        graph,
        communities,
        overwrite,
    ))
}
