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

use graphify_prs::gh::ProcessGhClient;
use graphify_prs::git::ProcessGitClient;

use crate::tools::{
    load_community_labels, maybe_reload, resource_audit, resource_questions, resource_surprises,
    tool_get_community, tool_get_neighbors, tool_get_node, tool_get_pr_impact, tool_god_nodes,
    tool_graph_stats, tool_list_prs, tool_query_graph, tool_shortest_path, tool_triage_prs,
};
use crate::{ReloadState, ServeError};

// ── Tool schema ───────────────────────────────────────────────────────────────

/// Returns the static list of MCP tool descriptors broadcast on the `tools/list` request.
fn tools_list() -> Vec<Value> {
    let mut tools = vec![
        tool_query_graph_schema(),
        tool_get_node_schema(),
        tool_get_neighbors_schema(),
        tool_get_community_schema(),
        tool_god_nodes_schema(),
        tool_graph_stats_schema(),
        tool_shortest_path_schema(),
        tool_list_prs_schema(),
        tool_get_pr_impact_schema(),
        tool_triage_prs_schema(),
    ];
    // Every tool accepts an optional `project_path` for multi-project serving:
    // routing a call to a different project's graph. Injected once here rather
    // than in each schema, mirroring graphify-py serve._build_server (#1594).
    for tool in &mut tools {
        if let Some(props) = tool
            .get_mut("inputSchema")
            .and_then(|s| s.get_mut("properties"))
            .and_then(Value::as_object_mut)
        {
            props.insert(
                "project_path".to_string(),
                json!({
                    "type": "string",
                    "description": "Absolute path to a project root; routes the call to that project's graphify-out/graph.json instead of the default graph."
                }),
            );
        }
    }
    tools
}

fn tool_query_graph_schema() -> Value {
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
    })
}

fn tool_get_node_schema() -> Value {
    json!({
        "name": "get_node",
        "description": "Get full details for a specific node by label or ID.",
        "inputSchema": {
            "type": "object",
            "properties": {"label": {"type": "string", "description": "Node label or ID to look up"}},
            "required": ["label"]
        }
    })
}

fn tool_get_neighbors_schema() -> Value {
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
    })
}

fn tool_get_community_schema() -> Value {
    json!({
        "name": "get_community",
        "description": "Get all nodes in a community by community ID.",
        "inputSchema": {
            "type": "object",
            "properties": {"community_id": {"type": "integer", "description": "Community ID (0-indexed by size)"}},
            "required": ["community_id"]
        }
    })
}

fn tool_god_nodes_schema() -> Value {
    json!({
        "name": "god_nodes",
        "description": "Return the most connected nodes - the core abstractions of the knowledge graph.",
        "inputSchema": {"type": "object", "properties": {"top_n": {"type": "integer", "default": 10}}}
    })
}

fn tool_graph_stats_schema() -> Value {
    json!({
        "name": "graph_stats",
        "description": "Return summary statistics: node count, edge count, communities, confidence breakdown.",
        "inputSchema": {"type": "object", "properties": {}}
    })
}

fn tool_shortest_path_schema() -> Value {
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
    })
}

fn tool_list_prs_schema() -> Value {
    json!({
        "name": "list_prs",
        "description": "List open GitHub PRs with CI status, review state, and graph impact \
    (which communities each PR touches, blast radius). Use this before starting \
    work to check if a PR already covers the area you're about to change.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "base": {"type": "string", "description": "Base branch to filter PRs by (auto-detected if omitted)"},
                "repo": {"type": "string", "description": "GitHub repo (owner/repo). Defaults to current repo."},
                "limit": {"type": "integer", "description": "Maximum number of PRs to return (default 50)"}
            }
        }
    })
}

fn tool_get_pr_impact_schema() -> Value {
    json!({
        "name": "get_pr_impact",
        "description": "Get detailed graph impact for a specific PR: which files it changes, \
    which knowledge-graph communities are affected, and how many nodes are touched. \
    Use this to assess merge risk or check for overlap with your current work.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "pr_number": {"type": "integer", "description": "PR number to analyse"},
                "repo": {"type": "string", "description": "GitHub repo (owner/repo). Defaults to current repo."}
            },
            "required": ["pr_number"]
        }
    })
}

fn tool_triage_prs_schema() -> Value {
    json!({
        "name": "triage_prs",
        "description": "Return all actionable open PRs (correct base, not stale) with full graph impact data \
    so you can reason about review priority, merge order, and conflict risk. \
    Call this when the user asks 'what PRs should I review?' or 'what\\'s ready to merge?'",
        "inputSchema": {
            "type": "object",
            "properties": {
                "base": {"type": "string", "description": "Base branch to filter PRs by (auto-detected if omitted)"},
                "repo": {"type": "string", "description": "GitHub repo (owner/repo). Defaults to current repo."}
            }
        }
    })
}

/// Static list of MCP resource descriptors returned on a `resources/list` request.
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
/// Build a JSON-RPC 2.0 success response envelope.
#[allow(clippy::needless_pass_by_value)]
fn ok_response(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// Build a JSON-RPC 2.0 error response envelope with the given numeric error code.
fn error_response(id: &Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message}
    })
}

// ── Message dispatcher ────────────────────────────────────────────────────────

/// Dispatch a `tools/call` MCP request to the matching tool handler.
///
/// Returns the tool's text output, which is wrapped in a `content` array
/// by the caller before being sent back as a JSON-RPC response.
fn dispatch_tool(
    name: &str,
    graph: &mut Graph,
    communities: &IndexMap<i64, Vec<String>>,
    arguments: &serde_json::Map<String, Value>,
    idf_cache: &mut HashMap<String, f64>,
    graph_path: &str,
) -> String {
    match name {
        "query_graph" => {
            // Log the MCP query (kind="mcp_query") with timing + mode/depth/budget,
            // mirroring graphify-py's serve._tool_query_graph (#1128).
            let t0 = std::time::Instant::now();
            let result = tool_query_graph(graph, arguments, idf_cache);
            log_mcp_query(arguments, graph_path, &result, t0.elapsed());
            result
        }
        "get_node" => tool_get_node(graph, arguments),
        "get_neighbors" => tool_get_neighbors(graph, arguments),
        "get_community" => tool_get_community(graph, communities, arguments),
        "god_nodes" => tool_god_nodes(graph, arguments),
        "graph_stats" => tool_graph_stats(graph, communities),
        "shortest_path" => tool_shortest_path(graph, arguments, idf_cache),
        "list_prs" => {
            let args = Value::Object(arguments.clone());
            match tool_list_prs(&args, &ProcessGhClient, &ProcessGitClient) {
                Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_default(),
                Err(e) => format!("Error: {e}"),
            }
        }
        "get_pr_impact" => {
            let args = Value::Object(arguments.clone());
            match tool_get_pr_impact(graph, &args, &ProcessGhClient) {
                Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_default(),
                Err(e) => format!("Error: {e}"),
            }
        }
        "triage_prs" => {
            let args = Value::Object(arguments.clone());
            match tool_triage_prs(&args, &ProcessGhClient, &ProcessGitClient) {
                Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_default(),
                Err(e) => format!("Error: {e}"),
            }
        }
        other => format!("Unknown tool: {other}"),
    }
}

/// Append a `kind="mcp_query"` record to the query log (fail-silent), with the
/// same `mode` / `depth` / `token_budget` defaults [`tool_query_graph`] applies.
fn log_mcp_query(
    arguments: &serde_json::Map<String, Value>,
    graph_path: &str,
    result: &str,
    elapsed: std::time::Duration,
) {
    let Some(question) = arguments.get("question").and_then(Value::as_str) else {
        return;
    };
    let mode = arguments
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("bfs");
    let depth = arguments
        .get("depth")
        .and_then(Value::as_u64)
        .map_or(3, |d| d.min(6));
    let budget = arguments
        .get("token_budget")
        .and_then(Value::as_u64)
        .unwrap_or(2000);
    let mut extra = IndexMap::new();
    extra.insert("mode".to_string(), json!(mode));
    extra.insert("depth".to_string(), json!(depth));
    extra.insert("token_budget".to_string(), json!(budget));
    crate::querylog::log_query(&crate::querylog::QueryLog {
        kind: "mcp_query",
        question,
        corpus: graph_path,
        result: Some(result),
        duration_ms: Some(elapsed.as_secs_f64() * 1000.0),
        extra,
        ..crate::querylog::QueryLog::default()
    });
}

/// Dispatch a `resources/read` MCP request by URI to the matching resource reader.
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
    let mut state = McpServerState::load(graph_path);

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
        if let Some(response) = state.handle(&msg, graph_path) {
            let mut out = serde_json::to_string(&response).unwrap_or_default();
            out.push('\n');
            if writer.write_all(out.as_bytes()).await.is_err() {
                break;
            }
        }
    }
    Ok(())
}

/// A single loaded graph plus its derived state, cached per resolved graph path.
struct GraphCtx {
    graph: Graph,
    communities: IndexMap<i64, Vec<String>>,
    reload_state: ReloadState,
    idf_cache: HashMap<String, f64>,
}

impl GraphCtx {
    /// Load the graph at `graph_path` and derive its state.
    fn load(graph_path: &str) -> Result<Self, ServeError> {
        use crate::graph::{communities_from_graph, load_graph};
        let graph = load_graph(graph_path)?;
        let communities = communities_from_graph(&graph);
        let (mtime_ns, size) = stat_mtime_size(graph_path);
        Ok(Self {
            graph,
            communities,
            reload_state: ReloadState { mtime_ns, size },
            idf_cache: HashMap::new(),
        })
    }

    /// Reload from disk when the file's `(mtime, size)` changed.
    fn reload(&mut self, graph_path: &str) {
        // A successful reload swaps in a new graph, so `idf_cache` — keyed only
        // by term but derived from node count and document frequency — is stale
        // and must be dropped. graphify-py stores `_idf_cache` ON the graph so a
        // swap invalidates it for free; the Rust holds it separately, so clear it
        // explicitly so a hot reload cannot rank queries with stale IDF weights.
        if maybe_reload(
            graph_path,
            &mut self.graph,
            &mut self.communities,
            &mut self.reload_state,
        ) {
            self.idf_cache.clear();
        }
    }
}

/// Read a graph file's `(mtime_ns, size)` for hot-reload bookkeeping; a missing
/// or unreadable file reports `(0, 0)`.
fn stat_mtime_size(graph_path: &str) -> (u64, u64) {
    std::fs::metadata(graph_path).map_or((0, 0), |m| {
        let mtime = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| {
                // Checked arithmetic so a post-2554 mtime saturates, not wraps.
                d.as_secs()
                    .checked_mul(1_000_000_000)
                    .and_then(|s| s.checked_add(u64::from(d.subsec_nanos())))
                    .unwrap_or(u64::MAX)
            });
        (mtime, m.len())
    })
}

/// In-memory MCP server state shared across messages.
///
/// Holds the default graph (the one the server was started with, or `None` when
/// it is absent — pure multi-project mode) plus a per-project-path cache of
/// graphs routed to via a tool call's optional `project_path`. Each context
/// hot-reloads independently. Both the stdio transport ([`run_server`]) and the
/// Streamable HTTP transport drive it through [`McpServerState::handle`], so the
/// two share one dispatch path and one reload contract.
pub(crate) struct McpServerState {
    default_path: String,
    default_ctx: Option<GraphCtx>,
    /// Per-project graph cache keyed by the resolved `graph.json` path. Keys are
    /// NOT canonicalised and the map is unbounded, mirroring graphify-py's
    /// `_ctx_cache` (a plain dict on the same resolved string): two aliases to one
    /// graph merely load twice (benign), and a server sees a bounded set of real
    /// project roots. Canonicalising/evicting is declined to keep routing parity.
    project_ctxs: HashMap<String, GraphCtx>,
}

impl McpServerState {
    /// Load the default graph at `graph_path`, tolerating its absence.
    ///
    /// A missing or unloadable default graph yields a pure multi-project server
    /// (`default_ctx == None`) rather than a startup failure, so `project_path`
    /// calls still resolve (#1594).
    #[must_use]
    pub(crate) fn load(graph_path: &str) -> Self {
        Self {
            default_path: graph_path.to_string(),
            default_ctx: GraphCtx::load(graph_path).ok(),
            project_ctxs: HashMap::new(),
        }
    }

    /// Resolve a tool call's optional `project_path` to a graph.json path.
    /// `None` → the default graph; else `<project_path>/<GRAPHIFY_OUT>/graph.json`.
    fn resolve_graph_path(&self, project_path: Option<&str>) -> String {
        match project_path {
            None => self.default_path.clone(),
            Some(p) => Path::new(p)
                .join(graphify_security::graphify_out())
                .join("graph.json")
                .to_string_lossy()
                .into_owned(),
        }
    }

    /// Get (loading + hot-reloading) the context for `resolved`, or an error
    /// string when the graph cannot be loaded (a tool error, never a crash).
    ///
    /// Loads run under the caller's `&mut self` borrow (and, in the HTTP
    /// transport, its state `Mutex`). graphify-py uses a lock-free hot path and
    /// locks only to build; the Rust keeps one borrow/lock per message — simpler
    /// and adequate for the MCP server's low request rate (HTTP is an off-by-
    /// default feature). A lock-free fast path fights the `&mut GraphCtx` borrow
    /// the tool dispatch requires, for no real-world throughput gain here.
    fn select_ctx(&mut self, resolved: &str) -> Result<&mut GraphCtx, String> {
        if resolved == self.default_path {
            if self.default_ctx.is_none() {
                self.default_ctx = Some(GraphCtx::load(resolved).map_err(|e| e.to_string())?);
            }
            let Some(ctx) = &mut self.default_ctx else {
                return Err("default graph unavailable".to_string());
            };
            ctx.reload(resolved);
            Ok(ctx)
        } else {
            use std::collections::hash_map::Entry;
            let ctx = match self.project_ctxs.entry(resolved.to_string()) {
                Entry::Occupied(e) => e.into_mut(),
                Entry::Vacant(e) => {
                    e.insert(GraphCtx::load(resolved).map_err(|err| err.to_string())?)
                }
            };
            ctx.reload(resolved);
            Ok(ctx)
        }
    }

    /// Handle a `tools/call`: pop `project_path`, select the target graph, and
    /// dispatch, returning the tool's text output (or an error string when the
    /// project's graph cannot be loaded — a tool error, never a crash).
    ///
    /// Errors are returned as ordinary text content, not with an `isError: true`
    /// flag, mirroring graphify-py's handler (it yields `TextContent(text="Error
    /// executing …")` and never sets `isError`). Matching that keeps the MCP wire
    /// output identical to the reference; adding a structured error status is a
    /// deliberate non-change for parity.
    fn call_tool(&mut self, params: &serde_json::Map<String, Value>) -> String {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let mut arguments = params
            .get("arguments")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        // Match graphify-py: absent / null / empty → the default graph; a
        // non-empty string routes to that project; any other JSON type is a
        // caller error (Python's `Path(project_path)` raises on a non-str, which
        // surfaces as a tool error) rather than silently falling back to default.
        let project_path = match arguments.remove("project_path") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s.is_empty() => None,
            Some(Value::String(s)) => Some(s),
            Some(other) => {
                return format!("Error: project_path must be a string, got: {other}");
            }
        };
        let resolved = self.resolve_graph_path(project_path.as_deref());
        match self.select_ctx(&resolved) {
            Ok(ctx) => dispatch_tool(
                &name,
                &mut ctx.graph,
                &ctx.communities,
                &arguments,
                &mut ctx.idf_cache,
                &resolved,
            ),
            Err(e) => format!(
                "Error: could not load graph for project_path '{}': {e}",
                project_path.as_deref().unwrap_or("")
            ),
        }
    }

    /// Handle a `resources/read` against the default graph, returning the
    /// resource text (or an error string when the default graph is absent).
    fn read_resource(&mut self, uri: &str) -> String {
        let resolved = self.resolve_graph_path(None);
        match self.select_ctx(&resolved) {
            Ok(ctx) => dispatch_resource(uri, &ctx.graph, &ctx.communities, &resolved),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Route one JSON-RPC message. Returns `Some(response)` for requests and
    /// `None` for notifications. `_graph_path` is retained for transport
    /// signature stability; the default path is owned by the state.
    pub(crate) fn handle(&mut self, msg: &Value, _graph_path: &str) -> Option<Value> {
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let empty_obj = serde_json::Map::new();
        let params = msg
            .get("params")
            .and_then(Value::as_object)
            .unwrap_or(&empty_obj);
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
                        "capabilities": {"tools": {}, "resources": {}},
                        "serverInfo": {"name": "graphify", "version": env!("CARGO_PKG_VERSION")}
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
                let text = self.call_tool(params);
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
                let uri = params
                    .get("uri")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let text = self.read_resource(&uri);
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
}
