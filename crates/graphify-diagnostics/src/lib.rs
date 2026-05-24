//! Read-only diagnostics for `MultiDiGraph` readiness.
//!
//! Ports `graphify-py/graphify/diagnostics.py`. Used by the
//! `graphify diagnose multigraph` CLI to report how many edges in a graph
//! (or raw extraction) would be silently collapsed by the simple-graph
//! builder, without mutating the input.

use std::path::Path;
use std::sync::LazyLock;

use indexmap::{IndexMap, IndexSet};
use regex::Regex;
use serde_json::{Map, Value, json};
use thiserror::Error;

/// Errors raised by the diagnostics pipeline.
#[derive(Debug, Error)]
pub enum DiagnosticsError {
    /// Failed to read the graph file.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Failed to parse the graph JSON.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// Graph file exceeded the memory-bomb size cap.
    #[error(transparent)]
    Security(#[from] graphify_security::SecurityError),

    /// Top-level diagnostic input was not a JSON object.
    #[error("diagnostic input must be a JSON object")]
    NotAnObject,
}

#[allow(clippy::expect_used)] // literal pattern; cannot fail at runtime
static SUPPRESSION_DECL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?P<name>seen_[A-Za-z0-9_]+)\s*[:=]").expect("static suppression-decl regex")
});
#[allow(clippy::expect_used)] // literal pattern; cannot fail at runtime
static TYPE_TUPLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"set\[tuple\[(?P<inside>[^\]]+)\]\]").expect("static type-tuple regex")
});

/// Normalise a single edge dict into the seven canonical-string fields used
/// by every downstream counter / variant group.
#[must_use]
fn canonical_edge(edge: &Value) -> IndexMap<String, String> {
    let mut out: IndexMap<String, String> = IndexMap::with_capacity(8);
    let Value::Object(map) = edge else {
        out.insert("source".to_string(), String::new());
        out.insert("target".to_string(), String::new());
        out.insert("relation".to_string(), String::new());
        out.insert("confidence".to_string(), String::new());
        out.insert("source_file".to_string(), String::new());
        out.insert("source_location".to_string(), String::new());
        out.insert("context".to_string(), String::new());
        out.insert("_invalid".to_string(), "non_object_edge".to_string());
        return out;
    };
    let source = map.get("source").or_else(|| map.get("from"));
    let target = map.get("target").or_else(|| map.get("to"));
    out.insert("source".to_string(), safe_text(source));
    out.insert("target".to_string(), safe_text(target));
    out.insert("relation".to_string(), safe_text(map.get("relation")));
    out.insert("confidence".to_string(), safe_text(map.get("confidence")));
    out.insert("source_file".to_string(), safe_text(map.get("source_file")));
    out.insert(
        "source_location".to_string(),
        safe_text(map.get("source_location")),
    );
    out.insert("context".to_string(), safe_text(map.get("context")));
    out.insert("_invalid".to_string(), String::new());
    out
}

fn safe_text(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        // `serde_json::to_string` on an arbitrary `Value` can fail only on
        // OOM or NaN/Infinity inside a Number (which `serde_json` rejects as
        // a JSON-spec violation). Surface those rather than silently
        // returning an empty string so the diagnostic output can still be
        // traced back to a malformed input.
        other => serde_json::to_string(other).unwrap_or_else(|e| {
            eprintln!("[graphify] diagnostics: failed to render edge field as JSON: {e}");
            String::new()
        }),
    }
}

fn edge_list(extraction: &Map<String, Value>) -> Vec<&Value> {
    let edges = extraction.get("edges").or_else(|| extraction.get("links"));
    edges
        .and_then(Value::as_array)
        .map(|arr| arr.iter().collect())
        .unwrap_or_default()
}

fn node_ids(extraction: &Map<String, Value>) -> IndexSet<String> {
    let Some(nodes) = extraction.get("nodes").and_then(Value::as_array) else {
        return IndexSet::new();
    };
    nodes
        .iter()
        .filter_map(|n| n.as_object())
        .filter_map(|m| m.get("id"))
        .filter(|v| !v.is_null())
        .map(|v| safe_text(Some(v)))
        .collect()
}

fn exact_signature(edge: &Value) -> String {
    let Value::Object(orig) = edge else {
        return "<non-object>".to_string();
    };
    let mut normalised = orig.clone();
    if !normalised.contains_key("source")
        && let Some(v) = normalised.remove("from")
    {
        normalised.insert("source".to_string(), v);
    } else {
        normalised.remove("from");
    }
    if !normalised.contains_key("target")
        && let Some(v) = normalised.remove("to")
    {
        normalised.insert("target".to_string(), v);
    } else {
        normalised.remove("to");
    }
    // Sort keys to produce a canonical signature.
    let mut sorted: Vec<(String, Value)> = normalised.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let sorted_map: Map<String, Value> = sorted.into_iter().collect();
    // Same caveat as `safe_text`: `serde_json::to_string` here only fails on
    // OOM or a NaN/Infinity number in the input. Surface the failure to
    // stderr so a malformed edge can be traced rather than silently
    // collapsing every malformed signature to the empty string (which would
    // make every such edge dedup against the others).
    serde_json::to_string(&Value::Object(sorted_map)).unwrap_or_else(|e| {
        eprintln!("[graphify] diagnostics: failed to render canonical edge signature: {e}");
        String::new()
    })
}

fn count_extra(counter: &IndexMap<String, usize>) -> usize {
    counter.values().filter(|&&c| c > 1).map(|c| c - 1).sum()
}

fn variant_group_count(
    grouped: &IndexMap<(String, String), Vec<IndexMap<String, String>>>,
    field: &str,
    relation_sensitive: bool,
) -> usize {
    let mut groups = 0;
    for edges in grouped.values() {
        if relation_sensitive {
            let mut by_relation: IndexMap<String, IndexSet<String>> = IndexMap::new();
            for edge in edges {
                let relation = edge.get("relation").cloned().unwrap_or_default();
                let value = edge.get(field).cloned().unwrap_or_default();
                by_relation.entry(relation).or_default().insert(value);
            }
            groups += by_relation.values().filter(|v| v.len() > 1).count();
        } else {
            let distinct: IndexSet<String> = edges
                .iter()
                .map(|e| e.get(field).cloned().unwrap_or_default())
                .collect();
            if distinct.len() > 1 {
                groups += 1;
            }
        }
    }
    groups
}

fn tuple_arity_from_annotation(line: &str) -> usize {
    let Some(caps) = TYPE_TUPLE.captures(line) else {
        return 0;
    };
    let inside = caps.name("inside").map_or("", |m| m.as_str()).trim();
    if inside.is_empty() {
        return 0;
    }
    inside.matches(',').count() + 1
}

/// Scan a Python extractor source file for `seen_*` producer-suppression
/// sets. Used as heuristic evidence in the diagnostic report.
///
/// Returns a JSON-shaped summary (`path`, `total_sites`, `sites`, `error`).
#[must_use]
pub fn scan_producer_suppression_sites(path: &Path) -> Value {
    if !path.exists() {
        return json!({
            "path": path.to_string_lossy(),
            "total_sites": 0,
            "sites": [],
            "error": "file not found",
        });
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return json!({
            "path": path.to_string_lossy(),
            "total_sites": 0,
            "sites": [],
            "error": "could not read file",
        });
    };
    let mut sites: Vec<Value> = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let Some(caps) = SUPPRESSION_DECL.captures(line) else {
            continue;
        };
        let name = caps.name("name").map_or("", |m| m.as_str()).to_string();
        let arity = tuple_arity_from_annotation(line);
        let sample: String = line.trim().chars().take(120).collect();
        sites.push(json!({
            "line": idx + 1,
            "name": name,
            "tuple_arity": arity,
            "sample": sample,
        }));
    }
    json!({
        "path": path.to_string_lossy(),
        "total_sites": sites.len(),
        "sites": sites,
        "error": "",
    })
}

/// Options controlling [`diagnose_extraction`] behaviour.
#[derive(Debug, Clone)]
pub struct DiagnoseOptions<'a> {
    /// Run the post-build phase with the directed flag set this way.
    pub directed: bool,
    /// Maximum number of high-multiplicity edge groups to include in the
    /// `examples` section. `0` disables examples.
    pub max_examples: usize,
    /// Path to the Python extractor file to scan for `seen_*` producer
    /// suppression sites. The Rust port has no producer-source equivalent;
    /// callers may point this at `graphify-py/graphify/extract.py` when they
    /// want the original heuristic, or `None` to skip.
    pub extract_path: Option<&'a Path>,
}

impl Default for DiagnoseOptions<'_> {
    fn default() -> Self {
        Self {
            directed: true,
            max_examples: 5,
            extract_path: None,
        }
    }
}

/// Summarise same-endpoint edge-collapse risk for a graph/extraction dict.
///
/// Returns a [`Value`] (JSON object) so callers can pass the result straight
/// through to [`format_diagnostic_json`] or [`format_diagnostic_report`]
/// without further parsing.
#[must_use]
#[allow(clippy::too_many_lines)] // single-pass diagnostic walk — splitting hurts clarity
#[allow(clippy::missing_panics_doc)] // no panics; all `unwrap` paths are in the JSON builder
pub fn diagnose_extraction(extraction: &Map<String, Value>, opts: &DiagnoseOptions) -> Value {
    let node_ids = node_ids(extraction);
    let raw_edges = edge_list(extraction);
    let canonical_edges: Vec<IndexMap<String, String>> =
        raw_edges.iter().map(|e| canonical_edge(e)).collect();

    let mut exact_counts: IndexMap<String, usize> = IndexMap::new();
    for edge in &raw_edges {
        *exact_counts.entry(exact_signature(edge)).or_insert(0) += 1;
    }

    let mut directed_pairs: IndexMap<(String, String), usize> = IndexMap::new();
    let mut undirected_pairs: IndexMap<(String, String), usize> = IndexMap::new();
    let mut grouped: IndexMap<(String, String), Vec<IndexMap<String, String>>> = IndexMap::new();

    let mut non_object_edges = 0_usize;
    let mut missing_endpoint_edges = 0_usize;
    let mut dangling_endpoint_edges = 0_usize;
    let mut self_loop_edges = 0_usize;
    let mut valid_candidate_edges = 0_usize;

    for edge in &canonical_edges {
        if edge.get("_invalid").is_some_and(|s| !s.is_empty()) {
            non_object_edges += 1;
            continue;
        }
        let source = edge.get("source").cloned().unwrap_or_default();
        let target = edge.get("target").cloned().unwrap_or_default();
        if source.is_empty() || target.is_empty() {
            missing_endpoint_edges += 1;
            continue;
        }
        if !node_ids.contains(&source) || !node_ids.contains(&target) {
            dangling_endpoint_edges += 1;
            continue;
        }
        if source == target {
            self_loop_edges += 1;
        }
        valid_candidate_edges += 1;
        let directed_pair = (source.clone(), target.clone());
        let undirected_pair = if source <= target {
            (source.clone(), target.clone())
        } else {
            (target.clone(), source.clone())
        };
        *directed_pairs.entry(directed_pair.clone()).or_insert(0) += 1;
        *undirected_pairs.entry(undirected_pair).or_insert(0) += 1;
        grouped.entry(directed_pair).or_default().push(edge.clone());
    }

    // Build examples — high-multiplicity groups first.
    let mut examples: Vec<Value> = Vec::new();
    if opts.max_examples > 0 {
        let mut pairs_sorted: Vec<(&(String, String), &usize)> = directed_pairs.iter().collect();
        pairs_sorted.sort_by(|a, b| b.1.cmp(a.1));
        for (pair, count) in pairs_sorted {
            if *count < 2 {
                continue;
            }
            let edges = &grouped[pair];
            let relations: IndexSet<String> = edges
                .iter()
                .filter_map(|e| e.get("relation").cloned())
                .collect();
            let mut relations: Vec<String> = relations.into_iter().collect();
            relations.sort();
            let source_files: IndexSet<String> = edges
                .iter()
                .filter_map(|e| e.get("source_file").cloned())
                .collect();
            let mut source_files: Vec<String> = source_files.into_iter().collect();
            source_files.sort();
            let source_locations: IndexSet<String> = edges
                .iter()
                .filter_map(|e| e.get("source_location").cloned())
                .collect();
            let mut source_locations: Vec<String> = source_locations.into_iter().collect();
            source_locations.sort();
            let contexts: IndexSet<String> = edges
                .iter()
                .filter_map(|e| e.get("context").cloned())
                .collect();
            let mut contexts: Vec<String> = contexts.into_iter().collect();
            contexts.sort();
            examples.push(json!({
                "source": pair.0,
                "target": pair.1,
                "edge_count": count,
                "relations": relations,
                "source_files": source_files,
                "source_locations": source_locations,
                "contexts": contexts,
            }));
            if examples.len() >= opts.max_examples {
                break;
            }
        }
    }

    // Post-build phase — try to build the graph and capture the resulting
    // node / edge counts. Errors are captured (not raised) so the diagnostic
    // never aborts.
    let mut graph_type = String::new();
    let mut post_build_node_count: Option<usize> = None;
    let mut post_build_edge_count: Option<usize> = None;
    let mut build_error = String::new();
    let extraction_for_build = Value::Object(extraction.clone());
    match graphify_build::build_from_json(extraction_for_build, opts.directed, None) {
        Ok(graph) => {
            graph_type = format!("{:?}", graph.kind);
            post_build_node_count = Some(graph.node_count());
            post_build_edge_count = Some(graph.edge_count());
        }
        Err(err) => {
            build_error = err.to_string();
        }
    }

    let suppression = if let Some(p) = opts.extract_path {
        scan_producer_suppression_sites(p)
    } else {
        json!({"path": "", "total_sites": 0, "sites": [], "error": ""})
    };

    let same_endpoint_group_count = directed_pairs.values().filter(|&&c| c > 1).count();

    json!({
        "node_count": node_ids.len(),
        "raw_edge_count": raw_edges.len(),
        "non_object_edges": non_object_edges,
        "missing_endpoint_edges": missing_endpoint_edges,
        "dangling_endpoint_edges": dangling_endpoint_edges,
        "self_loop_edges": self_loop_edges,
        "valid_candidate_edges": valid_candidate_edges,
        "exact_duplicate_edges": count_extra(&exact_counts),
        "directed_unique_endpoint_pairs": directed_pairs.len(),
        "directed_same_endpoint_collapsed_edges":
            count_extra(&pair_counter_to_index_map(&directed_pairs)),
        "undirected_unique_endpoint_pairs": undirected_pairs.len(),
        "undirected_same_endpoint_collapsed_edges":
            count_extra(&pair_counter_to_index_map(&undirected_pairs)),
        "same_endpoint_group_count": same_endpoint_group_count,
        "relation_variant_groups": variant_group_count(&grouped, "relation", false),
        "source_file_variant_groups": variant_group_count(&grouped, "source_file", true),
        "source_location_variant_groups":
            variant_group_count(&grouped, "source_location", true),
        "context_variant_groups": variant_group_count(&grouped, "context", true),
        "post_build_graph_type": graph_type,
        "post_build_node_count": post_build_node_count,
        "post_build_edge_count": post_build_edge_count,
        "post_build_error": build_error,
        "producer_suppression": suppression,
        "examples": examples,
    })
}

fn pair_counter_to_index_map(pairs: &IndexMap<(String, String), usize>) -> IndexMap<String, usize> {
    pairs
        .iter()
        .map(|((s, t), c)| (format!("{s}->{t}"), *c))
        .collect()
}

/// Read a graph/extraction JSON file from disk and run [`diagnose_extraction`].
///
/// Honours the JSON's `directed` flag when `directed` is `None`.
///
/// # Errors
///
/// Returns [`DiagnosticsError`] when the file is oversize, cannot be read,
/// cannot be parsed as JSON, or is not a JSON object.
pub fn diagnose_file(
    path: &Path,
    directed: Option<bool>,
    max_examples: usize,
    extract_path: Option<&Path>,
) -> Result<Value, DiagnosticsError> {
    graphify_security::check_graph_file_size_cap(path)?;
    let text = std::fs::read_to_string(path)?;
    let data: Value = serde_json::from_str(&text)?;
    let Value::Object(map) = data else {
        return Err(DiagnosticsError::NotAnObject);
    };

    let effective_directed =
        directed.unwrap_or_else(|| map.get("directed").and_then(Value::as_bool).unwrap_or(true));

    let opts = DiagnoseOptions {
        directed: effective_directed,
        max_examples,
        extract_path,
    };
    let mut summary = diagnose_extraction(&map, &opts);
    if let Some(obj) = summary.as_object_mut() {
        obj.insert(
            "input_path".to_string(),
            Value::String(path.to_string_lossy().into_owned()),
        );
        obj.insert(
            "effective_directed".to_string(),
            Value::Bool(effective_directed),
        );
    }
    Ok(summary)
}

/// Wrap a summary in the canonical JSON envelope (`schema_version`,
/// `summary`, `examples`, `producer_suppression`, `notes`).
#[must_use]
pub fn format_diagnostic_json(summary: &Value) -> Value {
    let mut summary_obj = Map::new();
    let examples = summary
        .get("examples")
        .cloned()
        .unwrap_or(Value::Array(vec![]));
    let suppression = summary
        .get("producer_suppression")
        .cloned()
        .unwrap_or(Value::Object(Map::new()));
    if let Some(map) = summary.as_object() {
        for (k, v) in map {
            if k != "examples" && k != "producer_suppression" {
                summary_obj.insert(k.clone(), v.clone());
            }
        }
    }
    json!({
        "schema_version": 1,
        "summary": Value::Object(summary_obj),
        "examples": examples,
        "producer_suppression": suppression,
        "notes": [
            "Diagnostics are read-only.",
            "A normal graph.json is already post-build and cannot recover raw producer edges.",
            "Producer suppression sites are heuristic source-code evidence.",
        ],
    })
}

/// Human-readable text report. Mirrors the line-by-line format of the
/// Python `format_diagnostic_report` so existing scripts can keep grepping
/// for the same field labels.
#[must_use]
#[allow(clippy::too_many_lines)] // line-by-line report — mirrors Python format
pub fn format_diagnostic_report(summary: &Value) -> String {
    let Some(map) = summary.as_object() else {
        return String::new();
    };
    let suppression = map
        .get("producer_suppression")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let get_field = |k: &str| map.get(k).cloned().unwrap_or(Value::Null);
    let stringify = |v: &Value| match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };

    let mut lines: Vec<String> = vec![
        "[graphify] MultiDiGraph edge-collapse diagnostic".to_string(),
        format!(
            "input: {}",
            stringify(&get_field("input_path"))
                .as_str()
                .strip_prefix('"')
                .unwrap_or(stringify(&get_field("input_path")).as_str())
                .replace('"', "")
        ),
        "input_stage: provided JSON (normal graph.json is post-build)".to_string(),
        format!(
            "effective_directed: {}",
            match map.get("effective_directed") {
                Some(Value::Bool(b)) => b.to_string(),
                _ => "<direct-call>".to_string(),
            }
        ),
        format!("nodes: {}", stringify(&get_field("node_count"))),
        format!("raw_edges: {}", stringify(&get_field("raw_edge_count"))),
        format!(
            "valid_candidate_edges: {}",
            stringify(&get_field("valid_candidate_edges"))
        ),
        format!(
            "missing_endpoint_edges: {}",
            stringify(&get_field("missing_endpoint_edges"))
        ),
        format!(
            "dangling_endpoint_edges: {}",
            stringify(&get_field("dangling_endpoint_edges"))
        ),
        format!(
            "self_loop_edges: {}",
            stringify(&get_field("self_loop_edges"))
        ),
        format!(
            "exact_duplicate_edges: {}",
            stringify(&get_field("exact_duplicate_edges"))
        ),
        format!(
            "directed_unique_endpoint_pairs: {}",
            stringify(&get_field("directed_unique_endpoint_pairs"))
        ),
        format!(
            "directed_same_endpoint_collapsed_edges: {}",
            stringify(&get_field("directed_same_endpoint_collapsed_edges"))
        ),
        format!(
            "undirected_unique_endpoint_pairs: {}",
            stringify(&get_field("undirected_unique_endpoint_pairs"))
        ),
        format!(
            "undirected_same_endpoint_collapsed_edges: {}",
            stringify(&get_field("undirected_same_endpoint_collapsed_edges"))
        ),
        format!(
            "same_endpoint_group_count: {}",
            stringify(&get_field("same_endpoint_group_count"))
        ),
        format!(
            "relation_variant_groups: {}",
            stringify(&get_field("relation_variant_groups"))
        ),
        format!(
            "source_file_variant_groups: {}",
            stringify(&get_field("source_file_variant_groups"))
        ),
        format!(
            "source_location_variant_groups: {}",
            stringify(&get_field("source_location_variant_groups"))
        ),
        format!(
            "context_variant_groups: {}",
            stringify(&get_field("context_variant_groups"))
        ),
        format!(
            "post_build_graph_type: {}",
            stringify(&get_field("post_build_graph_type"))
        ),
        format!(
            "post_build_edges: {}",
            stringify(&get_field("post_build_edge_count"))
        ),
        format!(
            "producer_suppression_sites: {}",
            suppression
                .get("total_sites")
                .map_or_else(|| "0".to_string(), std::string::ToString::to_string)
        ),
    ];
    let build_err = stringify(&get_field("post_build_error"));
    if !build_err.is_empty() {
        lines.push(format!("post_build_error: {build_err}"));
    }
    if let Some(err) = suppression.get("error").and_then(Value::as_str)
        && !err.is_empty()
    {
        lines.push(format!("producer_suppression_error: {err}"));
    }
    if let Some(sites) = suppression.get("sites").and_then(Value::as_array)
        && !sites.is_empty()
    {
        lines.push("producer_suppression_examples:".to_string());
        for site in sites.iter().take(8) {
            let Some(map) = site.as_object() else {
                continue;
            };
            let line = map.get("line").map(Value::to_string).unwrap_or_default();
            let name = map.get("name").and_then(Value::as_str).unwrap_or_default();
            let arity = map.get("tuple_arity").and_then(Value::as_u64).unwrap_or(0);
            let arity_str = if arity == 0 {
                "unknown".to_string()
            } else {
                arity.to_string()
            };
            lines.push(format!("  - L{line} {name} arity={arity_str}"));
        }
    }
    if let Some(examples) = get_field("examples").as_array()
        && !examples.is_empty()
    {
        lines.push("examples:".to_string());
        for ex in examples {
            let Some(em) = ex.as_object() else {
                continue;
            };
            let s = em.get("source").and_then(Value::as_str).unwrap_or("");
            let t = em.get("target").and_then(Value::as_str).unwrap_or("");
            let cnt = em
                .get("edge_count")
                .map(Value::to_string)
                .unwrap_or_default();
            let relations = serde_json::to_string(em.get("relations").unwrap_or(&Value::Null))
                .unwrap_or_default();
            let locations =
                serde_json::to_string(em.get("source_locations").unwrap_or(&Value::Null))
                    .unwrap_or_default();
            let contexts = serde_json::to_string(em.get("contexts").unwrap_or(&Value::Null))
                .unwrap_or_default();
            lines.push(format!(
                "  - {s} -> {t} edges={cnt} relations={relations} locations={locations} contexts={contexts}"
            ));
        }
    }
    lines.push(
        "note: normal graph.json is post-build; raw producer loss must be measured earlier."
            .to_string(),
    );
    lines.join("\n")
}
