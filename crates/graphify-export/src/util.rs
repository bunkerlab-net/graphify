//! Shared utility constants and helpers used by every exporter format.

use indexmap::IndexMap;

/// Community colors — shared across HTML, SVG, Obsidian, Canvas.
pub const COMMUNITY_COLORS: [&str; 10] = [
    "#4E79A7", "#F28E2B", "#E15759", "#76B7B2", "#59A14F", "#EDC948", "#B07AA1", "#FF9DA7",
    "#9C755F", "#BAB0AC",
];

/// Maximum nodes before HTML viz is disabled (configurable via
/// `GRAPHIFY_VIZ_NODE_LIMIT`).
pub const MAX_NODES_FOR_VIZ: usize = 5_000;

/// Artifacts worth preserving across rebuilds — listed here once so the
/// JSON exporter, the watch rebuild loop, and downstream consumers all
/// agree on the set.
pub const BACKUP_ARTIFACTS: &[&str] = &[
    "graph.json",
    "GRAPH_REPORT.md",
    ".graphify_labels.json",
    ".graphify_analysis.json",
    "manifest.json",
    ".graphify_semantic_marker",
    "cost.json",
];

/// Return the effective viz node limit, honoring the
/// `GRAPHIFY_VIZ_NODE_LIMIT` environment variable.
#[must_use]
pub fn viz_node_limit() -> usize {
    let raw = std::env::var("GRAPHIFY_VIZ_NODE_LIMIT").unwrap_or_default();
    if raw.trim().is_empty() {
        return MAX_NODES_FOR_VIZ;
    }
    raw.trim().parse().unwrap_or(MAX_NODES_FOR_VIZ)
}

/// Build a `node_id → community_id` inversion of the communities map.
///
/// Mirrors Python `_node_community_map`.
#[must_use]
pub fn node_community_map(communities: &IndexMap<i64, Vec<String>>) -> IndexMap<String, i64> {
    let mut m = IndexMap::new();
    for (cid, nodes) in communities {
        for n in nodes {
            m.insert(n.clone(), *cid);
        }
    }
    m
}

/// Sanitize a community name for use as an Obsidian tag.
///
/// Obsidian tags only allow alphanumerics, hyphens, underscores, and
/// slashes. Spaces become underscores; everything else is stripped.
///
/// Mirrors Python `_obsidian_tag`.
#[must_use]
pub fn obsidian_tag(name: &str) -> String {
    name.replace(' ', "_")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '/')
        .collect()
}

/// Strip combining diacritical marks via a fast range-check that covers
/// the four main Unicode combining blocks.
///
/// Mirrors Python `_strip_diacritics` (NFKD decomposition + drop
/// combining marks).
#[must_use]
pub fn strip_diacritics(text: &str) -> String {
    text.chars().filter(|c| !is_combining_char(*c)).collect()
}

/// Return `true` if `c` is a Unicode combining character
/// (`General_Category=M`).
///
/// Fast range check covering the most common combining blocks:
/// Combining Diacritical Marks, Supplement, Extended, and Half Marks.
fn is_combining_char(c: char) -> bool {
    let cp = c as u32;
    (0x0300..=0x036F).contains(&cp)
        || (0x1DC0..=0x1DFF).contains(&cp)
        || (0x1AB0..=0x1AFF).contains(&cp)
        || (0xFE20..=0xFE2F).contains(&cp)
}

/// Escape a value for safe embedding in a YAML double-quoted scalar
/// (F-009).
///
/// Handles backslash, double-quote, all line breaks, tab, NUL, and
/// other C0/DEL control characters.
///
/// Mirrors Python `_yaml_str`.
#[must_use]
pub fn yaml_str(s: &str) -> String {
    use std::fmt::Write as FmtWrite;
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let cp = ch as u32;
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            '\u{2028}' => out.push_str("\\L"),
            '\u{2029}' => out.push_str("\\P"),
            _ if cp < 0x20 || cp == 0x7F => {
                // Infallible write to String.
                let _ = write!(out, "\\x{cp:02x}");
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Default confidence → numeric score mapping.
///
/// Mirrors `_CONFIDENCE_SCORE_DEFAULTS`: `INFERRED → 0.5`,
/// `AMBIGUOUS → 0.2`, everything else → `1.0`.
#[must_use]
pub fn confidence_score(confidence: &str) -> f64 {
    match confidence {
        "INFERRED" => 0.5,
        "AMBIGUOUS" => 0.2,
        _ => 1.0,
    }
}
