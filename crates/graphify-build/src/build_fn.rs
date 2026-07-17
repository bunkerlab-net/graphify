//! Top-level [`build_from_json`] and [`build`] drivers.

use std::path::Path;
use std::sync::LazyLock;

use indexmap::IndexMap;
use serde_json::Value;

use crate::dedup_label::deduplicate_by_label;
use crate::error::BuildError;
use crate::graph::{Graph, GraphKind};
use crate::ingest::{add_edges, add_nodes, canonicalise_nodes, merge_ghost_duplicates};
use crate::normalize::norm_source_file;

static PERF_LOG: LazyLock<bool> = LazyLock::new(|| std::env::var("GRAPHIFY_PERF_LOG").is_ok());

/// Canonicalise a root path to a string for `source_file` relativisation,
/// falling back to the path as-is when it cannot be resolved (e.g. a
/// non-existent root in tests).
#[must_use]
fn canonicalize_root_to_string(root: &Path) -> String {
    root.canonicalize()
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Collapse nodes sharing an `id`, last-writer-wins on attributes.
///
/// Node insertion in [`build_from_json`] is idempotent — a later node with the
/// same `id` overwrites the earlier one's attributes — but the `--no-cluster`
/// write path dumps the raw node list without building a graph, so same-id
/// nodes (e.g. a Swift `type=module` anchor emitted once per importing file,
/// #1327) would otherwise survive as duplicates. Insertion order follows each
/// id's first appearance; the retained object is the last one seen. Nodes whose
/// `id` is missing, null, or non-string are skipped.
///
/// Mirrors graphify-py `build.dedupe_nodes`.
#[must_use]
pub fn dedupe_nodes(nodes: &[Value]) -> Vec<Value> {
    let mut by_id: IndexMap<String, Value> = IndexMap::new();
    for node in nodes {
        let Some(id) = node.get("id").and_then(Value::as_str) else {
            continue;
        };
        by_id.insert(id.to_owned(), node.clone());
    }
    by_id.into_values().collect()
}

/// Collapse exact parallel edges by `(source, target, relation)`, keeping the
/// first occurrence and preserving order.
///
/// The clustered build path runs edges through a graph that collapses parallel
/// edges automatically; the `--no-cluster` and incremental `update` write paths
/// bypass it and concatenate edge lists raw, so duplicates accumulate and edge
/// counts become non-deterministic across build modes and repeated updates
/// (#1317). Deduping on the connectivity identity is zero-signal-loss and
/// restores idempotency. Callers that intentionally keep parallel edges
/// (multigraph output) must not use this.
///
/// Mirrors graphify-py `build.dedupe_edges`.
#[must_use]
pub fn dedupe_edges(edges: &[Value]) -> Vec<Value> {
    type EdgeKey = (Option<String>, Option<String>, Option<String>);
    let mut seen: std::collections::HashSet<EdgeKey> = std::collections::HashSet::new();
    let mut out: Vec<Value> = Vec::with_capacity(edges.len());
    for edge in edges {
        let component = |k: &str| edge.get(k).and_then(Value::as_str).map(String::from);
        let key = (
            component("source"),
            component("target"),
            component("relation"),
        );
        if seen.insert(key) {
            out.push(edge.clone());
        }
    }
    out
}

/// Build a graph from a single extraction dict.
///
/// Mirrors Python `build_from_json(extraction, directed=False, root=None)`.
///
/// The function:
/// 1. Renames `"links"` → `"edges"` for compatibility with `NetworkX` dumps.
/// 2. Canonicalises node `file_type` values and renames `source` →
///    `source_file`.
/// 3. Runs the validator and surfaces real schema warnings on stderr
///    (dangling-edge warnings are suppressed since stdlib/external
///    imports are expected).
/// 4. Inserts nodes and edges into a fresh [`Graph`].
/// 5. Preserves `hyperedges` on `graph.graph_attrs` for downstream
///    consumers.
///
/// # Errors
///
/// Currently infallible (returns `Result` for API parity with [`build`],
/// which can fail with [`BuildError::WouldShrink`]).
pub fn build_from_json(
    mut extraction: Value,
    directed: bool,
    root: Option<&Path>,
) -> Result<Graph, BuildError> {
    let root_str = root.map(canonicalize_root_to_string);
    let kind = if directed {
        GraphKind::DiGraph
    } else {
        GraphKind::Graph
    };

    let Some(obj) = extraction.as_object_mut() else {
        return Ok(Graph::new(kind));
    };
    if !obj.contains_key("edges")
        && let Some(links) = obj.remove("links")
    {
        obj.insert("edges".into(), links);
    }

    let perf = *PERF_LOG;
    let t = std::time::Instant::now();
    canonicalise_nodes(&mut extraction);
    if perf {
        eprintln!(
            "[perf]   build_from_json/canonicalise: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }
    let t = std::time::Instant::now();
    // Mirror Python `build.py:148-152`: surface real schema errors, but ignore
    // dangling-edge warnings (stdlib/external imports are expected).
    let errors = graphify_validate::validate_extraction(&extraction);
    if perf {
        eprintln!(
            "[perf]   build_from_json/validate: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }
    let real_errors: Vec<&String> = errors
        .iter()
        .filter(|e| !e.contains("does not match any node id"))
        .collect();
    if let Some(first) = real_errors.first() {
        eprintln!(
            "[graphify] Extraction warning ({} issues): {first}",
            real_errors.len()
        );
    }
    // Parity dispute (CodeRabbit): validation runs once here, BEFORE the semantic
    // re-key below, matching graphify-py build.py. It is warnings-only (never
    // aborts), and `add_nodes` dedups any id the re-key collapses (last write
    // wins, like networkx), so a second post-rekey validation would only emit
    // warnings graphify-py never prints.

    // Deterministic semantic re-key (#1504/#1509): re-derive every non-AST node's
    // id from its own `source_file` so a cached/LLM fragment carrying a
    // pre-migration short id reconciles with the AST node instead of spawning a
    // ghost / a re-bill. AST-origin nodes are already canonical and untouched.
    crate::migrate::apply_semantic_rekey(&mut extraction, root_str.as_deref());

    // Merge a markdown quick-scan's bare doc node into its semantic `_doc` twin
    // for the same file, so a document is one node regardless of which pipeline
    // touched it last (#1799). Runs in the same extraction-mutation phase as the
    // semantic re-key, before graph construction.
    apply_doc_twin_remap(&mut extraction);

    let mut graph = Graph::new(kind);
    let t = std::time::Instant::now();
    add_nodes(&mut graph, &mut extraction, root_str.as_deref());
    if perf {
        eprintln!(
            "[perf]   build_from_json/add_nodes: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }
    // Collapse LLM ghost-duplicate nodes onto their AST canonical twins before
    // wiring edges, so edges referencing a ghost re-point to the canonical node.
    let ghost_remap = merge_ghost_duplicates(&mut graph);
    let t = std::time::Instant::now();
    add_edges(&mut graph, &extraction, root_str.as_deref(), &ghost_remap);
    if perf {
        eprintln!(
            "[perf]   build_from_json/add_edges: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }

    attach_validated_hyperedges(
        &mut graph,
        &mut extraction,
        &ghost_remap,
        root_str.as_deref(),
    );

    Ok(graph)
}

/// Relativise, validate, and attach hyperedges to the built graph (#1916/#1418).
///
/// Members absent from the built node set are pruned — mismatched ids remapped
/// through the same endpoint index `add_edges` uses (ghost + legacy aliases) —
/// and a hyperedge with no surviving member is dropped whole. `source_file` is
/// relativised so `to_json` (which writes `graph.hyperedges` verbatim and has no
/// root) never leaks an absolute path.
fn attach_validated_hyperedges(
    graph: &mut Graph,
    extraction: &mut Value,
    ghost_remap: &indexmap::IndexMap<String, String>,
    root_str: Option<&str>,
) {
    let Some(arr) = extraction
        .as_object_mut()
        .and_then(|o| o.get_mut("hyperedges"))
        .and_then(Value::as_array_mut)
        .filter(|a| !a.is_empty())
    else {
        return;
    };
    let (node_ids, norm_to_id) = crate::ingest::build_endpoint_index(graph, ghost_remap);
    let mut kept: Vec<Value> = Vec::with_capacity(arr.len());
    for he in arr.iter_mut() {
        let Some(map) = he.as_object_mut() else {
            kept.push(he.clone());
            continue;
        };
        normalize_hyperedge_members(map);
        if let Some(sf) = map.get("source_file").and_then(Value::as_str)
            && !sf.is_empty()
        {
            let normalized = norm_source_file(sf, root_str);
            map.insert("source_file".to_string(), Value::String(normalized));
        }
        if let Some(members) = map.get("nodes").and_then(Value::as_array) {
            let original = members.clone();
            let mut valid: Vec<Value> = Vec::with_capacity(original.len());
            for m in &original {
                let Some(s) = m.as_str() else {
                    continue; // non-string member: unresolvable
                };
                let resolved = crate::ingest::resolve_edge_id(s, &node_ids, &norm_to_id);
                if node_ids.contains(&resolved) {
                    valid.push(Value::String(resolved));
                }
            }
            if valid.is_empty() {
                let id = map.get("id").and_then(Value::as_str).unwrap_or("?");
                eprintln!(
                    "[graphify] WARNING: dropping hyperedge '{id}' — none of its \
                     members match built nodes."
                );
                continue;
            }
            if valid != original {
                map.insert("nodes".to_string(), Value::Array(valid));
            }
        }
        kept.push(he.clone());
    }
    if !kept.is_empty() {
        graph
            .graph_attrs
            .insert("hyperedges".to_string(), Value::Array(kept));
    }
}

/// Map a markdown quick-scan's bare doc node `<slug>` to the semantic
/// `<slug>_doc` node for the SAME file (#1799).
///
/// The markdown quick-scan mints a bare `<slug>` file node while the semantic
/// pass mints `<slug>_doc`; a `graphify update` after a semantic build leaves
/// both, splitting the file's edges across two disconnected nodes. Both twins
/// must be `file_type == "document"` with an identical non-empty `source_file`,
/// so an unrelated code symbol `foo` and `foo_doc` never merge.
fn doc_twin_remap(nodes: &[Value]) -> IndexMap<String, String> {
    let mut by_id: IndexMap<&str, &Value> = IndexMap::new();
    for n in nodes {
        if let Some(id) = n.get("id").and_then(Value::as_str)
            && !id.is_empty()
        {
            by_id.insert(id, n);
        }
    }
    let mut remap: IndexMap<String, String> = IndexMap::new();
    for (nid, node) in &by_id {
        let Some(bare_id) = nid.strip_suffix("_doc") else {
            continue;
        };
        let Some(&bare) = by_id.get(bare_id) else {
            continue;
        };
        let sf = node
            .get("source_file")
            .and_then(Value::as_str)
            .unwrap_or("");
        if sf.is_empty() || bare.get("source_file").and_then(Value::as_str) != Some(sf) {
            continue;
        }
        if node.get("file_type").and_then(Value::as_str) != Some("document")
            || bare.get("file_type").and_then(Value::as_str) != Some("document")
        {
            continue;
        }
        remap.insert(bare_id.to_string(), (*nid).to_string());
    }
    remap
}

/// Drop bare doc-twin nodes, repoint their edges/hyperedges onto the semantic
/// `_doc` node, and drop only the self-loops the remap itself collapsed (#1799).
fn apply_doc_twin_remap(extraction: &mut Value) {
    let remap = match extraction.get("nodes").and_then(Value::as_array) {
        Some(nodes) => doc_twin_remap(nodes),
        None => return,
    };
    if remap.is_empty() {
        return;
    }
    let Some(obj) = extraction.as_object_mut() else {
        return;
    };
    if let Some(Value::Array(nodes)) = obj.get_mut("nodes") {
        nodes.retain(|n| {
            n.get("id")
                .and_then(Value::as_str)
                .is_none_or(|id| !remap.contains_key(id))
        });
    }
    if let Some(Value::Array(edges)) = obj.get_mut("edges") {
        let mut kept: Vec<Value> = Vec::with_capacity(edges.len());
        for mut edge in edges.drain(..) {
            if let Some(map) = edge.as_object_mut() {
                let s0 = map.get("source").and_then(Value::as_str).map(str::to_owned);
                let t0 = map.get("target").and_then(Value::as_str).map(str::to_owned);
                let new_s = s0.as_deref().and_then(|s| remap.get(s)).cloned();
                let new_t = t0.as_deref().and_then(|t| remap.get(t)).cloned();
                let remapped = new_s.is_some() || new_t.is_some();
                if let Some(ns) = &new_s {
                    map.insert("source".to_string(), Value::String(ns.clone()));
                }
                if let Some(nt) = &new_t {
                    map.insert("target".to_string(), Value::String(nt.clone()));
                }
                let final_s = new_s.or(s0);
                let final_t = new_t.or(t0);
                // Drop a self-loop only when the remap produced it — i.e. an
                // endpoint was remapped and the edge now points to itself (a
                // bare->`_doc` link becoming `doc->doc`). Mirrors graphify-py
                // `build.py:482` exactly (`source == target and (s0 or t0 in
                // remap)`): a self-loop whose own node was remapped is also
                // dropped there, so preserving it would diverge from parity.
                if remapped && final_s.is_some() && final_s == final_t {
                    continue;
                }
            }
            kept.push(edge);
        }
        *edges = kept;
    }
    if let Some(Value::Array(hyperedges)) = obj.get_mut("hyperedges") {
        for he in hyperedges.iter_mut() {
            let Some(map) = he.as_object_mut() else {
                continue;
            };
            // Canonicalize `members`/`node_ids` aliases onto `nodes` FIRST (as
            // graphify-py does before its doc-remap), so an aliased member list
            // is remapped too and never keeps a removed bare id.
            normalize_hyperedge_members(map);
            if let Some(members) = map.get_mut("nodes").and_then(Value::as_array_mut) {
                for m in members.iter_mut() {
                    if let Some(new) = m.as_str().and_then(|id| remap.get(id)) {
                        *m = Value::String(new.clone());
                    }
                }
            }
        }
    }
}

/// Member-list alias keys a hyperedge may use in place of the canonical `nodes`.
const HE_MEMBER_ALIASES: [&str; 2] = ["members", "node_ids"];

/// Canonicalize a hyperedge's member list onto the `nodes` key, in place (#1561).
///
/// If `nodes` is already an array it wins and only stray alias keys are dropped.
/// Otherwise the first alias (`members`, then `node_ids`) that is an array is
/// moved to `nodes`, deduped preserving order, with a single stderr WARNING; a
/// non-string (unhashable) member is kept for the validator to flag. Leftover
/// alias keys are always removed so downstream code never re-reads them.
fn normalize_hyperedge_members(he: &mut serde_json::Map<String, Value>) {
    if !he.get("nodes").is_some_and(Value::is_array) {
        let found = HE_MEMBER_ALIASES
            .iter()
            .find_map(|&alias| match he.get(alias) {
                Some(Value::Array(vals)) => Some((alias, vals.clone())),
                _ => None,
            });
        if let Some((alias, vals)) = found {
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut deduped: Vec<Value> = Vec::with_capacity(vals.len());
            for ref_v in vals {
                if let Value::String(s) = &ref_v
                    && !seen.insert(s.clone())
                {
                    continue;
                }
                deduped.push(ref_v);
            }
            let id = he
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string();
            he.insert("nodes".to_string(), Value::Array(deduped));
            eprintln!(
                "[graphify] WARNING: hyperedge '{id}' uses field '{alias}' instead of 'nodes'; normalizing."
            );
        } else if let Some(&alias) = HE_MEMBER_ALIASES.iter().find(|&&a| he.contains_key(a)) {
            // graphify-py silently drops a malformed (non-array) alias, leaving the
            // hyperedge member-less; warn instead so a producer typo like
            // `"members": "n1"` surfaces rather than a quietly empty hyperedge.
            let id = he.get("id").and_then(Value::as_str).unwrap_or("?");
            eprintln!(
                "[graphify] WARNING: hyperedge '{id}' field '{alias}' is not an array; ignoring it."
            );
        }
    }
    for alias in HE_MEMBER_ALIASES {
        he.remove(alias);
    }
}

/// Merge multiple extraction dicts into one graph. Mirrors Python
/// `build(...)`.
///
/// `dedup` runs entity deduplication via [`deduplicate_by_label`]. The
/// Python version optionally also calls
/// `graphify.dedup.deduplicate_entities` for LLM-assisted fuzzy
/// matching — that path requires the `graphify-dedup` crate and is
/// opt-in via `dedup_llm_backend`. For now we only support the cheap
/// label-canonical dedup path; LLM-backed dedup is reserved for future
/// work.
///
/// # Errors
///
/// Propagates any error from [`build_from_json`].
pub fn build(
    extractions: &[Value],
    directed: bool,
    dedup: bool,
    root: Option<&Path>,
) -> Result<Graph, BuildError> {
    let mut combined_nodes: Vec<Value> = Vec::new();
    let mut combined_edges: Vec<Value> = Vec::new();
    let mut combined_hyperedges: Vec<Value> = Vec::new();
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;

    for ext in extractions {
        if let Some(arr) = ext.get("nodes").and_then(Value::as_array) {
            combined_nodes.extend(arr.iter().cloned());
        }
        if let Some(arr) = ext.get("edges").and_then(Value::as_array) {
            combined_edges.extend(arr.iter().cloned());
        } else if let Some(arr) = ext.get("links").and_then(Value::as_array) {
            combined_edges.extend(arr.iter().cloned());
        }
        if let Some(arr) = ext.get("hyperedges").and_then(Value::as_array) {
            combined_hyperedges.extend(arr.iter().cloned());
        }
        if let Some(n) = ext.get("input_tokens").and_then(Value::as_u64) {
            input_tokens += n;
        }
        if let Some(n) = ext.get("output_tokens").and_then(Value::as_u64) {
            output_tokens += n;
        }
    }

    if dedup && !combined_nodes.is_empty() {
        let (nodes, edges) = deduplicate_by_label(&combined_nodes, &combined_edges);
        combined_nodes = nodes;
        combined_edges = edges;
    }

    let combined = serde_json::json!({
        "nodes": combined_nodes,
        "edges": combined_edges,
        "hyperedges": combined_hyperedges,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
    });
    build_from_json(combined, directed, root)
}

/// Load an existing `graph.json`, merge `new_chunks` into it, and return the
/// combined graph. Mirrors graphify-py `build_merge`.
///
/// Never replaces — only grows, except when `prune_sources` is supplied, in
/// which case nodes and edges whose `source_file` matches a pruned (deleted)
/// source path are removed. When `root` is set, absolute `source_file` paths in
/// `new_chunks` are made root-relative.
///
/// # Errors
///
/// - [`BuildError::Security`] if the existing graph file exceeds the size cap.
/// - [`BuildError::WouldShrink`] if the merge would drop nodes without an
///   explicit `prune_sources` opt-in (and `dedup` is off).
/// - Propagates any I/O, JSON, or [`build`] error.
pub fn build_merge(
    new_chunks: &[Value],
    graph_path: &Path,
    prune_sources: Option<&[String]>,
    directed: bool,
    dedup: bool,
    root: Option<&Path>,
) -> Result<Graph, BuildError> {
    build_merge_with_graph_cap(
        new_chunks,
        graph_path,
        prune_sources,
        directed,
        dedup,
        root,
        // Honour GRAPHIFY_MAX_GRAPH_BYTES so large codebases can raise the cap.
        graphify_security::max_graph_file_bytes(),
    )
}

/// [`build_merge`] with an explicit graph-file size cap.
///
/// Exposed so callers (and tests) can exercise the oversize-rejection path with
/// a custom cap, mirroring graphify-py's `_MAX_GRAPH_FILE_BYTES` override.
/// Production callers should prefer [`build_merge`].
///
/// # Errors
///
/// See [`build_merge`].
pub fn build_merge_with_graph_cap(
    new_chunks: &[Value],
    graph_path: &Path,
    prune_sources: Option<&[String]>,
    directed: bool,
    dedup: bool,
    root: Option<&Path>,
    graph_cap: u64,
) -> Result<Graph, BuildError> {
    let graph_existed = graph_path.exists();
    let mut all_chunks: Vec<Value> = Vec::with_capacity(new_chunks.len() + 1);
    let mut existing_node_count = 0usize;
    let mut existing_hyperedges: Vec<Value> = Vec::new();

    // Effective root for relativizing absolute source_file / prune paths back to
    // the stored relative keys. Caller root wins; else fall back to the graph's
    // recorded scan root (the `.graphify_root` marker, then the output dir's
    // parent) so an absolute prune path or new-chunk path still matches even when
    // a caller omits `root` (#1571).
    let eff_root: Option<String> = root
        .map(canonicalize_root_to_string)
        .or_else(|| infer_merge_root(graph_path).map(|p| p.to_string_lossy().into_owned()));
    // Re-extracted files (present in `new_chunks`) REPLACE their prior
    // contribution and must never be pruned (#1796). Collect them
    // unconditionally — even on a first build with no existing graph — so a
    // source listed in BOTH new_chunks and prune_sources keeps its fresh nodes.
    let new_sources = collect_new_chunk_sources(new_chunks, eff_root.as_deref());

    if graph_existed {
        // Read the JSON directly rather than via a graph round-trip: an
        // undirected round-trip re-derives edge endpoints from node-insertion
        // order and silently flips directional edges (#760). The size cap guards
        // against a memory-bomb graph file.
        graphify_security::check_graph_file_size_cap_with(graph_path, graph_cap)?;
        let text = std::fs::read_to_string(graph_path)?;
        let data: Value =
            serde_json::from_str(&text).map_err(|e| crate::error::BuildError::CorruptGraph {
                path: graph_path.display().to_string(),
                source: e,
            })?;
        let mut existing_nodes = data
            .get("nodes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut existing_edges = data
            .get("links")
            .or_else(|| data.get("edges"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        existing_hyperedges = data
            .get("hyperedges")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // #1344: re-extracted files REPLACE their prior contribution. Drop from
        // the loaded graph every node/edge whose source_file (raw or
        // root-normalised) is re-emitted in `new_chunks`. Files absent from
        // `new_chunks` stay untouched; deletions go via prune_sources.
        if !new_sources.is_empty() {
            existing_nodes
                .retain(|n| source_file_not_replaced(n, &new_sources, eff_root.as_deref()));
            existing_edges
                .retain(|e| source_file_not_replaced(e, &new_sources, eff_root.as_deref()));
        }
        existing_node_count = existing_nodes.len();
        all_chunks.push(serde_json::json!({
            "nodes": existing_nodes,
            "edges": existing_edges,
        }));
    }

    all_chunks.extend(new_chunks.iter().cloned());
    let mut graph = build(&all_chunks, directed, dedup, root)?;

    // Control flow, the deleted-file COUNT in messages, and the shrink guard use
    // the RAW `prune_sources`; only the MATCHING set excludes re-extracted files.
    let pruned_raw = prune_sources.unwrap_or(&[]);
    // A file just re-extracted (present in `new_chunks`) is being REPLACED, never
    // deleted — so never prune it, even if the caller also lists it in
    // `prune_sources` (#1796). Otherwise its fresh, just-built nodes are silently
    // removed (data loss) when an edit keeps a node's label and the caller
    // follows the old workflow of passing the changed file in prune_sources.
    // "replace" wins over a contradictory "delete" of the same source.
    let pruned_effective: Vec<String> = pruned_raw
        .iter()
        .filter(|p| {
            let norm = crate::normalize::norm_source_file(p, eff_root.as_deref());
            !new_sources.contains(p.as_str()) && !new_sources.contains(norm.as_str())
        })
        .cloned()
        .collect();
    // Prune set for deleted sources — both raw and root-normalised forms so an
    // absolute deleted path matches a relativised node key (#1007/#1571).
    let mut prune_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for p in &pruned_effective {
        if p.is_empty() {
            continue;
        }
        prune_set.insert(p.clone());
        let norm = crate::normalize::norm_source_file(p, eff_root.as_deref());
        if !norm.is_empty() {
            prune_set.insert(norm);
        }
    }

    // Prune deleted sources FIRST so carried hyperedges can be validated against
    // the FINAL node set below: a hyperedge whose own file survives can still
    // reference a member node from a file that was pruned or re-extracted. Gated
    // on RAW prune_sources so a contradictory replace+delete still emits the
    // "already clean" message with the raw file count (matching graphify-py).
    if !pruned_raw.is_empty() {
        prune_deleted_sources(
            &mut graph,
            &pruned_effective,
            pruned_raw.len(),
            eff_root.as_deref().map(Path::new),
        );
    }

    // Carry forward hyperedges from files neither re-extracted nor deleted, and
    // drop any left dangling by the prune above (#1574); see the helper.
    carry_forward_hyperedges(
        &mut graph,
        std::mem::take(&mut existing_hyperedges),
        &new_sources,
        &prune_set,
        eff_root.as_deref(),
    );

    // Refuse to silently shrink the graph (#479). Shrinkage is intentional when
    // dedup or prune_sources is active, so only guard otherwise.
    if graph_existed && !dedup && pruned_raw.is_empty() {
        let now = graph.node_count();
        if now < existing_node_count {
            return Err(BuildError::WouldShrink {
                prev: existing_node_count,
                now,
            });
        }
    }

    Ok(graph)
}

/// Carry forward hyperedges (#1574) from files neither re-extracted nor deleted.
///
/// `build()` only sees the new chunks' hyperedges, so without this every
/// `--update` collapses the hyperedge set to just the changed files'.
/// Re-extracted files' prior hyperedges are dropped (their new version is
/// already on the graph); deleted files' are dropped via `prune_set`; id-dedup
/// (in [`attach_carried_hyperedges`]) so a carried hyperedge never duplicates a
/// re-emitted one. A hyperedge is ALSO dropped when any `nodes` member no longer
/// resolves to a live node — a dangling member (from a co-referenced file that
/// was pruned or re-extracted under a new id) breaks referential integrity.
/// graphify-py carries such hyperedges verbatim; fixing it is a deliberate
/// divergence per AGENTS.md (fix reference bugs, don't replicate them). Call
/// AFTER pruning so members are validated against the final graph.
fn carry_forward_hyperedges(
    graph: &mut Graph,
    existing_hyperedges: Vec<Value>,
    new_sources: &std::collections::HashSet<String>,
    prune_set: &std::collections::HashSet<String>,
    eff_root: Option<&str>,
) {
    if existing_hyperedges.is_empty() {
        return;
    }
    let live_nodes: std::collections::HashSet<String> =
        graph.nodes().map(|(id, _)| id.clone()).collect();
    let carried: Vec<Value> = existing_hyperedges
        .into_iter()
        .filter(|he| {
            let Some(map) = he.as_object() else {
                return false;
            };
            let sf = map.get("source_file").and_then(Value::as_str).unwrap_or("");
            let norm = crate::normalize::norm_source_file(sf, eff_root);
            if new_sources.contains(sf)
                || new_sources.contains(&norm)
                || prune_set.contains(sf)
                || prune_set.contains(&norm)
            {
                return false;
            }
            // Every declared member must still resolve to a live node.
            map.get("nodes")
                .and_then(Value::as_array)
                .is_none_or(|members| {
                    members
                        .iter()
                        .all(|m| m.as_str().is_some_and(|id| live_nodes.contains(id)))
                })
        })
        .collect();
    attach_carried_hyperedges(graph, carried);
}

/// Best-effort scan root for relativizing paths in [`build_merge`] when the
/// caller passes no `root` (#1571): the committed `graphify-out/.graphify_root`
/// marker (authoritative), else the output dir's parent (`graph.json`'s
/// grandparent, i.e. `<root>/graphify-out/graph.json` → `<root>`).
fn infer_merge_root(graph_path: &Path) -> Option<std::path::PathBuf> {
    let out_dir = graph_path.parent()?;
    let marker = out_dir.join(".graphify_root");
    if let Ok(text) = std::fs::read_to_string(&marker) {
        let recorded = text.trim();
        if !recorded.is_empty() {
            let p = std::path::PathBuf::from(recorded);
            return Some(p.canonicalize().unwrap_or(p));
        }
    }
    let parent = out_dir.parent()?;
    Some(
        parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf()),
    )
}

/// Merge `carried` hyperedges into `graph.graph_attrs["hyperedges"]` with id-dedup
/// (existing entries win on id collision). Inline so graphify-build stays a leaf
/// crate (no dependency on graphify-export's `attach_hyperedges`).
fn attach_carried_hyperedges(graph: &mut Graph, carried: Vec<Value>) {
    if carried.is_empty() {
        return;
    }
    let slot = graph
        .graph_attrs
        .entry("hyperedges".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !slot.is_array() {
        *slot = Value::Array(Vec::new());
    }
    let Some(arr) = slot.as_array_mut() else {
        return;
    };
    let mut seen: std::collections::HashSet<String> = arr
        .iter()
        .filter_map(|h| h.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    for he in carried {
        // An id-less or empty-id hyperedge is not carried: hyperedges are
        // identified (and deduped) by a non-empty id, matching graphify-py's
        // `attach_hyperedges` and graphify-export (a truthy-id guard).
        let Some(id) = he
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if seen.insert(id.to_string()) {
            arr.push(he);
        }
    }
}

/// Remove nodes and edges whose `source_file` matches any deleted source path.
///
/// The match set holds both the raw path (nodes that kept an absolute
/// `source_file`) and its root-relative normalised form, so manifest absolute
/// paths still match nodes relativised at build time (#1007). `.canonicalize()`
/// resolves symlinked roots and redundant `..`/`.` segments.
fn prune_deleted_sources(
    graph: &mut Graph,
    pruned: &[String],
    file_count: usize,
    root: Option<&Path>,
) {
    let root_str = root.map(canonicalize_root_to_string);
    let mut prune_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for p in pruned {
        if p.is_empty() {
            continue;
        }
        prune_set.insert(p.clone());
        let norm = norm_source_file(p, root_str.as_deref());
        if !norm.is_empty() {
            prune_set.insert(norm);
        }
    }

    let matches_pruned = |attrs: &indexmap::IndexMap<String, Value>| {
        attrs
            .get("source_file")
            .and_then(Value::as_str)
            .is_some_and(|sf| prune_set.contains(sf))
    };

    let to_remove: Vec<String> = graph
        .nodes()
        .filter(|(_, attrs)| matches_pruned(attrs))
        .map(|(id, _)| id.clone())
        .collect();
    let n_nodes = to_remove.len();
    graph.remove_nodes_from(to_remove.iter().map(String::as_str));
    if n_nodes > 0 {
        eprintln!("[graphify] Pruned {n_nodes} node(s) from {file_count} deleted source file(s).");
    }

    let edges_to_remove: Vec<(String, String)> = graph
        .edges()
        .filter(|e| matches_pruned(&e.attrs))
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();
    let n_edges = edges_to_remove.len();
    if n_edges > 0 {
        graph.remove_edges_from(
            edges_to_remove
                .iter()
                .map(|(u, v)| (u.as_str(), v.as_str())),
        );
        eprintln!("[graphify] Pruned {n_edges} edge(s) from deleted source file(s).");
    }

    if n_nodes == 0 && n_edges == 0 {
        eprintln!(
            "[graphify] {file_count} source file(s) deleted since last run — no matching nodes or edges in graph, already clean."
        );
    }
}

/// Collect the `source_file` values (raw and root-normalised) present in
/// `new_chunks` nodes. Items in the loaded graph matching any of these are the
/// stale contribution of a re-extracted file and are dropped before merging so
/// the new version REPLACES the old (#1344).
fn collect_new_chunk_sources(
    new_chunks: &[Value],
    root: Option<&str>,
) -> std::collections::HashSet<String> {
    let mut sources = std::collections::HashSet::new();
    for chunk in new_chunks {
        let Some(nodes) = chunk.get("nodes").and_then(Value::as_array) else {
            continue;
        };
        for node in nodes {
            let Some(sf) = node.get("source_file").and_then(Value::as_str) else {
                continue;
            };
            if sf.is_empty() {
                continue;
            }
            sources.insert(sf.to_string());
            let norm = norm_source_file(sf, root);
            if !norm.is_empty() {
                sources.insert(norm);
            }
        }
    }
    sources
}

/// `true` if `item` (a node or edge) should be KEPT in the loaded graph — i.e.
/// neither its raw nor its root-normalised `source_file` was re-emitted in the
/// new chunks. Items without a `source_file` are always kept.
fn source_file_not_replaced(
    item: &Value,
    new_sources: &std::collections::HashSet<String>,
    root: Option<&str>,
) -> bool {
    let Some(sf) = item.get("source_file").and_then(Value::as_str) else {
        return true;
    };
    if new_sources.contains(sf) {
        return false;
    }
    !new_sources.contains(norm_source_file(sf, root).as_str())
}
