//! Graph-to-callflow data builder: community indexing, section normalization,
//! and edge classification.
//!
//! Extracted so the structural analysis logic (which community goes in which
//! section, which edges are intra- vs inter-section) is separate from both
//! the raw data loading (`loader`) and the HTML rendering (`render`).
//!
//! Architecture archetype matching and community-to-section derivation live
//! in the sibling `archetypes` module.

use std::collections::HashMap;

use indexmap::IndexMap;
use sha2::{Digest, Sha256};

use super::loader::{is_zh, pick_text, safe_mermaid_text};
use super::options::{CfEdge, Node, Section};

// ── Label / kind helpers ────────────────────────────────────────────────────

/// Truncate `text` to `limit` bytes, appending `"..."` when cut.
pub(super) fn truncate_text(text: &str, limit: usize) -> String {
    let s: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.len() <= limit {
        s
    } else {
        format!(
            "{}...",
            &s[..s.len().min(limit.saturating_sub(3))].trim_end()
        )
    }
}

/// Convert a raw node label into a short, human-readable display string.
///
/// Strips common boilerplate (leading dots, file extensions, long `snake_case`
/// identifiers) and truncates to 42 characters.
pub(super) fn humanize_label(label: &str, source_file: &str) -> String {
    let label = label.trim();
    if label.is_empty() {
        return std::path::Path::new(source_file)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown")
            .to_owned();
    }
    if label.starts_with('.') && label.ends_with("()") {
        return label[1..].to_owned();
    }
    let code_exts = [
        ".py", ".ts", ".tsx", ".js", ".jsx", ".go", ".rs", ".java", ".rb",
    ];
    if code_exts.iter().any(|e| label.ends_with(e)) {
        return std::path::Path::new(label)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(label)
            .to_owned();
    }
    if label.contains('_') && !label.contains(' ') && label.len() > 28 {
        let parts: Vec<&str> = label.split('_').filter(|p| !p.is_empty()).collect();
        if !parts.is_empty() {
            let joined = parts[parts.len().saturating_sub(3)..].join(" ");
            return truncate_text(&joined, 42);
        }
    }
    truncate_text(label, 42)
}

/// Classify a node into a Mermaid shape category (`"klass"`, `"module"`, `"api"`, etc.).
///
/// Inspects `node_type`, `file_type`, and label heuristics in priority order,
/// mirroring Python's `_node_kind` in `callflow.py`.
pub(super) fn node_kind(node: &Node) -> &'static str {
    let label = node.label.to_lowercase();
    let source_file = node.source_file.to_lowercase();
    let file_type = node.file_type.to_lowercase();
    let node_type = node.node_type.to_lowercase();
    match node_type.as_str() {
        "class" | "klass" | "struct" | "interface" | "enum" | "trait" | "model" => return "klass",
        "module" | "file" | "package" | "namespace" => return "module",
        "endpoint" | "route" | "api" | "handler" | "controller" => return "api",
        "test" | "spec" => return "test",
        "component" | "hook" | "view" | "page" => return "ui",
        _ => {}
    }
    if file_type == "rationale" || file_type == "document" {
        return "concept";
    }
    if source_file.contains("test") || label.starts_with("test_") || source_file.contains("spec") {
        return "test";
    }
    if ["endpoint", "router", "api", "route"]
        .iter()
        .any(|w| label.contains(w))
    {
        return "api";
    }
    if ["cli", "command", "click", "typer"]
        .iter()
        .any(|w| label.contains(w))
    {
        return "entry";
    }
    if ["async", "await", "stream", "sse"]
        .iter()
        .any(|w| label.contains(w))
    {
        return "async";
    }
    let raw_label = &node.label;
    let hook_like = raw_label.starts_with("use")
        && raw_label.len() > 3
        && raw_label
            .chars()
            .nth(3)
            .is_some_and(|c| c.is_uppercase() || c == '_' || c == '-');
    let sf_lower = source_file.to_lowercase();
    if ["component", "props", "hook", "store"]
        .iter()
        .any(|w| label.contains(w))
        || hook_like
        || matches!(
            std::path::Path::new(&sf_lower)
                .extension()
                .and_then(|e| e.to_str()),
            Some("tsx" | "jsx" | "vue" | "svelte")
        )
    {
        return "ui";
    }
    if raw_label.chars().next().is_some_and(char::is_uppercase) && !raw_label.ends_with("()") {
        return "klass";
    }
    let rl_lower = raw_label.to_lowercase();
    let module_exts = [
        ".py", ".ts", ".tsx", ".js", ".jsx", ".go", ".rs", ".java", ".kt", ".rb", ".php", ".cs",
        ".swift", ".vue", ".svelte",
    ];
    if module_exts.iter().any(|e| rl_lower.ends_with(e)) {
        return "module";
    }
    "function"
}

/// Map a relation type string to a localized display label for Mermaid edges.
///
/// Tries a lookup table for known relation types; unknown relations have
/// underscores replaced with spaces. The result is sanitized via `safe_mermaid_text`.
pub(super) fn relation_label(relation: &str, lang: &str) -> String {
    let relation = relation.trim();
    let zh: HashMap<&str, &str> = [
        ("calls", "调用"),
        ("uses", "使用"),
        ("imports", "导入"),
        ("imports_from", "导入"),
        ("method", "方法"),
        ("contains", "包含"),
        ("rationale_for", "说明"),
        ("conceptually_related_to", "相关"),
        ("participate_in", "参与"),
        ("form", "组成"),
    ]
    .iter()
    .copied()
    .collect();
    let en: HashMap<&str, &str> = [
        ("calls", "calls"),
        ("uses", "uses"),
        ("imports", "imports"),
        ("imports_from", "imports"),
        ("method", "method"),
        ("contains", "contains"),
        ("rationale_for", "explains"),
        ("conceptually_related_to", "relates"),
        ("participate_in", "joins"),
        ("form", "forms"),
    ]
    .iter()
    .copied()
    .collect();
    let fallback = relation.replace('_', " ");
    let mapped: &str = if is_zh(lang) {
        zh.get(relation).copied().unwrap_or(fallback.as_str())
    } else {
        en.get(relation).copied().unwrap_or(fallback.as_str())
    };
    safe_mermaid_text(mapped)
}

/// Return `true` if the edge should be rendered in the callflow diagram.
///
/// EXTRACTED edges always pass; INFERRED edges require a confidence score ≥ 0.85.
pub(super) fn should_include_edge(edge: &CfEdge) -> bool {
    match edge.confidence.as_str() {
        "EXTRACTED" => true,
        "INFERRED" => edge.confidence_score >= 0.85,
        _ => false,
    }
}

/// Compute a priority score for an edge, used for diagram node selection.
///
/// Higher scores indicate more architecturally significant edges; call/use
/// relations score higher than structural ones.
pub(super) fn edge_score(edge: &CfEdge) -> f64 {
    let mut score = edge.confidence_score;
    if edge.confidence == "EXTRACTED" {
        score += 2.0;
    }
    match edge.relation.as_str() {
        "calls" | "uses" | "method" => score += 1.0,
        "imports" | "imports_from" => score += 0.6,
        "contains" => score -= 0.2,
        "rationale_for" => score -= 0.6,
        _ => {}
    }
    score
}

/// Filter edges to the preferred subset for diagram rendering.
///
/// Primary relations (calls, uses, imports) are preferred; structural relations
/// (`contains`, `rationale_for`) are included only when `allow_structure` is true.
/// Falls back to all passing edges if no preferred ones remain.
pub(super) fn preferred_edges(edges: &[CfEdge], allow_structure: bool) -> Vec<&CfEdge> {
    let primary: std::collections::HashSet<&str> =
        ["calls", "uses", "method", "imports", "imports_from"]
            .iter()
            .copied()
            .collect();
    let secondary: std::collections::HashSet<&str> =
        ["contains", "rationale_for", "conceptually_related_to"]
            .iter()
            .copied()
            .collect();
    let mut selected: Vec<&CfEdge> = edges
        .iter()
        .filter(|e| {
            should_include_edge(e)
                && (primary.contains(e.relation.as_str())
                    || (allow_structure && secondary.contains(e.relation.as_str())))
        })
        .collect();
    if selected.is_empty() {
        selected = edges.iter().filter(|e| should_include_edge(e)).collect();
    }
    selected
}

// ── Community / section indexing ────────────────────────────────────────────

/// Build a community-id → node-index map from a slice of nodes.
///
/// Used for fast community-member lookups without iterating the full node list.
pub(super) fn build_community_index(nodes: &[Node]) -> IndexMap<String, Vec<usize>> {
    let mut idx: IndexMap<String, Vec<usize>> = IndexMap::new();
    for (i, n) in nodes.iter().enumerate() {
        idx.entry(n.community.clone()).or_default().push(i);
    }
    idx
}

/// Produce a unique slug suitable for use as an HTML anchor `id` attribute.
///
/// Lowercases and slugifies `raw`, then appends a hash suffix when a collision
/// would occur within the set `used`. Guarantees uniqueness within a single page.
#[allow(clippy::expect_used)] // reason: static literal regex literals cannot fail
pub(super) fn html_anchor_id(
    raw: &str,
    fallback: &str,
    used: &mut std::collections::HashSet<String>,
) -> String {
    let re = regex::Regex::new(r"[^a-z0-9]+").expect("static regex literal cannot fail");
    let raw_str = if raw.is_empty() { fallback } else { raw };
    let base: String = {
        let lower = raw_str.to_lowercase();
        let slug = re.replace_all(&lower, "-").trim_matches('-').to_owned();
        if slug.is_empty() {
            let fb_lower = fallback.to_lowercase();
            let fb_slug = re.replace_all(&fb_lower, "-").trim_matches('-').to_owned();
            if fb_slug.is_empty() {
                "section".to_owned()
            } else {
                fb_slug
            }
        } else {
            slug
        }
    };
    let base = base[..base.len().min(48)].trim_end_matches('-');
    let base = if base.is_empty() { "section" } else { base };
    let mut candidate = base.to_owned();
    if used.contains(&candidate) {
        let mut hasher = Sha256::new();
        hasher.update(raw_str.as_bytes());
        let hash = hex::encode(&hasher.finalize()[..3]);
        candidate = format!("{base}-{hash}");
    }
    let mut suffix = 2usize;
    while used.contains(&candidate) {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    used.insert(candidate.clone());
    candidate
}

/// Normalize a list of sections, ensuring unique IDs and prepending overview.
#[must_use]
pub fn normalize_sections(sections: &[Section], lang: &str) -> Vec<Section> {
    let overview_name = pick_text(lang, "架构总览", "Architecture Overview");
    let mut result = vec![Section {
        id: "overview".to_owned(),
        name: overview_name.to_owned(),
        communities: vec![],
    }];
    let mut used: std::collections::HashSet<String> = ["overview", "hyperedges", "stats"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();

    for (index, raw) in sections.iter().enumerate() {
        let raw_id = if raw.id.is_empty() {
            format!("section-{}", index + 1)
        } else {
            raw.id.clone()
        };
        let raw_name = if raw.name.is_empty() {
            raw_id.clone()
        } else {
            raw.name.clone()
        };
        if raw_id.to_lowercase() == "overview" {
            result[0].name = if raw_name.is_empty() {
                overview_name.to_owned()
            } else {
                raw_name
            };
            continue;
        }
        let sid = html_anchor_id(&raw_id, &format!("section-{}", index + 1), &mut used);
        result.push(Section {
            id: sid,
            name: raw_name,
            communities: raw.communities.clone(),
        });
    }
    result
}

/// Build a section-id → node-index map by expanding each section's community list.
pub(super) fn build_section_node_map(
    sections: &[Section],
    comm_idx: &IndexMap<String, Vec<usize>>,
) -> IndexMap<String, Vec<usize>> {
    let mut map: IndexMap<String, Vec<usize>> = IndexMap::new();
    for sec in sections {
        let sid = &sec.id;
        if sid == "overview" {
            map.insert(sid.clone(), vec![]);
            continue;
        }
        let mut idxs = vec![];
        for cid in &sec.communities {
            if let Some(v) = comm_idx.get(cid.as_str()) {
                idxs.extend_from_slice(v);
            }
        }
        map.insert(sid.clone(), idxs);
    }
    map
}

// ── Edge classification ──────────────────────────────────────────────────────

pub(super) struct ClassifiedEdges {
    pub(super) intra: IndexMap<String, Vec<usize>>, // section_id -> edge indices
    pub(super) inter: Vec<usize>,
    pub(super) node_section: HashMap<String, String>,
}

/// Classify edges as intra-section or inter-section and build a node→section map.
///
/// Returns a `ClassifiedEdges` struct containing the intra-section edge index
/// map, the list of inter-section edge indices, and the node-to-section mapping
/// used by the diagram renderer.
pub(super) fn classify_edges(
    edges: &[CfEdge],
    section_nodes_map: &IndexMap<String, Vec<usize>>,
    nodes: &[Node],
) -> ClassifiedEdges {
    let mut node_section: HashMap<String, String> = HashMap::new();
    for (sid, idxs) in section_nodes_map {
        for &i in idxs {
            node_section.insert(nodes[i].id.clone(), sid.clone());
        }
    }
    let mut intra: IndexMap<String, Vec<usize>> = IndexMap::new();
    let mut inter: Vec<usize> = vec![];

    for (ei, e) in edges.iter().enumerate() {
        let src_sec = node_section.get(&e.source);
        let tgt_sec = node_section.get(&e.target);
        match (src_sec, tgt_sec) {
            (None, _) | (_, None) => {} // orphan — not tracked
            (Some(ss), Some(ts)) if ss == ts => intra.entry(ss.clone()).or_default().push(ei),
            (Some(_ss), Some(_ts)) => inter.push(ei),
        }
    }
    ClassifiedEdges {
        intra,
        inter,
        node_section,
    }
}

/// Summarize inter-section edge traffic as a `(src, tgt) → (count, top_relation)` map.
///
/// Used by the overview diagram to show cross-section dependency arrows with
/// edge counts and the most common relation type on each arrow.
pub(super) fn section_edge_summary(
    classified: &ClassifiedEdges,
    edges: &[CfEdge],
) -> IndexMap<(String, String), (usize, String)> {
    let mut summary: IndexMap<(String, String), (usize, IndexMap<String, usize>)> = IndexMap::new();
    for &ei in &classified.inter {
        let e = &edges[ei];
        if !should_include_edge(e) {
            continue;
        }
        let src_sec = classified.node_section.get(&e.source);
        let tgt_sec = classified.node_section.get(&e.target);
        match (src_sec, tgt_sec) {
            (Some(ss), Some(ts)) if ss != ts => {
                let entry = summary
                    .entry((ss.clone(), ts.clone()))
                    .or_insert((0, IndexMap::new()));
                entry.0 += 1;
                *entry.1.entry(e.relation.clone()).or_insert(0) += 1;
            }
            _ => {}
        }
    }
    // Convert to (count, most-common-relation).
    summary
        .into_iter()
        .map(|(k, (count, rels))| {
            let top_rel = rels
                .iter()
                .max_by_key(|(_, c)| *c)
                .map_or("relates", |(r, _)| r.as_str())
                .to_owned();
            (k, (count, top_rel))
        })
        .collect()
}
