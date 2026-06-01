//! Circular import detection at the file level.
//!
//! Ports `find_import_cycles` from `graphify-py/graphify/analyze.py` (#961).
//! Symbol-level nodes are collapsed to their parent file via the `source_file`
//! attribute, a directed file-level graph is built from `imports_from` /
//! `re_exports` edges, and simple cycles bounded by length are reported.

use graphify_build::Graph;
use indexmap::{IndexMap, IndexSet};
use serde_json::Value;

/// Default cap on the number of files a reported cycle may span.
const DEFAULT_MAX_CYCLE_LENGTH: usize = 5;
/// Default cap on the number of cycles returned (shortest first).
const DEFAULT_TOP_N: usize = 20;

/// A circular import dependency at the file level.
///
/// Mirrors the Python record shape `{"cycle": [...], "length": n, "why": ...}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportCycle {
    /// Files forming the cycle, in dependency order and starting from the
    /// lexicographically smallest file so rotations dedupe to one record.
    pub cycle: Vec<String>,
    /// Number of files in the cycle (`cycle.len()`).
    pub length: usize,
    /// Constant rationale string, kept for parity with the Python record.
    pub why: &'static str,
}

/// Detect circular import dependencies at the file level using the default
/// bounds (`max_cycle_length = 5`, `top_n = 20`).
///
/// Mirrors Python `find_import_cycles(G)`.
#[must_use]
pub fn find_import_cycles(graph: &Graph) -> Vec<ImportCycle> {
    find_import_cycles_bounded(graph, DEFAULT_MAX_CYCLE_LENGTH, DEFAULT_TOP_N)
}

/// Detect circular import dependencies, reporting at most `top_n` cycles each
/// spanning at most `max_cycle_length` files (shortest first).
///
/// Collapses symbol-level nodes to their parent file via `source_file`, orients
/// each `imports_from` / `re_exports` edge using the edge's own `source_file`,
/// finds simple cycles, and deduplicates rotations by normalising every cycle to
/// start from its lexicographically smallest file.
#[must_use]
pub fn find_import_cycles_bounded(
    graph: &Graph,
    max_cycle_length: usize,
    top_n: usize,
) -> Vec<ImportCycle> {
    // Every cycle spans at least one file, so a zero bound admits none. Return
    // early — mirrors Python's `len(cycle) <= max_cycle_length` rejecting every
    // cycle at max=0, and stops a self-loop (length 1) from leaking past the
    // documented bound.
    if max_cycle_length == 0 {
        return Vec::new();
    }

    // Zero results requested: short-circuit before building the file graph. The
    // `top_n * 10` cap would already yield nothing, but skipping the graph walk
    // avoids wasted work.
    if top_n == 0 {
        return Vec::new();
    }

    let adj = build_file_graph(graph);
    if adj.values().all(IndexSet::is_empty) {
        return Vec::new();
    }

    // Enumerate elementary cycles directly in normalised (smallest-file-first)
    // form: a cycle is emitted only from its lexicographically smallest file,
    // visiting strictly larger files in between, which yields each cycle exactly
    // once. The `top_n * 10` cap guards against combinatorial explosion.
    let cap = top_n.saturating_mul(10);
    let mut cycles = enumerate_cycles(&adj, max_cycle_length, cap);

    // Shortest first (tightest coupling); lexicographic tie-break keeps the
    // ordering deterministic regardless of graph iteration order.
    cycles.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    cycles.dedup();
    cycles.truncate(top_n);

    cycles
        .into_iter()
        .map(|cycle| ImportCycle {
            length: cycle.len(),
            cycle,
            why: "circular dependency",
        })
        .collect()
}

/// Resolve a node's owning file via its `source_file` attribute, or `""`.
fn endpoint_source_file<'a>(graph: &'a Graph, node_id: &str) -> &'a str {
    graph
        .node_data(node_id)
        .and_then(|attrs| attrs.get("source_file"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// Build the directed file-level adjacency from import/re-export edges.
///
/// Endpoints are resolved using `source_file` only — never inferred from a
/// label or id — so external nodes (no `source_file`) drop out cleanly.
fn build_file_graph(graph: &Graph) -> IndexMap<String, IndexSet<String>> {
    let mut adj: IndexMap<String, IndexSet<String>> = IndexMap::new();

    for edge in graph.edges() {
        let relation = edge
            .attrs
            .get("relation")
            .and_then(Value::as_str)
            .unwrap_or("");
        if relation != "imports_from" && relation != "re_exports" {
            continue;
        }
        let src_file = edge
            .attrs
            .get("source_file")
            .and_then(Value::as_str)
            .unwrap_or("");
        if src_file.is_empty() {
            continue;
        }

        let u_file = endpoint_source_file(graph, &edge.source);
        let v_file = endpoint_source_file(graph, &edge.target);

        // Orient the edge from its `source_file` endpoint to the opposite one.
        // Works for both directed and undirected inputs.
        let tgt_file = if u_file == src_file {
            v_file
        } else if v_file == src_file {
            u_file
        // Fallback: neither endpoint's source_file equals the edge's (e.g. path
        // normalisation mismatch or inconsistent data). Prefer the non-empty
        // endpoint (already known to differ from `src_file`, since the prior arm
        // ruled out `v_file == src_file`), else fall back to `u_file`, so
        // orientation still resolves to a valid target file.
        } else if !v_file.is_empty() {
            v_file
        } else {
            u_file
        };
        if tgt_file.is_empty() {
            continue;
        }

        adj.entry(src_file.to_string())
            .or_default()
            .insert(tgt_file.to_string());
        // Ensure the target file is a known node even with no out-edges.
        adj.entry(tgt_file.to_string()).or_default();
    }

    adj
}

/// Enumerate elementary cycles, each already normalised to start from its
/// lexicographically smallest file. Stops once `cap` cycles are collected.
fn enumerate_cycles(
    adj: &IndexMap<String, IndexSet<String>>,
    max_len: usize,
    cap: usize,
) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    // Visit start files in lexicographic order so that, when `cap` truncates a
    // combinatorially large set, the cycles collected are a deterministic
    // function of graph *content* rather than edge-insertion order.
    let mut starts: Vec<&String> = adj.keys().collect();
    starts.sort();
    for start in starts {
        if out.len() >= cap {
            break;
        }
        let mut path: Vec<&str> = vec![start.as_str()];
        let mut on_path: IndexSet<&str> = IndexSet::new();
        on_path.insert(start.as_str());
        dfs_cycles(
            adj,
            start,
            start,
            max_len,
            &mut path,
            &mut on_path,
            &mut out,
            cap,
        );
    }
    out
}

/// Depth-first walk emitting every simple cycle that closes back at `start`
/// while visiting only files strictly greater than `start` (so `start` is the
/// unique minimum and the emitted path is the normalised cycle).
#[allow(clippy::too_many_arguments)] // a focused recursive walker; an options
// struct would just scatter the loop state across allocations.
fn dfs_cycles<'a>(
    adj: &'a IndexMap<String, IndexSet<String>>,
    start: &'a str,
    current: &'a str,
    max_len: usize,
    path: &mut Vec<&'a str>,
    on_path: &mut IndexSet<&'a str>,
    out: &mut Vec<Vec<String>>,
    cap: usize,
) {
    if out.len() >= cap {
        return;
    }
    let Some(neighbors) = adj.get(current) else {
        return;
    };
    // Sorted neighbour traversal keeps cap-truncation deterministic by content
    // (see `enumerate_cycles`). Neighbour sets are small (file-level fan-out),
    // so sorting per visit is cheap.
    let mut nbrs: Vec<&str> = neighbors.iter().map(String::as_str).collect();
    nbrs.sort_unstable();
    for next in nbrs {
        if next == start {
            // Closing edge → `path` is a complete normalised cycle.
            out.push(path.iter().map(|s| (*s).to_string()).collect());
            if out.len() >= cap {
                return;
            }
        } else if next > start && path.len() < max_len && !on_path.contains(next) {
            path.push(next);
            on_path.insert(next);
            dfs_cycles(adj, start, next, max_len, path, on_path, out, cap);
            on_path.shift_remove(next);
            path.pop();
        }
    }
}
