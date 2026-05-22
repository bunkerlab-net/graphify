//! Dashboard rendering: `render_dashboard`, `render_worktrees`,
//! `render_conflicts`, `render_pr_detail`, `format_prs_text`.

use std::collections::HashMap;
use std::hash::BuildHasher;

use indexmap::IndexMap;

use crate::color::{bold, cyan, dim, green, pad, red, yellow};
use crate::model::{PrInfo, STATUS_ORDER};

// ── Internal helpers ───────────────────────────────────────────────────────

/// Truncate `s` to at most `n` Unicode scalar values, appending `…` if truncated.
fn truncate(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        s.to_string()
    } else {
        let truncated: String = chars[..n.saturating_sub(1)].iter().collect();
        format!("{truncated}…")
    }
}

/// Map a PR status string to its ANSI-coloured representation for the dashboard.
fn status_color(status: &str) -> String {
    match status {
        "READY" => green(status),
        "APPROVED" => bold(&green(status)),
        "CI-FAIL" | "CHANGES-REQ" => red(status),
        "WRONG-BASE" | "STALE" => dim(status),
        "DRAFT" | "PENDING" => yellow(status),
        other => other.to_string(),
    }
}

/// Return a coloured CI status icon (`✓`, `✗`, `…`, `–`) for the given CI status string.
fn ci_icon(status: &str) -> String {
    match status {
        "SUCCESS" => green("✓"),
        "FAILURE" => red("✗"),
        "PENDING" => yellow("…"),
        "NONE" => dim("–"),
        _ => "?".to_string(),
    }
}

/// Return the sort priority of `status` (lower = higher priority in the dashboard).
///
/// Looks up `status` in `STATUS_ORDER`; unknown statuses fall to index 99.
fn status_order_index(status: &str) -> usize {
    STATUS_ORDER.iter().position(|&s| s == status).unwrap_or(99)
}

// ── Public render functions ────────────────────────────────────────────────

/// Render the main PR dashboard to stdout.
#[allow(clippy::too_many_lines)] // direct port of Python render_dashboard
pub fn render_dashboard(prs: &[PrInfo], base: &str, show_wrong_base: bool) {
    let mut actionable: Vec<&PrInfo> = prs.iter().filter(|p| p.base_branch == base).collect();
    let wrong_base: Vec<&PrInfo> = prs.iter().filter(|p| p.base_branch != base).collect();

    actionable.sort_by_key(|p| (status_order_index(&p.status()), p.days_old()));

    println!();
    println!(
        "  {}",
        bold(&format!(
            "graphify prs  ·  base: {base}  ·  {} PRs",
            actionable.len()
        ))
    );
    println!();

    if actionable.is_empty() {
        println!("{}", dim("  No open PRs targeting this base branch."));
    } else {
        println!(
            "  {:>4}  {:2}  {:13}  {:8}  {:22}  TITLE",
            "#", "CI", "STATUS", "UPDATED", "IMPACT"
        );
        println!(
            "  {}  {}  {}  {}  {}  {}",
            "─".repeat(4),
            "─".repeat(2),
            "─".repeat(13),
            "─".repeat(8),
            "─".repeat(22),
            "─".repeat(40),
        );

        for pr in &actionable {
            let status_str = pad(&status_color(&pr.status()), 13);
            let ci_str = ci_icon(&pr.ci_status);
            let age = if pr.days_old() > 0 {
                format!("{}d", pr.days_old())
            } else {
                "today".to_string()
            };
            let br = pr.blast_radius();
            let impact = if br.is_empty() {
                pad(&dim("–"), 22)
            } else {
                pad(&dim(&truncate(&br, 22)), 22)
            };
            let wt = if pr.worktree_path.is_some() {
                format!(" {}", cyan("⬡"))
            } else {
                "  ".to_string()
            };
            let draft_str = if pr.is_draft {
                dim(" [draft]")
            } else {
                String::new()
            };
            let title = truncate(&pr.title, 52);
            let num = pad(&bold(&format!("#{}", pr.number)), 6);
            println!(
                "  {num}{wt}  {ci_str}  {status_str}  {age:>6}   {impact}  {title}{draft_str}"
            );
        }
    }

    // Summary line
    let mut by_status: HashMap<String, usize> = HashMap::new();
    for p in &actionable {
        *by_status.entry(p.status()).or_insert(0) += 1;
    }

    let mut parts: Vec<String> = Vec::new();
    if let Some(&n) = by_status.get("READY") {
        parts.push(green(&format!("{n} ready")));
    }
    if let Some(&n) = by_status.get("APPROVED") {
        parts.push(bold(&green(&format!("{n} approved"))));
    }
    if let Some(&n) = by_status.get("PENDING") {
        parts.push(yellow(&format!("{n} pending CI")));
    }
    if let Some(&n) = by_status.get("CI-FAIL") {
        parts.push(red(&format!("{n} CI failing")));
    }
    if let Some(&n) = by_status.get("CHANGES-REQ") {
        parts.push(red(&format!("{n} changes requested")));
    }
    if let Some(&n) = by_status.get("DRAFT") {
        parts.push(yellow(&format!("{n} draft")));
    }
    if let Some(&n) = by_status.get("STALE") {
        parts.push(dim(&format!("{n} stale")));
    }
    if !wrong_base.is_empty() {
        parts.push(dim(&format!("{} wrong base", wrong_base.len())));
    }

    println!();
    println!("  {}", parts.join(" · "));
    println!();

    if !wrong_base.is_empty() && show_wrong_base {
        println!(
            "{}",
            dim(&format!(
                "  ── {} PRs targeting wrong base ──",
                wrong_base.len()
            ))
        );
        let mut sorted_wrong: Vec<&PrInfo> = wrong_base;
        sorted_wrong.sort_by_key(|p| std::cmp::Reverse(p.number));
        for pr in sorted_wrong {
            println!(
                "{}",
                dim(&format!(
                    "  #{:4}  base={:12}  {}",
                    pr.number,
                    pr.base_branch,
                    truncate(&pr.title, 60)
                ))
            );
        }
        println!();
    }
}

/// Render the worktree → branch → PR mapping.
pub fn render_worktrees<S: BuildHasher>(prs: &[PrInfo], worktrees: &HashMap<String, String, S>) {
    println!();
    println!("  {}", bold("Worktrees"));
    println!();

    if worktrees.is_empty() {
        println!("{}", dim("  No active worktrees found."));
        println!();
        return;
    }

    let pr_by_branch: HashMap<&str, &PrInfo> = prs.iter().map(|p| (p.branch.as_str(), p)).collect();

    let mut sorted_wts: Vec<(&String, &String)> = worktrees.iter().collect();
    sorted_wts.sort_by_key(|(branch, _)| branch.as_str());

    for (branch, path) in sorted_wts {
        let pr = pr_by_branch.get(branch.as_str()).copied();
        println!("  {}", cyan(path));
        if let Some(pr) = pr {
            println!(
                "    {} {}  →  PR {}  [{}]  {}",
                dim("branch:"),
                branch,
                bold(&format!("#{}", pr.number)),
                status_color(&pr.status()),
                truncate(&pr.title, 50),
            );
        } else {
            println!("    {} {}  {}", dim("branch:"), branch, dim("(no open PR)"));
        }
        println!();
    }
}

/// Render community-conflict analysis.
pub fn render_conflicts(
    prs: &[PrInfo],
    base: &str,
    community_labels: Option<&IndexMap<i64, Vec<String>>>,
) {
    let actionable: Vec<&PrInfo> = prs
        .iter()
        .filter(|p| p.base_branch == base && !p.communities_touched.is_empty())
        .collect();

    if actionable.is_empty() {
        println!(
            "{}",
            dim("\n  No graph impact data — run with a valid graph.json to detect conflicts.\n")
        );
        return;
    }

    let mut comm_to_prs: IndexMap<i64, Vec<&PrInfo>> = IndexMap::new();
    for pr in &actionable {
        for &c in &pr.communities_touched {
            comm_to_prs.entry(c).or_default().push(pr);
        }
    }

    let conflicts: IndexMap<i64, Vec<&PrInfo>> = comm_to_prs
        .into_iter()
        .filter(|(_, ps)| ps.len() > 1)
        .collect();

    if conflicts.is_empty() {
        println!(
            "{}",
            green("\n  No community overlap between open PRs — safe to merge in any order.\n")
        );
        return;
    }

    println!();
    println!(
        "  {}",
        bold("Community conflicts (PRs sharing the same graph community)")
    );
    println!();

    let empty_labels: IndexMap<i64, Vec<String>> = IndexMap::new();
    let labels = community_labels.unwrap_or(&empty_labels);

    let mut sorted_conflicts: Vec<(i64, Vec<&PrInfo>)> = conflicts.into_iter().collect();
    sorted_conflicts.sort_by_key(|(_, ps)| std::cmp::Reverse(ps.len()));

    for (comm, ps) in sorted_conflicts {
        let label_str = labels
            .get(&comm)
            .filter(|v| !v.is_empty())
            .map(|v| dim(&format!("  — {}", v.join(", "))))
            .unwrap_or_default();
        println!(
            "  {}{}  ({} PRs overlap)",
            yellow(&format!("Community {comm}")),
            label_str,
            ps.len()
        );
        for pr in &ps {
            println!(
                "    #{:4}  {}  {}",
                pr.number,
                pad(&status_color(&pr.status()), 13),
                truncate(&pr.title, 55),
            );
        }
        println!();
    }
}

/// Render a single-PR deep-dive.
pub fn render_pr_detail(pr: &PrInfo) {
    println!();
    println!(
        "  {}",
        bold(&format!(
            "PR #{}  ·  {}",
            pr.number,
            status_color(&pr.status())
        ))
    );
    println!("  {}", pr.title);
    println!();
    println!("  {}  {}  →  {}", dim("branch:"), pr.branch, pr.base_branch);
    println!("  {}  {}", dim("author:"), pr.author);
    println!("  {} {}d ago", dim("updated:"), pr.days_old());
    println!(
        "  {}      {} {}",
        dim("CI:"),
        ci_icon(&pr.ci_status),
        pr.ci_status
    );
    if !pr.review_decision.is_empty() {
        println!("  {} {}", dim("review:"), pr.review_decision);
    }
    if let Some(ref wt) = pr.worktree_path {
        println!("  {} {}", dim("worktree:"), cyan(wt));
    }
    let br = pr.blast_radius();
    if !br.is_empty() {
        println!();
        println!("  {}  {}", bold("Graph impact:"), br);
        println!("  {} {:?}", dim("communities:"), pr.communities_touched);
        if !pr.files_changed.is_empty() {
            println!("  {} {}", dim("files changed:"), pr.files_changed.len());
            for f in pr.files_changed.iter().take(10) {
                println!("    {}", dim(f));
            }
            if pr.files_changed.len() > 10 {
                println!(
                    "{}",
                    dim(&format!("    … and {} more", pr.files_changed.len() - 10))
                );
            }
        }
    }
    println!();
}

/// Plain-text PR summary for MCP tool output (no ANSI).
#[must_use]
pub fn format_prs_text(prs: &[PrInfo], base: &str) -> String {
    let actionable: Vec<&PrInfo> = prs.iter().filter(|p| p.base_branch == base).collect();
    let wrong = prs.len() - actionable.len();

    let mut lines: Vec<String> = vec![format!(
        "Open PRs targeting {base}: {}  ({wrong} on wrong base, not shown)\n",
        actionable.len()
    )];

    let mut sorted: Vec<&PrInfo> = actionable;
    sorted.sort_by_key(|p| (status_order_index(&p.status()), p.days_old()));

    for p in sorted {
        let impact = if p.blast_radius().is_empty() {
            String::new()
        } else {
            format!("  blast_radius={}", p.blast_radius())
        };
        let review = if p.review_decision.is_empty() {
            "none"
        } else {
            &p.review_decision
        };
        lines.push(format!(
            "#{} [{}] CI={} review={} age={}d author={}{}\n  {}",
            p.number,
            p.status(),
            p.ci_status,
            review,
            p.days_old(),
            p.author,
            impact,
            p.title,
        ));
    }
    lines.join("\n\n")
}
