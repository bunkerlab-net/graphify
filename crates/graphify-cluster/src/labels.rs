//! Deterministic, LLM-free community labeling and membership fingerprints.
//!
//! Ports `label_communities_by_hub` and `community_member_sigs` from
//! `graphify-py/graphify/cluster.py`.

use graphify_build::Graph;
use indexmap::IndexMap;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Per-node degree over the full graph (edge incidence; a self-loop counts
/// twice, matching `NetworkX` `Graph.degree`).
fn degree_map(graph: &Graph) -> IndexMap<&str, usize> {
    let mut degrees: IndexMap<&str, usize> = IndexMap::with_capacity(graph.node_count());
    for edge in graph.edges() {
        *degrees.entry(edge.source.as_str()).or_insert(0) += 1;
        *degrees.entry(edge.target.as_str()).or_insert(0) += 1;
    }
    degrees
}

/// Deterministic, LLM-free community labels: name each community after its
/// highest-degree member — the structural hub — so a report reads `auth` /
/// `log_action` instead of `Community 70`.
///
/// Degree is measured on the full graph `graph`; ties break by node id
/// (ascending) for run-to-run stability. A community whose members are all
/// absent from the graph falls back to `Community {cid}`, as does one whose hub
/// has an empty (or `()`-only) label.
///
/// Used as the default (no-backend) labeler; a configured LLM naming pass
/// overrides these with richer names. Community-id iteration order is preserved.
#[must_use]
pub fn label_communities_by_hub(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
) -> IndexMap<i64, String> {
    let degrees = degree_map(graph);
    let mut labels: IndexMap<i64, String> = IndexMap::with_capacity(communities.len());
    for (&cid, members) in communities {
        // highest degree wins; ties broken by node id (ascending) for determinism.
        let hub = members
            .iter()
            .filter(|m| graph.node_data(m.as_str()).is_some())
            .min_by(|a, b| {
                let da = degrees.get(a.as_str()).copied().unwrap_or(0);
                let db = degrees.get(b.as_str()).copied().unwrap_or(0);
                db.cmp(&da).then_with(|| a.cmp(b))
            });
        let Some(hub) = hub else {
            labels.insert(cid, format!("Community {cid}"));
            continue;
        };
        let raw = graph
            .node_data(hub.as_str())
            .and_then(|attrs| attrs.get("label"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(hub);
        let mut name = raw.trim();
        if let Some(stripped) = name.strip_suffix("()") {
            name = stripped;
        }
        let label = if name.is_empty() {
            format!("Community {cid}")
        } else {
            name.to_string()
        };
        labels.insert(cid, label);
    }
    labels
}

/// Per-community membership fingerprints: `{cid: sha256(sorted member ids)}`,
/// truncated to the first 16 hex chars.
///
/// Persisted next to `.graphify_labels.json` so a later `cluster-only` can tell
/// which communities actually changed since labeling — a cid whose members no
/// longer hash the same is a different community, and reusing its old (LLM)
/// label there is the "stale label after re-scoping" bug this guards against.
/// Deterministic and independent of cid index, node order, and machine.
#[must_use]
pub fn community_member_sigs(communities: &IndexMap<i64, Vec<String>>) -> IndexMap<i64, String> {
    let mut sigs: IndexMap<i64, String> = IndexMap::with_capacity(communities.len());
    for (&cid, members) in communities {
        let mut sorted: Vec<&str> = members.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        let mut hasher = Sha256::new();
        for nid in sorted {
            hasher.update(nid.as_bytes());
            hasher.update([0u8]);
        }
        let digest = hex::encode(hasher.finalize());
        sigs.insert(cid, digest[..16].to_string());
    }
    sigs
}
