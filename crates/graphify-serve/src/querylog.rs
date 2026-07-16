//! Query logging — opt-in, append-only JSONL, fail-silent (#1128, #1797).
//!
//! Ports `graphify-py/graphify/querylog.py`. Logging is OFF by default (#1797):
//! a default-on plaintext record of proprietary queries would contradict
//! graphify's on-device, no-telemetry posture. When enabled, each query/path/
//! explain (and MCP tool call) appends one JSON record to the log file. Logging
//! never raises: any failure — disabled, unwritable path, serialization error —
//! is swallowed so it can't break a query.
//!
//! Env knobs (logging is off unless one of the enable vars is set):
//! - `GRAPHIFY_QUERY_LOG` — enable and write to this path (`~` expanded).
//! - `GRAPHIFY_QUERY_LOG_ENABLE` — `1`/`true`/`yes` enables at the default path
//!   `~/.cache/graphify-queries.log`.
//! - `GRAPHIFY_QUERY_LOG_DISABLE` — `1`/`true`/`yes` forces logging off (wins).
//! - `GRAPHIFY_QUERY_LOG_RESPONSES` — `1`/`true`/`yes` also records the full
//!   result text under `response` (when logging is enabled).

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Map, Value, json};

/// Matches the `N nodes found` / `1 node found` header emitted by query results.
#[allow(clippy::expect_used)] // literal pattern; build cannot fail
static NODES_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d+)\s+nodes?\s+found").expect("static nodes-found regex"));

/// One query-log record. `extra` carries arbitrary per-kind fields (e.g. `mode`,
/// `depth`) mirroring Python's `**extra` kwargs; `null` extras are dropped.
#[derive(Debug, Default)]
pub struct QueryLog<'a> {
    /// Query kind, e.g. `"query"`, `"path"`, `"explain"`, `"mcp_query"`.
    pub kind: &'a str,
    /// The user's question / query text.
    pub question: &'a str,
    /// The corpus the query ran against (graph path).
    pub corpus: &'a str,
    /// Rendered result text; used to infer `nodes_returned` and `result_chars`.
    pub result: Option<&'a str>,
    /// Explicit node count; takes precedence over inference from `result`.
    pub nodes_returned: Option<i64>,
    /// Wall-clock duration in milliseconds (rounded to 3 dp when recorded).
    pub duration_ms: Option<f64>,
    /// Extra per-kind fields; `null` values are omitted.
    pub extra: IndexMap<String, Value>,
}

/// `true` for `1`/`true`/`yes` (case-insensitive, trimmed).
fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes"
    )
}

/// User home from `$HOME` (matches the rest of the workspace's resolution).
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Expand a leading `~` / `~/` to the home directory (mirrors `Path.expanduser`).
fn expanduser(path: &str) -> PathBuf {
    if path == "~" {
        return home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}

/// Resolve the log path (opt-in, #1797). Logging is OFF by default: it is
/// enabled only by `GRAPHIFY_QUERY_LOG=<path>` (that path) or
/// `GRAPHIFY_QUERY_LOG_ENABLE` truthy (the default `~/.cache/graphify-queries.log`).
/// `GRAPHIFY_QUERY_LOG_DISABLE` truthy forces it off (wins, back-compat). A
/// default-on plaintext record of proprietary queries would contradict
/// graphify's on-device, no-telemetry posture. Returns `None` when off.
fn log_path() -> Option<PathBuf> {
    if std::env::var("GRAPHIFY_QUERY_LOG_DISABLE").is_ok_and(|v| truthy(&v)) {
        return None;
    }
    let override_path = std::env::var("GRAPHIFY_QUERY_LOG").unwrap_or_default();
    let override_path = override_path.trim();
    if !override_path.is_empty() {
        return Some(expanduser(override_path));
    }
    if std::env::var("GRAPHIFY_QUERY_LOG_ENABLE").is_ok_and(|v| truthy(&v)) {
        return home_dir().map(|h| h.join(".cache").join("graphify-queries.log"));
    }
    None
}

/// Whether to record the full response text (`GRAPHIFY_QUERY_LOG_RESPONSES`).
fn log_responses() -> bool {
    std::env::var("GRAPHIFY_QUERY_LOG_RESPONSES").is_ok_and(|v| truthy(&v))
}

/// Parse the `N nodes found` count out of a rendered query result.
#[must_use]
pub fn nodes_from_result(result: &str) -> Option<i64> {
    NODES_RE
        .captures(result)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<i64>().ok())
}

/// Append one JSONL record to the query log. Never panics — every failure path
/// (disabled, unwritable, serialization error) is swallowed.
pub fn log_query(rec: &QueryLog<'_>) {
    // Errors are intentionally ignored: logging must never break a query.
    let _ = try_log_query(rec);
}

/// Build the record and append it; returns the error so [`log_query`] can drop it.
fn try_log_query(rec: &QueryLog<'_>) -> std::io::Result<()> {
    let Some(path) = log_path() else {
        return Ok(());
    };
    let nodes_returned = rec
        .nodes_returned
        .or_else(|| rec.result.and_then(nodes_from_result));

    let mut map = Map::new();
    map.insert("ts".to_string(), json!(chrono::Utc::now().to_rfc3339()));
    map.insert("kind".to_string(), json!(rec.kind));
    map.insert("question".to_string(), json!(rec.question));
    map.insert("corpus".to_string(), json!(rec.corpus));
    map.insert("nodes_returned".to_string(), json!(nodes_returned));
    if let Some(result) = rec.result {
        map.insert("result_chars".to_string(), json!(result.chars().count()));
    }
    if let Some(duration) = rec.duration_ms {
        // round(x, 3) — three decimal places, matching the Python record.
        map.insert(
            "duration_ms".to_string(),
            json!((duration * 1000.0).round() / 1000.0),
        );
    }
    for (key, value) in &rec.extra {
        if !value.is_null() {
            map.insert(key.clone(), value.clone());
        }
    }
    if let Some(result) = rec.result
        && log_responses()
    {
        map.insert("response".to_string(), json!(result));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_string(&Value::Object(map)).map_err(std::io::Error::other)?;
    // Append the newline to the payload so the record is written with a single
    // `write_all`. With two writes, concurrent appenders could interleave a
    // record and its terminating newline and corrupt the JSONL framing.
    line.push('\n');
    let mut fh = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    fh.write_all(line.as_bytes())?;
    Ok(())
}
