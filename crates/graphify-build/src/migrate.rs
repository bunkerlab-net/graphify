//! Node-ID migration for the #1504/#1509 full-path stem rule.
//!
//! The node-ID stem is now the full repo-relative path (`docs/v1/api/README.md`
//! → `docs_v1_api_readme`) instead of just the immediate parent dir + filename
//! (`api_readme`). The semantic cache is **unversioned**, so a cached/LLM
//! fragment can still carry a pre-migration short id. [`apply_semantic_rekey`]
//! deterministically re-derives every non-AST node's id from its own
//! `source_file` so a drifted fragment reconciles with the AST node instead of
//! spawning a ghost / a re-bill, and [`register_legacy_id_aliases`] registers
//! the old-stem forms as edge-resolution aliases so a stale edge endpoint from
//! an un-re-keyed fragment still resolves to the migrated node.
//!
//! `graphify-build` is the dependency leaf (`graphify-extract` depends on
//! [`normalize_id`]), so the canonical `file_stem`/`make_id` in
//! `graphify-extract::ids` cannot be imported here. The two tiny helpers below
//! recompute the same recipe on top of [`normalize_id`]; the observable node
//! IDs they produce are pinned by the build/extract parity tests, so any drift
//! from `extract::ids` fails loudly rather than silently.

use std::collections::HashSet;
use std::path::Path;

use indexmap::IndexMap;
use serde_json::Value;

use crate::normalize::{norm_source_file, normalize_id};

/// Only file-level nodes (`source_location == "L1"`) are sampled by
/// [`graph_has_legacy_ids`]; cap the scan so a huge graph stays cheap.
const LEGACY_ID_SAMPLE: usize = 300;

/// Build a stable node ID from one or more name parts — mirrors
/// `graphify-extract::ids::make_id`.
#[must_use]
fn make_id(parts: &[&str]) -> String {
    let combined = parts
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| p.trim_matches(|c| c == '_' || c == '.'))
        .collect::<Vec<_>>()
        .join("_");
    normalize_id(&combined)
}

/// Single-part [`make_id`].
#[must_use]
fn make_id1(part: &str) -> String {
    make_id(&[part])
}

/// Full repo-relative path with the extension dropped, POSIX form — mirrors
/// `graphify-extract::ids::file_stem`. `make_id` collapses the separators later.
#[must_use]
fn file_stem(path: &Path) -> String {
    // No file name (`Path(".")` — a `source_file` equal to the scan root) → no
    // per-file stem; return "" so the caller leaves the id untouched (#1618).
    if path.file_name().is_none() {
        return String::new();
    }
    path.with_extension("").to_string_lossy().replace('\\', "/")
}

/// Pre-migration stem forms a semantic fragment may have used for `rel`,
/// ordered longest-first so prefix stripping is greedy and unambiguous:
/// the one-parent form (`parent.stem`, the old `_file_stem` rule) then the
/// zero-parent form (`stem`, the old llm-prompt rule, #1509). Top-level files
/// collapse both forms to one.
#[must_use]
fn old_file_stems(rel: &Path) -> Vec<String> {
    let parent = rel
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = rel
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut forms: Vec<String> = Vec::new();
    if !parent.is_empty() && parent != "." {
        forms.push(make_id1(&format!("{parent}.{stem}")));
    }
    forms.push(make_id1(&stem));
    let mut seen: HashSet<String> = HashSet::new();
    forms
        .into_iter()
        .filter(|f| !f.is_empty() && seen.insert(f.clone()))
        .collect()
}

/// Normalised `source_file` of a node, falling back to the raw value when
/// relativisation yields nothing. `None` when the field is missing/empty.
fn node_rel_source(map: &serde_json::Map<String, Value>, root: Option<&str>) -> Option<String> {
    let sf = map.get("source_file").and_then(Value::as_str)?;
    if sf.is_empty() {
        return None;
    }
    let norm = norm_source_file(sf, root);
    Some(if norm.is_empty() {
        sf.to_string()
    } else {
        norm
    })
}

/// Re-derive non-AST node ids from `source_file` using the canonical full-path
/// stem, so a cached/LLM fragment carrying a pre-migration short id reconciles
/// with the AST node instead of spawning a ghost (#1504/#1509).
///
/// Drift-proof by construction: the new id is computed from `source_file` in
/// code, never trusted from the fragment's own id. AST-origin nodes
/// (`_origin == "ast"`) already carry canonical ids and are skipped. Returns a
/// map of old id → new id.
#[must_use]
fn semantic_id_remap(nodes: &[Value], root: Option<&str>) -> IndexMap<String, String> {
    let mut remap: IndexMap<String, String> = IndexMap::new();
    for node in nodes {
        let Some(map) = node.as_object() else {
            continue;
        };
        if map.get("_origin").and_then(Value::as_str) == Some("ast") {
            continue;
        }
        let Some(nid) = map.get("id").and_then(Value::as_str) else {
            continue;
        };
        if nid.is_empty() {
            continue;
        }
        let Some(sf_norm) = node_rel_source(map, root) else {
            continue;
        };
        let rel = Path::new(&sf_norm);
        if rel.is_absolute() {
            continue; // can't relativize (no/failed root) — leave id untouched
        }
        let new_stem = make_id1(&file_stem(rel));
        if new_stem.is_empty() {
            continue;
        }
        let norm_nid = normalize_id(nid);
        let mut new_id: Option<String> = None;
        for old_stem in old_file_stems(rel) {
            if old_stem == new_stem {
                continue; // already canonical for this form
            }
            if norm_nid == old_stem {
                new_id = Some(new_stem.clone()); // the file node itself
                break;
            }
            if let Some(entity) = norm_nid.strip_prefix(&format!("{old_stem}_")) {
                new_id = Some(make_id(&[&new_stem, entity]));
                break;
            }
        }
        if let Some(new_id) = new_id
            && new_id != nid
        {
            remap.insert(nid.to_string(), new_id);
        }
    }
    remap
}

/// Re-key cached/LLM fragment ids onto the new full-path-stem form in place,
/// rewriting node ids, edge `source`/`target`, and hyperedge node lists
/// (#1504/#1509). No-op when nothing drifted.
pub(crate) fn apply_semantic_rekey(extraction: &mut Value, root: Option<&str>) {
    let remap = {
        let nodes = extraction
            .as_object()
            .and_then(|o| o.get("nodes"))
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        semantic_id_remap(nodes, root)
    };
    if remap.is_empty() {
        return;
    }
    let Some(obj) = extraction.as_object_mut() else {
        return;
    };
    if let Some(arr) = obj.get_mut("nodes").and_then(Value::as_array_mut) {
        for node in arr.iter_mut() {
            if let Some(map) = node.as_object_mut()
                && let Some(new_id) = map
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|id| remap.get(id))
            {
                let new_id = Value::String(new_id.clone());
                map.insert("id".to_string(), new_id);
            }
        }
    }
    if let Some(arr) = obj.get_mut("edges").and_then(Value::as_array_mut) {
        for edge in arr.iter_mut() {
            let Some(map) = edge.as_object_mut() else {
                continue;
            };
            for key in ["source", "target"] {
                if let Some(new_id) = map
                    .get(key)
                    .and_then(Value::as_str)
                    .and_then(|v| remap.get(v))
                {
                    let new_id = Value::String(new_id.clone());
                    map.insert(key.to_string(), new_id);
                }
            }
        }
    }
    if let Some(arr) = obj.get_mut("hyperedges").and_then(Value::as_array_mut) {
        for he in arr.iter_mut() {
            if let Some(ns) = he
                .as_object_mut()
                .and_then(|m| m.get_mut("nodes"))
                .and_then(Value::as_array_mut)
            {
                for n in ns.iter_mut() {
                    if let Some(new_id) = n.as_str().and_then(|s| remap.get(s)) {
                        *n = Value::String(new_id.clone());
                    }
                }
            }
        }
    }
}

/// The normalised legacy-stem aliases node `nid` (in root-relative file `sf`,
/// labelled `label`) would claim, or empty when it claims none.
///
/// "This node IS the file" is detected by `label == basename`, not an id-prefix
/// test: a salted `.h`/`.cpp` pair (#1556) no longer string-prefixes its own
/// `new_stem`, so the old id test dropped the salted header from the alias race
/// and an unrelated same-stem file won as "unambiguous" (96db75c).
fn claimed_aliases(nid: &str, sf: &str, label: Option<&str>) -> Vec<String> {
    if sf.is_empty() {
        return Vec::new();
    }
    let rel = Path::new(sf);
    if rel.is_absolute() {
        return Vec::new();
    }
    let new_stem = make_id1(&file_stem(rel));
    let is_file_node = rel
        .file_name()
        .is_some_and(|n| Some(n.to_string_lossy().as_ref()) == label);
    let suffix = if is_file_node {
        String::new()
    } else {
        let norm_nid = normalize_id(nid);
        let Some(suffix) = norm_nid.strip_prefix(&new_stem) else {
            // `nid` isn't derived from this file's stem (e.g. a disambiguated id):
            // an empty-suffix fallback would map unrelated edges onto this node.
            return Vec::new();
        };
        if !suffix.is_empty() && !suffix.starts_with('_') {
            return Vec::new();
        }
        suffix.to_string()
    };
    old_file_stems(rel)
        .into_iter()
        .filter(|old_stem| *old_stem != new_stem)
        .map(|old_stem| normalize_id(&format!("{old_stem}{suffix}")))
        .collect()
}

/// Register each canonical node's OLD-stem id forms as edge-resolution aliases,
/// so a stale-id edge endpoint from an un-re-keyed fragment (e.g. an incremental
/// update referencing a symbol in a file that was NOT re-extracted) still
/// resolves to the migrated node instead of dangling (#1504). Only fills gaps —
/// never overrides a real node id. An alias claimed by more than one file is
/// dropped (ambiguous), so a dangling edge stays dangling instead of riding the
/// alias onto an arbitrary same-named file (3b2ca2e).
pub(crate) fn register_legacy_id_aliases(
    norm_to_id: &mut IndexMap<String, String>,
    node_source_files: &IndexMap<String, String>,
    node_labels: &IndexMap<String, String>,
) {
    let mut candidates: IndexMap<String, indexmap::IndexSet<String>> = IndexMap::new();
    for (nid, sf) in node_source_files {
        for alias in claimed_aliases(nid, sf, node_labels.get(nid).map(String::as_str)) {
            candidates.entry(alias).or_default().insert(nid.clone());
        }
    }
    for (alias, claimants) in &candidates {
        if claimants.len() == 1
            && let Some(nid) = claimants.iter().next()
        {
            norm_to_id
                .entry(alias.clone())
                .or_insert_with(|| nid.clone());
        }
    }
}

/// Whether a loaded graph still uses pre-#1504 node IDs (parent-dir / filename
/// stem) rather than the full repo-relative path. Read-only consumers (query,
/// serve) use this to nudge the user to rebuild, since they don't re-extract.
///
/// Heuristic and cheap: only **file-level** nodes (`source_location == "L1"`)
/// are inspected, because their ID is unambiguously the file stem. Returns
/// `true` as soon as one file node's ID matches an OLD stem form but not the
/// canonical full-path form.
#[must_use]
pub fn graph_has_legacy_ids(nodes: &[Value], root: Option<&str>) -> bool {
    let mut checked = 0usize;
    for node in nodes {
        let Some(map) = node.as_object() else {
            continue;
        };
        if map
            .get("source_location")
            .and_then(Value::as_str)
            .unwrap_or_default()
            != "L1"
        {
            continue; // only file-level nodes carry an unambiguous file-stem ID
        }
        let Some(nid) = map.get("id").and_then(Value::as_str) else {
            continue;
        };
        if nid.is_empty() {
            continue;
        }
        let Some(sf_norm) = node_rel_source(map, root) else {
            continue;
        };
        let rel = Path::new(&sf_norm);
        if rel.is_absolute() {
            continue;
        }
        let new_stem = make_id1(&file_stem(rel));
        if new_stem.is_empty() {
            continue;
        }
        let norm = normalize_id(nid);
        if norm != new_stem && !norm.starts_with(&format!("{new_stem}_")) {
            for old in old_file_stems(rel) {
                if old != new_stem && (norm == old || norm.starts_with(&format!("{old}_"))) {
                    return true;
                }
            }
        }
        checked += 1;
        if checked >= LEGACY_ID_SAMPLE {
            break;
        }
    }
    false
}
