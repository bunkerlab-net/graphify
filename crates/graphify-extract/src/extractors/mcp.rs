//! MCP (Model Context Protocol) server-config extractor.
//!
//! Reads `.mcp.json` / `claude_desktop_config.json` / `mcp.json` /
//! `mcp_servers.json` and turns the `mcpServers` map into Graphify nodes and
//! edges. Mirrors `graphify-py/graphify/mcp_ingest.py`.
//!
//! # Schema emitted
//!
//! Node kinds (carried in `metadata.mcp_kind`):
//! - `mcp_config_file` — the config file itself
//! - `mcp_server` — one per entry under `mcpServers` (stem-scoped ID)
//! - `mcp_command` — executable (`npx`, `uvx`, ...) — global ID
//! - `mcp_package` — npm / pypi package parsed from args — global ID
//! - `env_var` — env variable NAME only — global ID. VALUES ARE NEVER READ.
//!
//! Edge relations: `contains` (file → server), `references` (server → command
//! / package), `requires_env` (server → env var).
//!
//! # Security
//! - Env var VALUES are never read, persisted, or surfaced — only NAMES.
//! - File size capped at 1 MiB (matches `extract_json`).
//! - All labels pass through [`sanitize_label`].
//! - Args are not persisted (avoids leaking paths/secrets embedded as args).

use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;

use graphify_security::sanitize_label;
use indexmap::IndexMap;
use regex::Regex;
use serde_json::Value;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};

/// Recognised MCP config filenames (matched on basename, case-sensitive).
pub static MCP_CONFIG_FILENAMES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        ".mcp.json",
        "claude_desktop_config.json",
        "mcp.json",
        "mcp_servers.json",
    ]
    .into_iter()
    .collect()
});

const MAX_BYTES: u64 = 1_048_576; // 1 MiB — same cap as extract_json
const MAX_SERVERS_PER_FILE: usize = 200; // generous; flags pathological configs

#[allow(clippy::expect_used)] // literal patterns; build cannot fail
static NPM_PKG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^@[a-z0-9][a-z0-9._-]*/[a-z0-9][a-z0-9._-]*(?:@[\w.\-+]+)?$")
        .expect("static npm package regex")
});
#[allow(clippy::expect_used)]
static PY_MCP_PKG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z0-9][a-z0-9._-]*-mcp(?:-[a-z0-9._-]+)?$|^mcp-[a-z0-9][a-z0-9._-]*$")
        .expect("static python mcp package regex")
});
#[allow(clippy::expect_used)]
static ARG_FLAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^-{1,2}\w").expect("static arg flag regex"));

/// Return `true` when `path` is a recognised MCP config filename.
#[must_use]
pub fn is_mcp_config_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| MCP_CONFIG_FILENAMES.contains(name))
}

/// Parse an MCP config file into Graphify nodes and edges.
///
/// Returns a [`FileResult`] with populated `nodes`/`edges` on success, or an
/// error-carrying result (empty nodes/edges) on parse failure, oversize file,
/// or a missing `mcpServers` map.
#[must_use]
pub fn extract_mcp_config(path: &Path) -> FileResult {
    let mut raw = Vec::new();
    match std::fs::File::open(path) {
        Ok(f) => {
            if let Err(e) = f.take(MAX_BYTES + 1).read_to_end(&mut raw) {
                return FileResult::error(format!("mcp_ingest read error: {e}"));
            }
        }
        Err(e) => return FileResult::error(format!("mcp_ingest read error: {e}")),
    }

    if raw.len() as u64 > MAX_BYTES {
        return FileResult::error("mcp config too large to index");
    }

    let text = match std::str::from_utf8(&raw) {
        Ok(t) => t,
        Err(e) => return FileResult::error(format!("mcp_ingest decode error: {e}")),
    };

    let doc: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return FileResult::error(format!("mcp_ingest json error: {e}")),
    };

    let Value::Object(doc) = doc else {
        return FileResult::error("mcp_ingest: root is not an object");
    };

    // Prefer the canonical `mcpServers` map; fall back to one well-known nested
    // shape (`{"mcp": {"servers": {...}}}`) but do not search exhaustively.
    let servers = match doc.get("mcpServers") {
        Some(Value::Object(m)) => m,
        _ => match doc.get("mcp").and_then(|n| n.get("servers")) {
            Some(Value::Object(m)) => m,
            _ => return FileResult::error("mcp_ingest: no mcpServers map"),
        },
    };

    let str_path = path.to_string_lossy().into_owned();
    let file_nid = make_id1(&str_path);
    let filename = path
        .file_name()
        .map_or_else(String::new, |f| f.to_string_lossy().into_owned());

    let mut builder = McpBuilder::new(str_path);
    builder.add_node(&file_nid, &filename, "mcp_config_file");

    let stem = file_stem(path);
    let mut server_count = 0usize;
    for (server_name, spec) in servers {
        if server_name.is_empty() {
            continue;
        }
        let Value::Object(spec) = spec else {
            // Skip non-object server entries silently — the broken entry is
            // the user's, not ours.
            continue;
        };
        if server_count >= MAX_SERVERS_PER_FILE {
            break;
        }
        server_count += 1;
        builder.emit_server(server_name, spec, &file_nid, &stem);
    }

    FileResult {
        nodes: builder.nodes,
        edges: builder.edges,
        raw_calls: Vec::new(),
        error: None,
    }
}

/// Accumulates nodes/edges while de-duplicating by node ID and
/// `(source, target, relation)` edge key.
struct McpBuilder {
    source_file: String,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    seen_node_ids: HashSet<String>,
    seen_edge_keys: HashSet<(String, String, String)>,
}

impl McpBuilder {
    fn new(source_file: String) -> Self {
        Self {
            source_file,
            nodes: Vec::new(),
            edges: Vec::new(),
            seen_node_ids: HashSet::new(),
            seen_edge_keys: HashSet::new(),
        }
    }

    /// Append a node if not already present. `kind` is metadata, not `file_type`.
    fn add_node(&mut self, nid: &str, label: &str, kind: &str) {
        if nid.is_empty() || !self.seen_node_ids.insert(nid.to_string()) {
            return;
        }
        let mut metadata = IndexMap::new();
        metadata.insert("mcp_kind".to_string(), Value::String(kind.to_string()));
        self.nodes.push(Node {
            id: nid.to_string(),
            label: sanitize_label(Some(label)),
            file_type: "code".to_string(),
            source_file: self.source_file.clone(),
            source_location: Some("L1".to_string()),
            metadata: Some(metadata),
        });
    }

    /// Append an edge if `(source, target, relation)` is not already present.
    fn add_edge(&mut self, source: &str, target: &str, relation: &str, context: Option<&str>) {
        if source.is_empty() || target.is_empty() || source == target {
            return;
        }
        let key = (source.to_string(), target.to_string(), relation.to_string());
        if !self.seen_edge_keys.insert(key) {
            return;
        }
        self.edges.push(Edge {
            external: false,
            source: source.to_string(),
            target: target.to_string(),
            relation: relation.to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: self.source_file.clone(),
            source_location: Some("L1".to_string()),
            weight: 1.0,
            context: context.map(str::to_string),
            confidence_score: Some(1.0),
        });
    }

    /// Emit nodes/edges for one entry under `mcpServers`.
    fn emit_server(
        &mut self,
        server_name: &str,
        spec: &serde_json::Map<String, Value>,
        file_nid: &str,
        file_stem: &str,
    ) {
        let server_nid = make_id(&[file_stem, "mcp_server", server_name]);
        self.add_node(&server_nid, server_name, "mcp_server");
        self.add_edge(file_nid, &server_nid, "contains", None);

        if let Some(Value::String(command)) = spec.get("command") {
            let cmd_label = command.trim();
            if !cmd_label.is_empty() {
                let cmd_nid = make_id(&["mcp_command", cmd_label]);
                self.add_node(&cmd_nid, cmd_label, "mcp_command");
                self.add_edge(&server_nid, &cmd_nid, "references", Some("command"));
            }
        }

        if let Some(Value::Array(args)) = spec.get("args")
            && let Some(package) = detect_package_from_args(args)
        {
            let pkg_nid = make_id(&["mcp_package", &package]);
            self.add_node(&pkg_nid, &package, "mcp_package");
            self.add_edge(&server_nid, &pkg_nid, "references", Some("package"));
        }

        if let Some(Value::Object(env)) = spec.get("env") {
            // ONLY KEYS. Values may contain secrets and are never read here.
            for env_name in env.keys() {
                if env_name.is_empty() {
                    continue;
                }
                let env_nid = make_id(&["env_var", env_name]);
                self.add_node(&env_nid, env_name, "env_var");
                self.add_edge(&server_nid, &env_nid, "requires_env", None);
            }
        }
    }
}

/// Return the first arg that looks like an npm or pypi package id, else `None`.
///
/// Skips short flags (`-y`, `--yes`) and option arguments (`--local-timezone=UTC`).
#[must_use]
fn detect_package_from_args(args: &[Value]) -> Option<String> {
    for raw in args {
        let Value::String(raw) = raw else {
            continue;
        };
        let arg = raw.trim();
        if arg.is_empty() || ARG_FLAG_RE.is_match(arg) {
            continue;
        }
        if NPM_PKG_RE.is_match(arg) {
            return Some(strip_version(arg));
        }
        if PY_MCP_PKG_RE.is_match(arg) {
            return Some(arg.to_string());
        }
    }
    None
}

/// Drop the `@version` suffix from an npm package id, preserving the scope.
///
/// Scoped (`@scope/name@1.2.3`) has at most two `@`; the second is the version
/// separator. Unscoped (`name@1.2.3`) splits on the first `@`.
#[must_use]
fn strip_version(pkg: &str) -> String {
    if let Some(rest) = pkg.strip_prefix('@') {
        // `rest` drops the leading scope `@`, so an `@` found at index `rel`
        // in `rest` sits at index `rel + 1` in `pkg`. Slicing `pkg[..=rel]`
        // (inclusive of `rel`) therefore keeps `@scope/name` and drops the
        // `@version` suffix.
        match rest.find('@') {
            Some(rel) => pkg[..=rel].to_string(),
            None => pkg.to_string(),
        }
    } else {
        match pkg.find('@') {
            Some(i) => pkg[..i].to_string(),
            None => pkg.to_string(),
        }
    }
}
