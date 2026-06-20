//! Union-Find, winner selection, and the three deduplication passes.

use std::sync::LazyLock;

use indexmap::IndexMap;
use rayon::prelude::*;
use regex::Regex;
use serde_json::Value;

use crate::{
    DedupError, DedupLlmBackend, JudgeResult,
    minhash::MinHash,
    score::{
        COMMUNITY_BOOST, ENTROPY_THRESHOLD, LSH_THRESHOLD, MERGE_THRESHOLD, entropy,
        is_variant_pair, jaro_score, jaro_winkler_score, make_minhash, norm, numeric_tokens_differ,
        short_label_blocked,
    },
};

/// Candidate-count threshold above which `MinHash` construction and LSH pair
/// scanning are dispatched to Rayon.
const PARALLEL_DEDUP_THRESHOLD: usize = 64;

// ── chunk-suffix regex ────────────────────────────────────────────────────────

#[allow(clippy::expect_used)] // literal pattern; cannot panic at runtime.
static CHUNK_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"_c\d+$").expect("static chunk-suffix regex"));

// ── union-find ────────────────────────────────────────────────────────────────

/// Disjoint-set data structure for tracking which node IDs belong to the same merge group.
///
/// Supports path-compressed `find` and union-by-root `union`. Used by all three
/// deduplication passes to accumulate merge decisions before the final remap is applied.
pub struct UnionFind {
    parent: IndexMap<String, String>,
}

impl Default for UnionFind {
    fn default() -> Self {
        Self::new()
    }
}

impl UnionFind {
    /// Creates a new, empty `UnionFind`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            parent: IndexMap::new(),
        }
    }

    /// Returns the root representative of `x`, inserting it if absent, with path compression.
    pub fn find(&mut self, x: &str) -> String {
        // Ensure x has an entry.
        self.parent
            .entry(x.to_string())
            .or_insert_with(|| x.to_string());
        // Path compression loop (path halving).
        let mut current = x.to_string();
        loop {
            let p = self
                .parent
                .get(&current)
                .cloned()
                .unwrap_or_else(|| current.clone());
            if p == current {
                break;
            }
            let gp = self.parent.get(&p).cloned().unwrap_or_else(|| p.clone());
            self.parent.insert(current.clone(), gp.clone());
            current = gp;
        }
        current
    }

    /// Merges the sets containing `x` and `y`.
    pub fn union(&mut self, x: &str, y: &str) {
        self.parent
            .entry(x.to_string())
            .or_insert_with(|| x.to_string());
        self.parent
            .entry(y.to_string())
            .or_insert_with(|| y.to_string());
        let rx = self.find(x);
        let ry = self.find(y);
        if rx != ry {
            self.parent.insert(ry, rx);
        }
    }

    /// Return a map of root -> \[members\].
    pub fn components(&mut self) -> IndexMap<String, Vec<String>> {
        let keys: Vec<String> = self.parent.keys().cloned().collect();
        let mut groups: IndexMap<String, Vec<String>> = IndexMap::new();
        for k in keys {
            let root = self.find(&k);
            groups.entry(root).or_default().push(k);
        }
        groups
    }
}

// ── winner selection ──────────────────────────────────────────────────────────

/// Pick the canonical survivor node: prefer no `_c<N>` chunk suffix, then
/// shorter ID. Matches `_pick_winner` in Python.
///
/// # Errors
///
/// Returns [`DedupError::EmptyGroup`] if `nodes` is empty.
pub fn pick_winner<'a>(nodes: &'a [&'a Value]) -> Result<&'a Value, DedupError> {
    if nodes.is_empty() {
        return Err(DedupError::EmptyGroup);
    }
    let best = nodes.iter().copied().min_by_key(|n| {
        let id = n.get("id").and_then(Value::as_str).unwrap_or("");
        let has_suffix = CHUNK_SUFFIX.is_match(id);
        (u8::from(has_suffix), id.len())
    });
    // nodes is non-empty, so min_by_key always returns Some.
    best.ok_or(DedupError::EmptyGroup)
}

// ── LSH blocking ─────────────────────────────────────────────────────────────

/// Naive LSH: for each candidate pair whose Jaccard estimate >= `threshold`,
/// yield `(i, j)`. O(n²) on candidates that pass the entropy gate; in practice
/// the candidate list is much smaller than the full node list.
///
/// Returns index pairs `(i, j)` from `minhashes` whose estimated Jaccard
/// similarity meets `threshold`. The outer loop is parallelised via Rayon
/// when the candidate set is large enough; pairs are then sorted into the
/// same `(pos_a, pos_b)` order the sequential implementation would emit,
/// preserving downstream determinism in `pass2_fuzzy`'s union-find.
fn lsh_pairs(minhashes: &[(usize, MinHash)], threshold: f64) -> Vec<(usize, usize)> {
    let scan_row = |pos_a: usize| -> Vec<(usize, usize, usize, usize)> {
        let (idx_a, mh_a) = &minhashes[pos_a];
        let mut row = Vec::new();
        for (pos_b, (idx_b, mh_b)) in minhashes.iter().enumerate().skip(pos_a + 1) {
            if mh_a.jaccard(mh_b) >= threshold {
                row.push((pos_a, pos_b, *idx_a, *idx_b));
            }
        }
        row
    };

    let mut rows: Vec<(usize, usize, usize, usize)> = if minhashes.len() >= PARALLEL_DEDUP_THRESHOLD
    {
        (0..minhashes.len())
            .into_par_iter()
            .flat_map_iter(scan_row)
            .collect()
    } else {
        (0..minhashes.len()).flat_map(scan_row).collect()
    };

    // Sort by (pos_a, pos_b) so the order matches the sequential implementation;
    // downstream union-find depends on stable pair ordering for determinism.
    rows.sort_unstable_by_key(|(pos_a, pos_b, _, _)| (*pos_a, *pos_b));
    rows.into_iter().map(|(_, _, ia, ib)| (ia, ib)).collect()
}

// ── LLM tiebreaker ────────────────────────────────────────────────────────────

/// Score range sent to LLM for disambiguation.
const LLM_LOW: f64 = 75.0;
const LLM_HIGH: f64 = 92.0;

/// Runs LLM-assisted disambiguation on `candidates`, unioning pairs in the 75–92 score band that the backend confirms as duplicates.
pub fn llm_tiebreak(
    candidates: &[&Value],
    uf: &mut UnionFind,
    communities: &IndexMap<String, i64>,
    backend: &dyn DedupLlmBackend,
) {
    for (i, node_a) in candidates.iter().enumerate() {
        let id_a = node_a.get("id").and_then(Value::as_str).unwrap_or("");
        let norm_a = norm(node_a.get("label").and_then(Value::as_str).unwrap_or(id_a));
        for node_b in candidates.iter().skip(i + 1) {
            consider_tiebreak_pair(uf, communities, backend, node_a, id_a, &norm_a, node_b);
        }
    }
}

/// Evaluate one candidate pair: bail if blocked, ask the LLM if the score lies in
/// the tiebreak band, and union them on a positive verdict.
fn consider_tiebreak_pair(
    uf: &mut UnionFind,
    communities: &IndexMap<String, i64>,
    backend: &dyn DedupLlmBackend,
    node_a: &Value,
    id_a: &str,
    norm_a: &str,
    node_b: &Value,
) {
    let id_b = node_b.get("id").and_then(Value::as_str).unwrap_or("");
    if uf.find(id_a) == uf.find(id_b) {
        return;
    }
    let norm_b = norm(node_b.get("label").and_then(Value::as_str).unwrap_or(id_b));
    // Mirror pass 2: plain Jaro for cross-file long labels (#1243). Jaro-Winkler's
    // leading-prefix bonus would otherwise lift shared-prefix/token-divergent
    // pairs past threshold; on plain Jaro they fall short while true cross-file
    // duplicates still clear it. Same-file and short pairs keep Jaro-Winkler.
    let xfile = source_file(node_a) != source_file(node_b);
    let mut score = if xfile && norm_a.len().max(norm_b.len()) >= 12 {
        jaro_score(norm_a, &norm_b)
    } else {
        jaro_winkler_score(norm_a, &norm_b)
    };
    if is_variant_pair(norm_a, &norm_b) {
        return;
    }
    if short_label_blocked(norm_a, &norm_b, score) {
        return;
    }
    if is_prefix_extension(norm_a, &norm_b) {
        return;
    }
    // Mirror pass 2: numbered/versioned siblings and cross-file file-anchored
    // boilerplate (rationale/document) are decisively distinct, never reach the
    // LLM (#1284).
    if numeric_tokens_differ(norm_a, &norm_b) {
        return;
    }
    if crossfile_fileanchored_blocked(node_a, node_b) {
        return;
    }
    let c1 = communities.get(id_a).copied();
    let c2 = communities.get(id_b).copied();
    if c1.is_some() && c2.is_some() && c1 == c2 && norm_a.len().min(norm_b.len()) >= 12 {
        score += COMMUNITY_BOOST;
    }
    if (LLM_LOW..LLM_HIGH).contains(&score)
        && let JudgeResult::Merge = backend.judge(norm_a, &norm_b)
    {
        // Reuse `pick_winner` so the chunk-suffix preference matches the
        // other merge paths instead of using a shorter-id-only fallback.
        let pair: [&Value; 2] = [node_a, node_b];
        let Ok(winner) = pick_winner(&pair) else {
            return;
        };
        let winner_id = winner
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        uf.union(&winner_id, id_a);
        uf.union(&winner_id, id_b);
    }
}

// ── helpers used by `run` ─────────────────────────────────────────────────────

/// Extracts the `"label"` field from a node value, falling back to `"id"` if absent.
fn node_label(node: &Value) -> &str {
    node.get("label")
        .and_then(Value::as_str)
        .or_else(|| node.get("id").and_then(Value::as_str))
        .unwrap_or("")
}

/// Extracts the `"id"` field from a node value, returning an empty string if absent.
fn node_id(node: &Value) -> &str {
    node.get("id").and_then(Value::as_str).unwrap_or("")
}

/// True when one normalised label is a strict prefix of the other
/// (`getActiveSession` / `getActiveSessions`). Such pairs are almost never
/// duplicates and must be blocked regardless of Jaro-Winkler score (#1201).
fn is_prefix_extension(a: &str, b: &str) -> bool {
    let (lo, hi) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    hi.starts_with(lo) && hi != lo
}

/// True for AST-extracted code symbols.
///
/// Code-node identity is the node ID (which already encodes the fully qualified
/// module/class/symbol path). The label is only a display name — a bare
/// `.draw()` method name, or a function name shared by two parallel backends —
/// so label-based merging conflates distinct symbols (#1205). Genuine duplicates
/// (the same symbol re-extracted) share an ID and are already collapsed by the
/// exact-ID pre-dedup, so code never needs label-based merging.
fn is_code(node: &Value) -> bool {
    node.get("file_type").and_then(Value::as_str) == Some("code")
}

/// `file_type` values whose identity is anchored to their source location, not
/// their label text. Like code (#1205), these must not be label-merged across
/// files: `rationale` = module/class docstrings, `document` =
/// headings/positional content. `concept` is intentionally excluded — it is the
/// type meant to unify across files (protected from over-merge by the
/// numeric/Jaro guards instead). Mirrors `_FILE_ANCHORED_NONCODE`.
const FILE_ANCHORED_NONCODE: [&str; 2] = ["rationale", "document"];

/// True when `node`'s `file_type` is one of [`FILE_ANCHORED_NONCODE`].
fn is_file_anchored_noncode(node: &Value) -> bool {
    node.get("file_type")
        .and_then(Value::as_str)
        .is_some_and(|ft| FILE_ANCHORED_NONCODE.contains(&ft))
}

/// Extract a node's `source_file`, treating an absent or non-string value as
/// the empty string (mirrors Python's `node.get("source_file") or ""`).
fn source_file(node: &Value) -> &str {
    node.get("source_file")
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// Block label-based merging of file-anchored non-code nodes across files (#1284).
///
/// `rationale`/`document` nodes are docstring- and heading-derived and as
/// file-anchored as the code they describe (#1205's reasoning, one layer up):
/// parallel modules carry near-identical boilerplate ("Django app config for
/// apps.<name>. No business logic here…") that differs by one word and sails
/// past the JW threshold. Returns `true` only when at least one node is
/// file-anchored AND the two live in different source files; same-file
/// duplicates of these types may still merge. Mirrors
/// `_crossfile_fileanchored_blocked`.
fn crossfile_fileanchored_blocked(node: &Value, neighbor: &Value) -> bool {
    if !is_file_anchored_noncode(node) && !is_file_anchored_noncode(neighbor) {
        return false;
    }
    source_file(node) != source_file(neighbor)
}

// ── pass 1: exact normalisation ───────────────────────────────────────────────

/// Groups nodes with identical normalised labels within the same source file and unions them in the returned `UnionFind`.
fn pass1_exact(unique_nodes: &[&Value]) -> Result<UnionFind, DedupError> {
    // Map norm(label) -> indices into unique_nodes.
    let mut norm_to_idx: IndexMap<String, Vec<usize>> = IndexMap::new();
    for (i, node) in unique_nodes.iter().enumerate() {
        // Code symbols are keyed by ID, never by label — skip them entirely so
        // distinct same-named symbols are never merged by string similarity (#1205).
        if is_code(node) {
            continue;
        }
        let key = norm(node_label(node));
        if !key.is_empty() {
            norm_to_idx.entry(key).or_default().push(i);
        }
    }

    let mut uf = UnionFind::new();

    for indices in norm_to_idx.values() {
        if indices.len() <= 1 {
            continue;
        }
        // Partition by source_file.
        let mut by_file: IndexMap<String, Vec<&Value>> = IndexMap::new();
        for &idx in indices {
            let sf = unique_nodes[idx]
                .get("source_file")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            by_file.entry(sf).or_default().push(unique_nodes[idx]);
        }
        for (sf, file_group) in &by_file {
            // No source_file — cannot prove same symbol; skip to avoid collapsing
            // distinct nodes that happen to share a label (#1178).
            if sf.is_empty() {
                continue;
            }
            if file_group.len() > 1 {
                let winner = pick_winner(file_group)?;
                let winner_id = node_id(winner);
                for node in file_group {
                    uf.union(winner_id, node_id(node));
                }
            }
        }
    }

    Ok(uf)
}

// ── pass 2: fuzzy matching ────────────────────────────────────────────────────

/// Performs MinHash/LSH blocking followed by Jaro-Winkler verification to union near-duplicate node pairs.
fn pass2_fuzzy(
    unique_nodes: &[&Value],
    uf: &mut UnionFind,
    communities: &IndexMap<String, i64>,
) -> Result<(), DedupError> {
    let mut seen_norms: indexmap::IndexSet<String> = indexmap::IndexSet::new();
    let mut candidates: Vec<&Value> = Vec::new();
    for node in unique_nodes {
        // Exclude code symbols from fuzzy matching: two functions with similar
        // long names in different files (parallel backends, sibling classes)
        // must not be fuzzy-merged, and a code<->concept fuzzy match must not
        // transitively union two distinct code symbols via a concept (#1205).
        if is_code(node) {
            continue;
        }
        let label = node_label(node);
        let key = norm(label);
        if !key.is_empty() && seen_norms.insert(key) && entropy(label) >= ENTROPY_THRESHOLD {
            candidates.push(node);
        }
    }

    if candidates.len() < 2 {
        return Ok(());
    }

    // `make_minhash` is pure, dominated by hashing — fan out across Rayon
    // when the candidate set is large enough to amortise pool overhead.
    let minhashes: Vec<(usize, MinHash)> = if candidates.len() >= PARALLEL_DEDUP_THRESHOLD {
        candidates
            .par_iter()
            .enumerate()
            .map(|(i, node)| (i, make_minhash(&norm(node_label(node)))))
            .collect()
    } else {
        candidates
            .iter()
            .enumerate()
            .map(|(i, node)| (i, make_minhash(&norm(node_label(node)))))
            .collect()
    };

    let pairs = lsh_pairs(&minhashes, LSH_THRESHOLD);

    for (idx_a, idx_b) in pairs {
        let node_a = candidates[idx_a];
        let node_b = candidates[idx_b];
        let id_a = node_id(node_a);
        let id_b = node_id(node_b);

        if uf.find(id_a) == uf.find(id_b) {
            continue;
        }

        let norm_a = norm(node_label(node_a));
        let norm_b = norm(node_label(node_b));
        // Cross-file long labels score on plain Jaro (no prefix bonus). Jaro-Winkler's
        // leading-prefix bonus lifts shared-prefix/token-divergent pairs past
        // threshold, fabricating destructive cross-file merges; plain Jaro drops
        // them while true cross-file duplicates still clear it (#1243). Same-file
        // near-duplicates and short labels keep Jaro-Winkler.
        let xfile = source_file(node_a) != source_file(node_b);
        let mut score = if xfile && norm_a.len().max(norm_b.len()) >= 12 {
            jaro_score(&norm_a, &norm_b)
        } else {
            jaro_winkler_score(&norm_a, &norm_b)
        };

        if is_variant_pair(&norm_a, &norm_b) {
            continue;
        }
        if short_label_blocked(&norm_a, &norm_b, score) {
            continue;
        }
        // Prefix-extension pairs (getActiveSession / getActiveSessions,
        // parseConfig / parseConfigFile) are almost never duplicates — one is a
        // strict suffix-extension of the other. Block regardless of JW score (#1201).
        if is_prefix_extension(&norm_a, &norm_b) {
            continue;
        }
        // Numbered/versioned siblings and cross-file file-anchored boilerplate
        // (rationale/document) are decisively distinct regardless of score (#1284).
        if numeric_tokens_differ(&norm_a, &norm_b) {
            continue;
        }
        if crossfile_fileanchored_blocked(node_a, node_b) {
            continue;
        }

        let c1 = communities.get(id_a).copied();
        let c2 = communities.get(id_b).copied();
        if c1.is_some() && c2.is_some() && c1 == c2 && norm_a.len().min(norm_b.len()) >= 12 {
            score += COMMUNITY_BOOST;
        }

        if score >= MERGE_THRESHOLD {
            // Identical labels across different source files almost always mean
            // same-named-but-different symbols (trait impls, wrapper methods,
            // common type names). Mirror Pass 1's source_file partition for this
            // sub-case (graphify-py #1046). NB: the unique-by-norm `candidates`
            // construction above means two compared candidates never share a
            // norm, so this guard is currently unreachable — it is retained for
            // parity with graphify-py and as insurance should that invariant
            // ever change.
            if norm_a == norm_b {
                let sf_a = node_a
                    .get("source_file")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let sf_b = node_b
                    .get("source_file")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if sf_a != sf_b {
                    continue;
                }
            }
            // Pick the winner from the verified pair only. Selecting it from the
            // union of both normalized-label groups pulls never-compared nodes
            // (same label, different source_file) into the merge, bypassing the
            // #1046/#1178 guards (#1247).
            let pair = [node_a, node_b];
            let winner = pick_winner(&pair)?;
            let winner_id = node_id(winner);
            uf.union(winner_id, id_a);
            uf.union(winner_id, id_b);
        }
    }

    Ok(())
}

// ── main dedup logic ─────────────────────────────────────────────────────────

/// Run the three-pass entity deduplication pipeline.
///
/// See [`crate::deduplicate_entities`] for the public interface.
///
/// # Errors
///
/// Returns [`DedupError::MultipleRepos`] if nodes span more than one repo.
/// Returns [`DedupError::EmptyGroup`] if an internal winner selection receives an empty group.
pub fn run(
    nodes: &[Value],
    edges: &[Value],
    communities: &IndexMap<String, i64>,
    dedup_llm_backend: Option<&dyn DedupLlmBackend>,
) -> Result<(Vec<Value>, Vec<Value>), DedupError> {
    // Guard: cross-project dedup is not supported.
    let repos: indexmap::IndexSet<String> = nodes
        .iter()
        .filter_map(|n| n.get("repo").and_then(Value::as_str).map(str::to_string))
        .collect();
    if repos.len() > 1 {
        let mut sorted: Vec<&str> = repos.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        return Err(DedupError::MultipleRepos(sorted.join(", ")));
    }

    if nodes.len() <= 1 {
        return Ok((nodes.to_vec(), edges.to_vec()));
    }

    // Pre-dedup: keep first occurrence of each id.
    let mut seen_ids: IndexMap<String, &Value> = IndexMap::new();
    for node in nodes {
        let nid = node_id(node);
        if !nid.is_empty() {
            seen_ids.entry(nid.to_string()).or_insert(node);
        }
    }
    let unique_nodes: Vec<&Value> = seen_ids.values().copied().collect();

    if unique_nodes.len() <= 1 {
        let owned: Vec<Value> = unique_nodes.iter().map(|v| (*v).clone()).collect();
        return Ok((owned, edges.to_vec()));
    }

    // Pass 1: exact normalisation.
    let mut uf = pass1_exact(&unique_nodes)?;

    // Pass 2: MinHash/LSH + Jaro-Winkler.
    pass2_fuzzy(&unique_nodes, &mut uf, communities)?;

    // Pass 3: LLM tiebreaker (opt-in).
    if let Some(backend) = dedup_llm_backend {
        // Re-build candidate list (same logic as pass2_fuzzy but without minhash).
        let mut seen_norms: indexmap::IndexSet<String> = indexmap::IndexSet::new();
        let candidates: Vec<&Value> = unique_nodes
            .iter()
            .filter(|node| {
                let key = norm(node_label(node));
                !key.is_empty()
                    && seen_norms.insert(key)
                    && entropy(node_label(node)) >= ENTROPY_THRESHOLD
            })
            .copied()
            .collect();
        llm_tiebreak(&candidates, &mut uf, communities, backend);
    }

    // Build remap from union-find components.
    let components = uf.components();
    let mut remap: IndexMap<String, String> = IndexMap::new();

    for members in components.values() {
        if members.len() == 1 {
            continue;
        }
        let group_nodes: Vec<&Value> = unique_nodes
            .iter()
            .filter(|n| {
                n.get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| members.contains(&id.to_string()))
            })
            .copied()
            .collect();
        let winner_id = if group_nodes.is_empty() {
            members.first().cloned().unwrap_or_default()
        } else {
            pick_winner(&group_nodes)?
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        for member in members {
            if *member != winner_id {
                remap.insert(member.clone(), winner_id.clone());
            }
        }
    }

    // Apply remap.
    if remap.is_empty() {
        let owned: Vec<Value> = unique_nodes.iter().map(|v| (*v).clone()).collect();
        return Ok((owned, edges.to_vec()));
    }

    let deduped_nodes: Vec<Value> = unique_nodes
        .iter()
        .filter(|n| {
            n.get("id")
                .and_then(Value::as_str)
                .is_none_or(|id| !remap.contains_key(id))
        })
        .map(|n| (*n).clone())
        .collect();

    let deduped_edges = rewrite_edges(edges, &remap);

    Ok((deduped_nodes, deduped_edges))
}

// ── edge rewriting ────────────────────────────────────────────────────────────

/// Rewrites edge `source`/`target` fields according to `remap`, dropping self-loops produced by merges.
fn rewrite_edges(edges: &[Value], remap: &IndexMap<String, String>) -> Vec<Value> {
    let mut out = Vec::with_capacity(edges.len());
    for edge in edges {
        let Some(map) = edge.as_object() else {
            out.push(edge.clone());
            continue;
        };

        // Tolerate "from"/"to" keys alongside "source"/"target" (#803).
        let src = if map.contains_key("source") {
            map.get("source")
                .and_then(Value::as_str)
                .map(str::to_string)
        } else {
            map.get("from").and_then(Value::as_str).map(str::to_string)
        };
        let tgt = if map.contains_key("target") {
            map.get("target")
                .and_then(Value::as_str)
                .map(str::to_string)
        } else {
            map.get("to").and_then(Value::as_str).map(str::to_string)
        };

        let (Some(src), Some(tgt)) = (src, tgt) else {
            continue;
        };

        let new_src = remap.get(&src).cloned().unwrap_or(src);
        let new_tgt = remap.get(&tgt).cloned().unwrap_or(tgt);

        if new_src == new_tgt {
            continue; // self-loop after merge — drop.
        }

        let mut new_edge = map.clone();
        new_edge.insert("source".to_string(), Value::String(new_src));
        new_edge.insert("target".to_string(), Value::String(new_tgt));
        new_edge.remove("from");
        new_edge.remove("to");
        out.push(Value::Object(new_edge));
    }
    out
}
