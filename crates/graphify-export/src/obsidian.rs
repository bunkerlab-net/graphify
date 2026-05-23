//! Obsidian vault export — `to_obsidian`.
//!
//! Mirrors Python `to_obsidian` from `graphify-py/graphify/export.py`.
//!
//! Produces one `.md` note per graph node plus one `_COMMUNITY_<name>.md`
//! overview note per community, then writes `.obsidian/graph.json` with
//! community colour groups.

use std::path::Path;

use graphify_build::Graph;
use indexmap::IndexMap;
use rayon::prelude::*;
use regex::Regex;
use serde_json::Value;

use crate::{COMMUNITY_COLORS, ExportError, node_community_map, obsidian_tag, yaml_str};

// ── Filename sanitisation ─────────────────────────────────────────────────────

#[allow(clippy::expect_used)]
static UNSAFE_CHARS_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r#"[\\/*?:"<>|#^\[\]]"#).expect("literal pattern is valid")
});

#[allow(clippy::expect_used)]
static MD_EXT_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"(?i)\.(md|mdx|qmd|markdown)$").expect("literal pattern is valid")
});

/// Sanitize a node label for use as an Obsidian filename.
///
/// Mirrors Python `safe_name` inside `to_obsidian`.
fn safe_name(label: &str) -> String {
    let cleaned = label.replace(['\r', '\n'], " ");
    let cleaned = UNSAFE_CHARS_RE.replace_all(&cleaned, "");
    let cleaned = cleaned.trim().to_string();
    let cleaned = MD_EXT_RE.replace(&cleaned, "").into_owned();
    if cleaned.is_empty() {
        "unnamed".to_string()
    } else {
        cleaned
    }
}

// ── File-type → graphify tag ──────────────────────────────────────────────────

/// Convert a graphify file-type string to its Obsidian tag path.
fn ftype_tag(ftype: &str) -> String {
    match ftype {
        "code" => "graphify/code".to_string(),
        "document" | "" => "graphify/document".to_string(),
        "paper" => "graphify/paper".to_string(),
        "image" => "graphify/image".to_string(),
        other => format!("graphify/{other}"),
    }
}

// ── Dominant confidence for a node ───────────────────────────────────────────

/// Return the most-frequently-occurring confidence string across all edges for `node_id`.
///
/// Falls back to `"EXTRACTED"` when the node has no edges.
fn dominant_confidence(graph: &Graph, node_id: &str) -> String {
    let mut counts: IndexMap<String, usize> = IndexMap::new();
    for edge in graph.edges() {
        if edge.source == node_id || edge.target == node_id {
            let conf = edge
                .attrs
                .get("confidence")
                .and_then(Value::as_str)
                .unwrap_or("EXTRACTED")
                .to_string();
            *counts.entry(conf).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map_or_else(|| "EXTRACTED".to_string(), |(k, _)| k)
}

// ── Community reach ───────────────────────────────────────────────────────────

/// Count the number of distinct communities that `node_id` connects to via edges.
///
/// Used in Obsidian note YAML to indicate cross-community bridging importance.
fn community_reach(graph: &Graph, node_id: &str, node_community: &IndexMap<String, i64>) -> usize {
    let my_cid = node_community.get(node_id).copied();
    let mut other_cids: IndexMap<i64, ()> = IndexMap::new();
    for edge in graph.edges() {
        let nb = if edge.source == node_id {
            Some(edge.target.as_str())
        } else if edge.target == node_id {
            Some(edge.source.as_str())
        } else {
            None
        };
        if let Some(nb_id) = nb
            && let Some(&cid) = node_community.get(nb_id)
            && Some(cid) != my_cid
        {
            other_cids.insert(cid, ());
        }
    }
    other_cids.len()
}

// ── Main export ───────────────────────────────────────────────────────────────

/// Write a single node note to the output directory.
fn write_node_note(
    node_id: &str,
    attrs: &indexmap::IndexMap<String, Value>,
    graph: &Graph,
    node_community: &IndexMap<String, i64>,
    node_filename: &IndexMap<String, String>,
    community_labels: Option<&IndexMap<i64, String>>,
    output_dir: &Path,
) -> Result<(), ExportError> {
    let label = attrs
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or(node_id);
    let cid = node_community.get(node_id).copied();
    let community_name = community_labels
        .and_then(|cl| cid.and_then(|c| cl.get(&c)))
        .cloned()
        .unwrap_or_else(|| {
            cid.map_or_else(
                || "Community None".to_string(),
                |c| format!("Community {c}"),
            )
        });

    let ftype = attrs.get("file_type").and_then(Value::as_str).unwrap_or("");
    let ft_tag = ftype_tag(ftype);
    let dom_conf = dominant_confidence(graph, node_id);
    let conf_tag = format!("graphify/{dom_conf}");
    let comm_tag = format!("community/{}", obsidian_tag(&community_name));
    let node_tags = [ft_tag.as_str(), conf_tag.as_str(), comm_tag.as_str()];

    let mut lines: Vec<String> = Vec::new();

    // YAML frontmatter
    lines.push("---".to_string());
    lines.push(format!(
        "source_file: \"{}\"",
        yaml_str(
            attrs
                .get("source_file")
                .and_then(Value::as_str)
                .unwrap_or("")
        )
    ));
    lines.push(format!("type: \"{}\"", yaml_str(ftype)));
    lines.push(format!("community: \"{}\"", yaml_str(&community_name)));
    if let Some(loc) = attrs.get("source_location").and_then(Value::as_str) {
        lines.push(format!("location: \"{}\"", yaml_str(loc)));
    }
    lines.push("tags:".to_string());
    for tag in &node_tags {
        lines.push(format!("  - {tag}"));
    }
    lines.push("---".to_string());
    lines.push(String::new());
    lines.push(format!("# {label}"));
    lines.push(String::new());

    // Outgoing edges as wikilinks
    let mut neighbor_ids: Vec<&str> = Vec::new();
    for edge in graph.edges() {
        if edge.source == node_id {
            neighbor_ids.push(&edge.target);
        } else if edge.target == node_id {
            neighbor_ids.push(&edge.source);
        }
    }
    neighbor_ids.dedup();
    neighbor_ids.sort_by_key(|nb| {
        graph
            .node_data(nb)
            .and_then(|a| a.get("label"))
            .and_then(Value::as_str)
            .unwrap_or(nb)
    });

    if !neighbor_ids.is_empty() {
        lines.push("## Connections".to_string());
        for nb in &neighbor_ids {
            let edata = graph.edge_data(node_id, nb);
            let neighbor_label = node_filename
                .get(*nb)
                .cloned()
                .unwrap_or_else(|| (*nb).to_string());
            let relation = edata
                .and_then(|d| d.get("relation"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let confidence = edata
                .and_then(|d| d.get("confidence"))
                .and_then(Value::as_str)
                .unwrap_or("EXTRACTED");
            lines.push(format!(
                "- [[{neighbor_label}]] - `{relation}` [{confidence}]"
            ));
        }
        lines.push(String::new());
    }

    // Inline tags at bottom
    let inline_tags: String = node_tags
        .iter()
        .map(|t| format!("#{t}"))
        .collect::<Vec<_>>()
        .join(" ");
    lines.push(inline_tags);

    let fname = format!("{}.md", node_filename[node_id]);
    std::fs::write(output_dir.join(fname), lines.join("\n"))?;
    Ok(())
}

/// Shared graph + per-vault context for community-note writes.
struct CommunityNoteCtx<'a> {
    graph: &'a Graph,
    node_community: &'a IndexMap<String, i64>,
    node_filename: &'a IndexMap<String, String>,
    community_labels: Option<&'a IndexMap<i64, String>>,
    cohesion: Option<&'a IndexMap<i64, f64>>,
    inter_community: &'a IndexMap<i64, IndexMap<i64, usize>>,
    output_dir: &'a Path,
}

/// Write a single community overview note.
#[allow(clippy::too_many_lines)] // long sequential markdown emission; phases share locals.
fn write_community_note(
    ctx: &CommunityNoteCtx<'_>,
    cid: i64,
    members: &[String],
) -> Result<(), ExportError> {
    let CommunityNoteCtx {
        graph,
        node_community,
        node_filename,
        community_labels,
        cohesion,
        inter_community,
        output_dir,
    } = *ctx;
    let community_name = community_labels
        .and_then(|cl| cl.get(&cid))
        .cloned()
        .unwrap_or_else(|| format!("Community {cid}"));
    let n_members = members.len();
    let coh_value = cohesion.and_then(|c| c.get(&cid).copied());

    let mut lines: Vec<String> = Vec::new();

    // YAML frontmatter
    lines.push("---".to_string());
    lines.push("type: community".to_string());
    if let Some(coh) = coh_value {
        lines.push(format!("cohesion: {coh:.2}"));
    }
    lines.push(format!("members: {n_members}"));
    lines.push("---".to_string());
    lines.push(String::new());
    lines.push(format!("# {community_name}"));
    lines.push(String::new());

    // Cohesion summary
    if let Some(coh) = coh_value {
        let cohesion_desc = if coh >= 0.7 {
            "tightly connected"
        } else if coh >= 0.4 {
            "moderately connected"
        } else {
            "loosely connected"
        };
        lines.push(format!("**Cohesion:** {coh:.2} - {cohesion_desc}"));
    }
    lines.push(format!("**Members:** {n_members} nodes"));
    lines.push(String::new());

    // Members section
    lines.push("## Members".to_string());
    let mut sorted_members = members.to_vec();
    sorted_members.sort_by_key(|n| {
        graph
            .node_data(n)
            .and_then(|a| a.get("label"))
            .and_then(Value::as_str)
            .map_or_else(|| n.clone(), str::to_string)
    });
    for node_id in &sorted_members {
        let data = graph.node_data(node_id);
        let node_label = node_filename
            .get(node_id)
            .cloned()
            .unwrap_or_else(|| node_id.clone());
        let ftype = data
            .and_then(|a| a.get("file_type"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let source = data
            .and_then(|a| a.get("source_file"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut entry = format!("- [[{node_label}]]");
        if !ftype.is_empty() {
            entry.push(' ');
            entry.push('-');
            entry.push(' ');
            entry.push_str(ftype);
        }
        if !source.is_empty() {
            entry.push(' ');
            entry.push('-');
            entry.push(' ');
            entry.push_str(source);
        }
        lines.push(entry);
    }
    lines.push(String::new());

    // Dataview live query
    let comm_tag_name = obsidian_tag(&community_name);
    lines.push("## Live Query (requires Dataview plugin)".to_string());
    lines.push(String::new());
    lines.push("```dataview".to_string());
    lines.push(format!(
        "TABLE source_file, type FROM #community/{comm_tag_name}"
    ));
    lines.push("SORT file.name ASC".to_string());
    lines.push("```".to_string());
    lines.push(String::new());

    // Connections to other communities
    if let Some(cross) = inter_community.get(&cid)
        && !cross.is_empty()
    {
        lines.push("## Connections to other communities".to_string());
        let mut cross_sorted: Vec<(i64, usize)> = cross.iter().map(|(&k, &v)| (k, v)).collect();
        cross_sorted.sort_by_key(|&(_, v)| std::cmp::Reverse(v));
        for (other_cid, edge_count) in cross_sorted {
            let other_name = community_labels
                .and_then(|cl| cl.get(&other_cid))
                .cloned()
                .unwrap_or_else(|| format!("Community {other_cid}"));
            let other_safe = safe_name(&other_name);
            let s = if edge_count == 1 { "" } else { "s" };
            lines.push(format!(
                "- {edge_count} edge{s} to [[_COMMUNITY_{other_safe}]]"
            ));
        }
        lines.push(String::new());
    }

    // Top bridge nodes
    let mut bridge_nodes: Vec<(&str, usize, usize)> = members
        .iter()
        .filter_map(|node_id| {
            let reach = community_reach(graph, node_id, node_community);
            if reach > 0 {
                let deg = graph
                    .edges()
                    .filter(|e| e.source == *node_id || e.target == *node_id)
                    .count();
                Some((node_id.as_str(), deg, reach))
            } else {
                None
            }
        })
        .collect();
    bridge_nodes.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| b.1.cmp(&a.1)));
    let top_bridges = &bridge_nodes[..bridge_nodes.len().min(5)];
    if !top_bridges.is_empty() {
        lines.push("## Top bridge nodes".to_string());
        for &(node_id, degree, reach) in top_bridges {
            let node_label = node_filename
                .get(node_id)
                .cloned()
                .unwrap_or_else(|| node_id.to_string());
            let comm_word = if reach == 1 {
                "community"
            } else {
                "communities"
            };
            lines.push(format!(
                "- [[{node_label}]] - degree {degree}, connects to {reach} {comm_word}"
            ));
        }
    }

    let community_safe = safe_name(&community_name);
    let fname = format!("_COMMUNITY_{community_safe}.md");
    std::fs::write(output_dir.join(fname), lines.join("\n"))?;
    Ok(())
}

/// Export graph as an Obsidian vault.
///
/// One `.md` file per node with `[[wikilinks]]`, plus one `_COMMUNITY_name.md`
/// overview note per community. Writes `.obsidian/graph.json` for community
/// colour groups.
///
/// Returns the total number of notes written (node notes + community notes).
///
/// Mirrors Python `to_obsidian`.
///
/// # Panics
///
/// Panics if a `COMMUNITY_COLORS` hex literal fails to parse — this is a
/// programmer error and cannot happen with the compile-time constants.
///
/// # Errors
///
/// Returns [`ExportError::Io`] on any file-system failure.
pub fn to_obsidian(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    output_dir: &Path,
    community_labels: Option<&IndexMap<i64, String>>,
    cohesion: Option<&IndexMap<i64, f64>>,
) -> Result<usize, ExportError> {
    std::fs::create_dir_all(output_dir)?;

    let node_community = node_community_map(communities);

    // Build node_id → safe filename (deduplication via numeric suffix)
    let mut node_filename: IndexMap<String, String> = IndexMap::new();
    let mut seen_names: IndexMap<String, usize> = IndexMap::new();
    for (node_id, attrs) in graph.nodes() {
        let raw_label = attrs
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or(node_id);
        let base = safe_name(raw_label);
        let fname = if let Some(count) = seen_names.get_mut(&base) {
            *count += 1;
            format!("{base}_{count}")
        } else {
            seen_names.insert(base.clone(), 0);
            base.clone()
        };
        node_filename.insert(node_id.clone(), fname);
    }

    // Write one .md per node — fully independent file writes, so fan out
    // across Rayon. `?` is hoisted out via `collect::<Result<_, _>>()` so the
    // first error short-circuits the parallel walk.
    let node_refs: Vec<(&String, &indexmap::IndexMap<String, Value>)> = graph.nodes().collect();
    node_refs
        .par_iter()
        .try_for_each(|(node_id, attrs)| -> Result<(), ExportError> {
            write_node_note(
                node_id,
                attrs,
                graph,
                &node_community,
                &node_filename,
                community_labels,
                output_dir,
            )
        })?;

    // Build inter-community edge counts
    let mut inter_community: IndexMap<i64, IndexMap<i64, usize>> =
        communities.keys().map(|&k| (k, IndexMap::new())).collect();
    for edge in graph.edges() {
        let cu = node_community.get(&edge.source).copied();
        let cv = node_community.get(&edge.target).copied();
        if let (Some(cu), Some(cv)) = (cu, cv)
            && cu != cv
        {
            *inter_community
                .entry(cu)
                .or_default()
                .entry(cv)
                .or_insert(0) += 1;
            *inter_community
                .entry(cv)
                .or_default()
                .entry(cu)
                .or_insert(0) += 1;
        }
    }

    // Write one _COMMUNITY_<name>.md per community. Per-community writes are
    // independent files, safe to fan out across Rayon.
    let community_pairs: Vec<(&i64, &Vec<String>)> = communities.iter().collect();
    let community_notes_written = community_pairs.len();
    let note_ctx = CommunityNoteCtx {
        graph,
        node_community: &node_community,
        node_filename: &node_filename,
        community_labels,
        cohesion,
        inter_community: &inter_community,
        output_dir,
    };
    community_pairs
        .par_iter()
        .try_for_each(|(cid, members)| -> Result<(), ExportError> {
            write_community_note(&note_ctx, **cid, members)
        })?;

    // Write .obsidian/graph.json for community colour groups
    let obsidian_dir = output_dir.join(".obsidian");
    std::fs::create_dir_all(&obsidian_dir)?;

    let color_groups: Vec<serde_json::Value> = community_labels.map_or_else(Vec::new, |cl| {
        let mut sorted: Vec<(i64, &String)> = cl.iter().map(|(&k, v)| (k, v)).collect();
        sorted.sort_by_key(|(k, _)| *k);
        sorted
            .into_iter()
            .map(|(cid, label)| {
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let color_hex =
                    COMMUNITY_COLORS[(cid.unsigned_abs() as usize) % COMMUNITY_COLORS.len()];
                #[allow(clippy::expect_used)]
                let rgb_int = u32::from_str_radix(color_hex.trim_start_matches('#'), 16)
                    .expect("COMMUNITY_COLORS entries are valid hex");
                serde_json::json!({
                    "query": format!("tag:#community/{}", label.replace(' ', "_")),
                    "color": {"a": 1, "rgb": rgb_int}
                })
            })
            .collect()
    });

    let graph_config = serde_json::json!({ "colorGroups": color_groups });
    std::fs::write(
        obsidian_dir.join("graph.json"),
        serde_json::to_string_pretty(&graph_config)?,
    )?;

    Ok(graph.node_count() + community_notes_written)
}
