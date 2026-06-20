//! `FalkorDB` push export — `push_to_falkordb`.
//!
//! Mirrors Python `push_to_falkordb` from `graphify-py/graphify/export.py`.
//!
//! `FalkorDB` is `OpenCypher`-compatible, so the MERGE/SET upsert statements are
//! identical to the Neo4j path — they are reused verbatim from [`crate::neo4j`]
//! ([`merge_node_cypher`] / [`merge_edge_cypher`]). The transport differs:
//! `FalkorDB` is a Redis module, so the live push (behind the `falkordb` cargo
//! feature) connects with the `redis` client and runs each statement via the
//! `GRAPH.QUERY <graph_name> <cypher>` command instead of a Bolt session.
//!
//! The connection parsing ([`parse_falkordb_uri`]) is always compiled and unit
//! tested offline; only the `redis`-backed [`push_to_falkordb`] is gated.

/// Connection parameters resolved from a `FalkorDB` URI plus optional arg
/// credentials. The scheme is informational: `falkordb://localhost:6379`,
/// `redis://localhost:6379`, and a bare `localhost:6379` are all equivalent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalkorConn {
    /// Host (default `localhost`).
    pub host: String,
    /// Port (default `6379`).
    pub port: u16,
    /// Username — `None` for anonymous (`FalkorDB` auth is optional).
    pub username: Option<String>,
    /// Password — `None` for anonymous.
    pub password: Option<String>,
}

/// Resolve [`FalkorConn`] from a URI and optional fallback `user`/`password`.
///
/// Mirrors Python's `urlparse` logic: only host/port (and any embedded
/// credentials) are read from the URI. Credentials embedded in the URI take
/// precedence over the arguments; the fallback `user` is only applied when a
/// `password` is supplied (`FalkorDB` rejects a bare bolt-style default username
/// like Neo4j's `neo4j` as an unknown ACL user).
#[must_use]
pub fn parse_falkordb_uri(uri: &str, user: Option<&str>, password: Option<&str>) -> FalkorConn {
    // Empty args are treated as absent (Python's truthiness).
    let user = user.filter(|s| !s.is_empty());
    let password = password.filter(|s| !s.is_empty());

    let normalized = if uri.contains("://") {
        uri.to_string()
    } else {
        format!("redis://{uri}")
    };
    let parsed = url::Url::parse(&normalized).ok();

    let host = parsed
        .as_ref()
        .and_then(|u| u.host_str())
        .filter(|h| !h.is_empty())
        .unwrap_or("localhost")
        .to_string();
    let port = parsed.as_ref().and_then(url::Url::port).unwrap_or(6379);

    let uri_user = parsed
        .as_ref()
        .map(url::Url::username)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let uri_password = parsed
        .as_ref()
        .and_then(url::Url::password)
        .map(str::to_string);

    // connect_user = uri.username or (user if password else None)
    let username = uri_user.or_else(|| {
        if password.is_some() {
            user.map(str::to_string)
        } else {
            None
        }
    });
    // connect_password = uri.password or (password or None)
    let password = uri_password.or_else(|| password.map(str::to_string));

    FalkorConn {
        host,
        port,
        username,
        password,
    }
}

/// Push a graph to a running `FalkorDB` instance via the Redis `GRAPH.QUERY`
/// command. Nodes and edges are upserted with MERGE/SET, so re-running is safe.
/// Returns `(nodes_pushed, edges_pushed)`.
///
/// `graph_name` selects the named graph (`FalkorDB` keys each graph by name in the
/// same instance; default `graphify`). Requires the `falkordb` cargo feature.
///
/// # Errors
/// Returns [`FalkorDbError::Redis`] on a connection or query failure.
#[cfg(feature = "falkordb")]
pub fn push_to_falkordb(
    uri: &str,
    user: Option<&str>,
    password: Option<&str>,
    graph: &graphify_build::Graph,
    communities: &indexmap::IndexMap<i64, Vec<String>>,
    graph_name: &str,
) -> Result<(usize, usize), FalkorDbError> {
    use crate::neo4j::{build_edge_rows, build_node_rows, merge_edge_cypher, merge_node_cypher};
    use redis::IntoConnectionInfo;

    // `parse_falkordb_uri` swallows a parse failure and defaults to
    // localhost:6379 (mirroring Python's lenient `urlparse`, fine for the
    // offline cypher.txt path). In push mode that silent fallback could write a
    // graph to the wrong database, so fail fast here before any connection: the
    // strict `url` parser rejects a malformed URI that `urlparse` would have
    // limped past.
    let normalized = if uri.contains("://") {
        uri.to_string()
    } else {
        format!("redis://{uri}")
    };
    if url::Url::parse(&normalized).is_err() {
        return Err(FalkorDbError::InvalidUri(uri.to_string()));
    }

    let conn = parse_falkordb_uri(uri, user, password);
    let mut redis_info = redis::RedisConnectionInfo::default();
    if let Some(username) = conn.username {
        redis_info = redis_info.set_username(username);
    }
    if let Some(password) = conn.password {
        redis_info = redis_info.set_password(password);
    }
    let info = redis::ConnectionAddr::Tcp(conn.host, conn.port)
        .into_connection_info()?
        .set_redis_settings(redis_info);
    let client = redis::Client::open(info)?;
    let mut con = client.get_connection()?;

    let node_rows = build_node_rows(graph, communities);
    let edge_rows = build_edge_rows(graph);

    let mut nodes_pushed = 0usize;
    for row in &node_rows {
        redis::cmd("GRAPH.QUERY")
            .arg(graph_name)
            .arg(merge_node_cypher(row))
            .query::<redis::Value>(&mut con)?;
        nodes_pushed += 1;
    }
    let mut edges_pushed = 0usize;
    for row in &edge_rows {
        redis::cmd("GRAPH.QUERY")
            .arg(graph_name)
            .arg(merge_edge_cypher(row))
            .query::<redis::Value>(&mut con)?;
        edges_pushed += 1;
    }
    Ok((nodes_pushed, edges_pushed))
}

/// Errors produced by the `FalkorDB` push operation.
#[cfg(feature = "falkordb")]
#[derive(Debug, thiserror::Error)]
pub enum FalkorDbError {
    /// The connection URI could not be parsed; refused before connecting so a
    /// push never silently lands on the default localhost database.
    #[error("falkordb: could not parse connection URI {0:?}")]
    InvalidUri(String),

    /// A `redis` client / `GRAPH.QUERY` error.
    #[error("falkordb/redis error: {0}")]
    Redis(#[from] redis::RedisError),
}
