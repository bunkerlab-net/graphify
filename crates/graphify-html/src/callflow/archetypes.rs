//! Architecture section archetypes and community-to-section derivation.
//!
//! Extracted from `builder` so the large keyword tables and the community
//! classification algorithm can be read in isolation from the lower-level
//! graph-structure helpers.

use std::collections::HashMap;

use indexmap::IndexMap;

use super::builder::build_community_index;
use super::loader::pick_text;
use super::options::{Node, Section};

// ── Architecture keyword archetypes ────────────────────────────────────────

/// Architecture keyword archetypes for section classification.
static SECTION_ARCHETYPES: &[(&str, &str, &str, &[&str])] = &[
    (
        "extract-pipeline",
        "提取管线",
        "Extraction Pipeline",
        &[
            "extract",
            "extractor",
            "tree",
            "sitter",
            "parser",
            "language",
            "python",
            "javascript",
            "typescript",
            "rust",
            "java",
            "go",
            "ast",
            "calls",
            "imports",
            "multilang",
        ],
    ),
    (
        "build-graph",
        "图谱构建",
        "Graph Build",
        &[
            "build",
            "graph",
            "merge",
            "dedup",
            "node",
            "edge",
            "hyperedge",
            "json",
            "schema",
            "normalize",
            "confidence",
        ],
    ),
    (
        "analysis-clustering",
        "分析聚类",
        "Analysis & Clustering",
        &[
            "cluster",
            "community",
            "leiden",
            "cohesion",
            "analyze",
            "god",
            "surprise",
            "question",
            "query",
            "path",
            "explain",
            "benchmark",
        ],
    ),
    (
        "outputs-docs",
        "输出文档",
        "Outputs & Docs",
        &[
            "export",
            "html",
            "wiki",
            "obsidian",
            "canvas",
            "svg",
            "graphml",
            "report",
            "callflow",
            "mermaid",
            "tree",
            "documentation",
        ],
    ),
    (
        "cli-skills",
        "CLI 与技能安装",
        "CLI & Skill Installers",
        &[
            "main",
            "install",
            "uninstall",
            "skill",
            "agent",
            "claude",
            "codex",
            "opencode",
            "aider",
            "copilot",
            "kiro",
            "vscode",
            "hook",
            "command",
        ],
    ),
    (
        "ingest-cache-update",
        "摄取与增量更新",
        "Ingestion & Updates",
        &[
            "ingest",
            "fetch",
            "download",
            "url",
            "html",
            "markdown",
            "cache",
            "manifest",
            "watch",
            "update",
            "incremental",
            "transcribe",
            "video",
            "audio",
            "google",
        ],
    ),
    (
        "serve-api",
        "服务 API",
        "Serving API",
        &[
            "serve", "api", "request", "response", "endpoint", "router", "handle", "upload",
            "search", "delete", "enrich",
        ],
    ),
    (
        "security-global",
        "安全与全局图",
        "Security & Global Graph",
        &[
            "security",
            "safe",
            "ssrf",
            "xss",
            "path",
            "traversal",
            "global",
            "prefix",
            "prune",
            "repo",
            "clone",
        ],
    ),
    (
        "tests-fixtures",
        "测试与样例",
        "Tests & Fixtures",
        &[
            "test", "tests", "fixture", "fixtures", "sample", "assert", "pytest", "mock",
        ],
    ),
];

// ── Keyword helpers ─────────────────────────────────────────────────────────

/// Build a single lowercase text blob from a community's label and node attributes.
///
/// Used as input to `keyword_score` for archetype classification.
fn community_text(nodes: &[&Node], label: &str) -> String {
    let mut parts = vec![label.to_lowercase()];
    for node in nodes.iter().take(80) {
        parts.push(node.label.to_lowercase());
        parts.push(node.source_file.to_lowercase());
        parts.push(node.node_type.to_lowercase());
        parts.push(node.file_type.to_lowercase());
    }
    parts.join(" ")
}

/// Count whole-token keyword matches in `text`.
///
/// Splits `text` into alphanumeric tokens and counts how many match any keyword
/// in `keywords`. Semantically equivalent to Python's word-boundary regex but
/// avoids lookbehind assertions not supported by the `regex` crate.
fn keyword_score(text: &str, keywords: &[&str]) -> usize {
    // The Rust `regex` crate does not support lookbehind, so we cannot directly port
    // Python's `(?<![a-z0-9])kw(?![a-z0-9])`. Instead we split the text into
    // alphanumeric tokens (treating `_`, `-`, `.`, `/` as delimiters, like Python's
    // pattern) and count whole-token matches. This is semantically equivalent for the
    // all-lowercase-ASCII keywords in SECTION_ARCHETYPES.
    let tokens: Vec<&str> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let mut score = 0usize;
    for &kw in keywords {
        score += tokens.iter().filter(|&&t| t == kw).count();
    }
    score
}

/// Extract the top `limit` representative keywords from a community's nodes.
///
/// Tokenizes labels and source paths, filters stopwords, counts occurrences,
/// and returns the most-frequent tokens in stable insertion order (ties broken
/// by first-seen, matching Python's `Counter.most_common` behaviour).
pub(super) fn section_keywords(nodes: &[&Node], limit: usize) -> Vec<String> {
    let stopwords: std::collections::HashSet<&str> = [
        "the", "and", "for", "with", "from", "this", "that", "class", "function", "method", "file",
        "src", "lib", "core", "index", "main", "init", "py", "ts", "tsx", "js", "jsx", "go", "rs",
        "java", "html", "css",
    ]
    .iter()
    .copied()
    .collect();
    // Use IndexMap to preserve insertion order — matches Python Counter.most_common()
    // behaviour where ties are broken by insertion (first-seen) order.
    let mut counts: IndexMap<String, usize> = IndexMap::new();
    for node in nodes {
        let text = format!("{} {}", node.label, node.source_file)
            .replace('/', " ")
            .replace(['_', '-'], " ");
        for raw in text.split_whitespace() {
            let word: String = raw
                .chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect();
            if word.len() >= 3 && !stopwords.contains(word.as_str()) {
                *counts.entry(word).or_insert(0) += 1;
            }
        }
    }
    let mut sorted: Vec<(String, usize)> = counts.into_iter().collect();
    // Stable sort by count-descending preserves insertion order for ties,
    // matching Python's Counter.most_common() behaviour.
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1));
    sorted.into_iter().take(limit).map(|(w, _)| w).collect()
}

/// Derive a display label for a community.
///
/// Prefers explicit labels from the labels map; falls back to the top 3
/// keyword tokens from the community's nodes; and finally falls back to a
/// generic `"Community <id>"` placeholder.
fn label_for_community<S: std::hash::BuildHasher>(
    cid: &str,
    labels: &HashMap<String, String, S>,
    nodes: &[&Node],
    lang: &str,
) -> String {
    if let Some(l) = labels.get(cid)
        && !l.is_empty()
    {
        return l.clone();
    }
    let kws = section_keywords(nodes, 3);
    if !kws.is_empty() {
        return kws
            .iter()
            .map(|w| {
                let mut c = w.chars();
                c.next().map_or_else(String::new, |f| {
                    f.to_uppercase().collect::<String>() + c.as_str()
                })
            })
            .collect::<Vec<_>>()
            .join(" ");
    }
    pick_text(lang, &format!("社区 {cid}"), &format!("Community {cid}")).to_owned()
}

// ── Section derivation ──────────────────────────────────────────────────────

/// A grouped section used during community classification.
struct GroupedSection {
    id: String,
    name: String,
    communities: Vec<String>,
    node_count: usize,
    priority: usize,
}

/// Derive architecture sections from communities when no sections file is given.
#[must_use]
pub fn derive_sections_from_communities<S: std::hash::BuildHasher>(
    nodes: &[Node],
    labels: &HashMap<String, String, S>,
    lang: &str,
    max_sections: usize,
) -> Vec<Section> {
    let comm_idx = build_community_index(nodes);
    let mut sections = vec![Section {
        id: "overview".to_owned(),
        name: pick_text(lang, "架构总览", "Architecture Overview").to_owned(),
        communities: vec![],
    }];

    let mut grouped: IndexMap<String, GroupedSection> = IndexMap::new();
    let mut unassigned: Vec<(String, Vec<usize>, String)> = vec![];

    // Sort communities largest-first.
    let mut sorted_comms: Vec<(&String, &Vec<usize>)> = comm_idx.iter().collect();
    sorted_comms.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));

    for (cid, node_indices) in &sorted_comms {
        let comm_nodes: Vec<&Node> = node_indices.iter().map(|&i| &nodes[i]).collect();
        let label = label_for_community(cid, labels, &comm_nodes, lang);
        let text = community_text(&comm_nodes, &label);

        // Find the best matching archetype.
        let mut best_sid: Option<(&str, &str, &str, usize)> = None;
        let mut best_score = 0usize;
        for (priority, (sid, zh, en, keywords)) in SECTION_ARCHETYPES.iter().enumerate() {
            let score = keyword_score(&text, keywords);
            if score > best_score {
                best_score = score;
                best_sid = Some((sid, zh, en, priority));
            }
        }

        if let Some((sid, zh_name, en_name, priority)) = best_sid.filter(|_| best_score >= 2) {
            let sec = grouped
                .entry((*sid).to_owned())
                .or_insert_with(|| GroupedSection {
                    id: (*sid).to_owned(),
                    name: pick_text(lang, zh_name, en_name).to_owned(),
                    communities: vec![],
                    node_count: 0,
                    priority,
                });
            sec.communities.push((*cid).clone());
            sec.node_count += node_indices.len();
        } else {
            unassigned.push(((*cid).clone(), (*node_indices).clone(), label));
        }
    }

    // Rank grouped sections.
    let mut ranked: Vec<GroupedSection> = grouped.into_values().collect();
    ranked.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then(b.node_count.cmp(&a.node_count))
            .then(a.id.cmp(&b.id))
    });

    let cap = max_sections.max(1).saturating_sub(1);
    let selected: Vec<GroupedSection> = ranked.drain(..ranked.len().min(cap)).collect();
    let overflow: Vec<GroupedSection> = ranked;

    sections.extend(selected.into_iter().map(|s| Section {
        id: s.id,
        name: s.name,
        communities: s.communities,
    }));

    let mut overflow_communities: Vec<String> = vec![];
    for s in overflow {
        overflow_communities.extend(s.communities);
    }

    let remaining_slots = max_sections
        .saturating_sub(sections.len().saturating_sub(1))
        .saturating_sub(1);
    for (cid, _, label) in unassigned.iter().take(remaining_slots) {
        sections.push(Section {
            id: if label.is_empty() {
                format!("community-{cid}")
            } else {
                label.clone()
            },
            name: label.clone(),
            communities: vec![cid.clone()],
        });
    }
    overflow_communities.extend(
        unassigned[remaining_slots.min(unassigned.len())..]
            .iter()
            .map(|(cid, _, _)| cid.clone()),
    );
    if !overflow_communities.is_empty() {
        sections.push(Section {
            id: "other".to_owned(),
            name: pick_text(lang, "其他", "Other").to_owned(),
            communities: overflow_communities,
        });
    }
    sections
}
