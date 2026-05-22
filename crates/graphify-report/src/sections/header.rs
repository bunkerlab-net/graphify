//! Header section renderer: date, root label, and graph-freshness block.
//!
//! Extracted from `lib.rs` so that the top-of-report lines and the
//! "Graph Freshness" commit-hash block live together.

/// Render the "Graph Freshness" block when a commit hash is known.
///
/// Mirrors the `_render_freshness` block in `report.py`.
pub(crate) fn render_freshness(lines: &mut Vec<String>, commit: &str) {
    let short = if commit.len() >= 8 {
        &commit[..8]
    } else {
        commit
    };
    lines.push(String::new());
    lines.push("## Graph Freshness".to_string());
    lines.push(format!("- Built from commit: `{short}`"));
    lines
        .push("- Run `git rev-parse HEAD` and compare to check if the graph is stale.".to_string());
    lines.push("- Run `graphify update .` after code changes (no API cost).".to_string());
}
