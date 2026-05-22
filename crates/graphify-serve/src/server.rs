//! MCP stdio JSON-RPC server transport.
//!
//! The Python `mcp` package uses **line-delimited JSON** (newline-framed, no
//! Content-Length header). Each message is one JSON object per line. Blank
//! lines are silently dropped (some MCP clients emit them between messages).
//!
//! We hand-roll the protocol instead of pulling in `rmcp` so there is no
//! additional dependency on the MCP SDK and the test surface is smaller.

use std::collections::HashMap;
use std::path::Path;

use graphify_build::Graph;
use indexmap::IndexMap;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::tools::{
    load_community_labels, maybe_reload, resource_audit, resource_questions, resource_surprises,
    tool_get_community, tool_get_neighbors, tool_get_node, tool_god_nodes, tool_graph_stats,
    tool_query_graph, tool_shortest_path,
};
use crate::{ReloadState, ServeError};

// ── Tool schema ───────────────────────────────────────────────────────────────

fn tools_list() -> Vec<Value> {
    vec![
        json!({
            "name": "query_graph",
            "description": "Search the knowledge graph using BFS or DFS. Returns relevant nodes and edges as text context.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": {"type": "string", "description": "Natural language question or keyword search"},
                    "mode": {"type": "string", "enum": ["bfs", "dfs"], "default": "bfs",
                             "description": "bfs=broad context, dfs=trace a specific path"},
                    "depth": {"type": "integer", "default": 3, "description": "Traversal depth (1-6)"},
                    "token_budget": {"type": "integer", "default": 2000, "description": "Max output tokens"},
                    "context_filter": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Optional explicit edge-context filter, e.g. ['call', 'field']"
                    }
                },
                "required": ["question"]
            }
        }),
        json!({
            "name": "get_node",
            "description": "Get full details for a specific node by label or ID.",
            "inputSchema": {
                "type": "object",
                "properties": {"label": {"type": "string", "description": "Node label or ID to look up"}},
                "required": ["label"]
            }
        }),
        json!({
            "name": "get_neighbors",
            "description": "Get all direct neighbors of a node with edge details.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "label": {"type": "string"},
                    "relation_filter": {"type": "string", "description": "Optional: filter by relation type"}
                },
                "required": ["label"]
            }
        }),
        json!({
            "name": "get_community",
            "description": "Get all nodes in a community by community ID.",
            "inputSchema": {
                "type": "object",
                "properties": {"community_id": {"type": "integer", "description": "Community ID (0-indexed by size)"}},
                "required": ["community_id"]
            }
        }),
        json!({
            "name": "god_nodes",
            "description": "Return the most connected nodes - the core abstractions of the knowledge graph.",
            "inputSchema": {"type": "object", "properties": {"top_n": {"type": "integer", "default": 10}}}
        }),
        json!({
            "name": "graph_stats",
            "description": "Return summary statistics: node count, edge count, communities, confidence breakdown.",
            "inputSchema": {"type": "object", "properties": {}}
        }),
        json!({
            "name": "shortest_path",
            "description": "Find the shortest path between two concepts in the knowledge graph.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {"type": "string", "description": "Source concept label or keyword"},
                    "target": {"type": "string", "description": "Target concept label or keyword"},
                    "max_hops": {"type": "integer", "default": 8, "description": "Maximum hops to consider"}
                },
                "required": ["source", "target"]
            }
        }),
    ]
}

fn resources_list() -> Vec<Value> {
    vec![
        json!({"uri": "graphify://report", "name": "Graph Report", "description": "Full GRAPH_REPORT.md", "mimeType": "text/markdown"}),
        json!({"uri": "graphify://stats", "name": "Graph Stats", "description": "Node/edge/community counts and confidence breakdown", "mimeType": "text/plain"}),
        json!({"uri": "graphify://god-nodes", "name": "God Nodes", "description": "Top 10 most-connected nodes", "mimeType": "text/plain"}),
        json!({"uri": "graphify://surprises", "name": "Surprising Connections", "description": "Cross-community surprising connections", "mimeType": "text/plain"}),
        json!({"uri": "graphify://audit", "name": "Confidence Audit", "description": "EXTRACTED/INFERRED/AMBIGUOUS edge breakdown", "mimeType": "text/plain"}),
        json!({"uri": "graphify://questions", "name": "Suggested Questions", "description": "Suggested questions for this codebase", "mimeType": "text/plain"}),
    ]
}

// ── JSON-RPC helpers ──────────────────────────────────────────────────────────

// needless_pass_by_value: `result` is passed to `json!` by value; refactoring
// to `&Value` would require cloning at every call site.
#[allow(clippy::needless_pass_by_value)]
fn ok_response(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error_response(id: &Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message}
    })
}

// ── Message dispatcher ────────────────────────────────────────────────────────

fn dispatch(
    msg: &Value,
    graph: &mut Graph,
    communities: &mut IndexMap<i64, Vec<String>>,
    reload_state: &mut ReloadState,
    idf_cache: &mut HashMap<String, f64>,
    graph_path: &str,
) -> Option<Value> {
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let empty_obj = serde_json::Map::new();
    let params = msg
        .get("params")
        .and_then(Value::as_object)
        .unwrap_or(&empty_obj);

    // Notifications (no id) don't get a response.
    let is_notification = msg.get("id").is_none();

    match method {
        "initialize" => {
            if is_notification {
                return None;
            }
            Some(ok_response(
                &id,
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {},
                        "resources": {}
                    },
                    "serverInfo": {
                        "name": "graphify",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            ))
        }
        "notifications/initialized" => None,
        "tools/list" => {
            if is_notification {
                return None;
            }
            Some(ok_response(&id, json!({"tools": tools_list()})))
        }
        "tools/call" => {
            if is_notification {
                return None;
            }
            maybe_reload(graph_path, graph, communities, reload_state);
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let empty_args = serde_json::Map::new();
            let arguments = params
                .get("arguments")
                .and_then(Value::as_object)
                .unwrap_or(&empty_args);
            let text = dispatch_tool(name, graph, communities, arguments, idf_cache);
            Some(ok_response(
                &id,
                json!({"content": [{"type": "text", "text": text}]}),
            ))
        }
        "resources/list" => {
            if is_notification {
                return None;
            }
            Some(ok_response(&id, json!({"resources": resources_list()})))
        }
        "resources/read" => {
            if is_notification {
                return None;
            }
            maybe_reload(graph_path, graph, communities, reload_state);
            let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
            let text = dispatch_resource(uri, graph, communities, graph_path);
            Some(ok_response(
                &id,
                json!({"contents": [{"uri": uri, "mimeType": "text/plain", "text": text}]}),
            ))
        }
        _ => {
            if is_notification {
                return None;
            }
            Some(error_response(
                &id,
                -32_601,
                &format!("Method not found: {method}"),
            ))
        }
    }
}

fn dispatch_tool(
    name: &str,
    graph: &mut Graph,
    communities: &IndexMap<i64, Vec<String>>,
    arguments: &serde_json::Map<String, Value>,
    idf_cache: &mut HashMap<String, f64>,
) -> String {
    match name {
        "query_graph" => tool_query_graph(graph, arguments, idf_cache),
        "get_node" => tool_get_node(graph, arguments),
        "get_neighbors" => tool_get_neighbors(graph, arguments),
        "get_community" => tool_get_community(graph, communities, arguments),
        "god_nodes" => tool_god_nodes(graph, arguments),
        "graph_stats" => tool_graph_stats(graph, communities),
        "shortest_path" => tool_shortest_path(graph, arguments, idf_cache),
        other => format!("Unknown tool: {other}"),
    }
}

fn dispatch_resource(
    uri: &str,
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    graph_path: &str,
) -> String {
    match uri {
        "graphify://report" => {
            let report_path = Path::new(graph_path)
                .parent()
                .map(|p| p.join("GRAPH_REPORT.md"));
            match report_path.and_then(|p| std::fs::read_to_string(p).ok()) {
                Some(text) => text,
                None => "GRAPH_REPORT.md not found. Run graphify extract first.".to_string(),
            }
        }
        "graphify://stats" => tool_graph_stats(graph, communities),
        "graphify://god-nodes" => {
            let mut args = serde_json::Map::new();
            args.insert("top_n".to_string(), json!(10));
            tool_god_nodes(graph, &args)
        }
        "graphify://surprises" => resource_surprises(graph, communities),
        "graphify://audit" => resource_audit(graph),
        "graphify://questions" => {
            let community_labels = load_community_labels(graph_path, communities);
            resource_questions(graph, communities, &community_labels)
        }
        other => format!("Unknown resource: {other}"),
    }
}

// ── Stdio server loop ─────────────────────────────────────────────────────────

/// Run the MCP server on the provided async reader/writer pair.
///
/// Reads line-delimited JSON messages, dispatches them, and writes responses.
/// Blank lines are silently dropped (mirrors Python `_filter_blank_stdin`).
///
/// # Errors
///
/// Returns `ServeError` if the graph cannot be loaded.
pub async fn run_server<R, W>(reader: R, writer: W, graph_path: &str) -> Result<(), ServeError>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    use crate::graph::{communities_from_graph, load_graph};

    let mut graph = load_graph(graph_path)?;
    let mut communities = communities_from_graph(&graph);

    let meta = std::fs::metadata(graph_path);
    let (init_mtime, init_size) = meta.map_or((0, 0), |m| {
        let mtime = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| {
                u64::from(d.subsec_nanos()) + d.as_secs() * 1_000_000_000
            });
        (mtime, m.len())
    });
    let mut reload_state = ReloadState {
        mtime_ns: init_mtime,
        size: init_size,
    };
    let mut idf_cache: HashMap<String, f64> = HashMap::new();

    let buf_reader = BufReader::new(reader);
    let mut lines = buf_reader.lines();
    let mut writer = writer;

    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue, // Ignore malformed lines.
        };
        if let Some(response) = dispatch(
            &msg,
            &mut graph,
            &mut communities,
            &mut reload_state,
            &mut idf_cache,
            graph_path,
        ) {
            let mut out = serde_json::to_string(&response).unwrap_or_default();
            out.push('\n');
            if writer.write_all(out.as_bytes()).await.is_err() {
                break;
            }
        }
    }
    Ok(())
}
