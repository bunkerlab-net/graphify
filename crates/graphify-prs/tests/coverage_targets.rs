//! Coverage tests for the smaller modules of `graphify-prs`.

#![allow(clippy::expect_used)]

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{Duration, Utc};
use graphify_prs::color::{ansi, bold, cyan, dim, green, magenta, pad, red, yellow};
use graphify_prs::dashboard::{
    format_prs_text, render_conflicts, render_dashboard, render_pr_detail, render_worktrees,
};
use graphify_prs::detect_default_branch;
use graphify_prs::error::PrsError;
use graphify_prs::gh::GhClient;
use graphify_prs::git::GitClient;
use graphify_prs::model::PrInfo;
use graphify_prs::triage::{NoOpTriageBackend, TriageBackend, build_triage_prompt};
use graphify_prs::{PrsArgs, run_cmd_prs};
use indexmap::IndexMap;

// ── color.rs ────────────────────────────────────────────────────────────────

#[test]
fn color_helpers_produce_strings() {
    // We can't reliably toggle NO_COLOR mid-process because of LazyLock,
    // but we can verify the helpers don't panic and produce non-empty output.
    for f in [green, red, yellow, cyan, bold, dim, magenta] as [fn(&str) -> String; 7] {
        let s = f("text");
        assert!(s.contains("text"));
    }
    assert!(ansi("31", "hello").contains("hello"));
}

#[test]
fn pad_pads_visible_width() {
    // No-color case: just padding to width.
    let s = pad("ab", 6);
    // The visible characters count after stripping ANSI.
    assert!(s.starts_with("ab"));
    // length depends on whether ANSI is stripped, but minimum width is 6.
    assert!(s.len() >= 6);
}

#[test]
fn pad_handles_string_longer_than_width() {
    let long = pad("abcdef", 3);
    // String is already longer than width; should be returned unchanged.
    assert_eq!(long, "abcdef");
}

// ── args.rs ─────────────────────────────────────────────────────────────────

#[test]
fn args_parse_defaults() {
    let args = PrsArgs::parse(&[]).expect("default parse should succeed");
    assert_eq!(args.limit, 50);
    assert!(args.graph_path.is_some());
}

#[test]
fn args_parse_flags() {
    let args = PrsArgs::parse(&["--triage", "--worktrees", "--conflicts", "--wrong-base"])
        .expect("flag parse");
    assert!(args.do_triage);
    assert!(args.do_worktrees);
    assert!(args.do_conflicts);
    assert!(args.show_wrong_base);
}

#[test]
fn args_parse_base_long() {
    let args = PrsArgs::parse(&["--base", "develop"]).expect("base parse");
    assert_eq!(args.base.as_deref(), Some("develop"));
}

#[test]
fn args_parse_base_equals() {
    let args = PrsArgs::parse(&["--base=v8"]).expect("base= parse");
    assert_eq!(args.base.as_deref(), Some("v8"));
}

#[test]
fn args_parse_base_short() {
    let args = PrsArgs::parse(&["-b", "trunk"]).expect("-b parse");
    assert_eq!(args.base.as_deref(), Some("trunk"));
}

#[test]
fn args_parse_repo_long_and_short() {
    let a = PrsArgs::parse(&["--repo", "owner/repo"]).expect("--repo parse should succeed");
    assert_eq!(a.repo.as_deref(), Some("owner/repo"));
    let b = PrsArgs::parse(&["-R", "owner/repo"]).expect("-R parse should succeed");
    assert_eq!(b.repo.as_deref(), Some("owner/repo"));
}

#[test]
fn args_parse_graph_paths() {
    let a = PrsArgs::parse(&["--graph", "/tmp/g.json"]).expect("--graph parse should succeed");
    assert_eq!(a.graph_path, Some(PathBuf::from("/tmp/g.json")));
    let b = PrsArgs::parse(&["--graph=/tmp/g.json"]).expect("--graph= parse should succeed");
    assert_eq!(b.graph_path, Some(PathBuf::from("/tmp/g.json")));
}

#[test]
fn args_parse_limit() {
    let a = PrsArgs::parse(&["--limit", "100"]).expect("--limit parse should succeed");
    assert_eq!(a.limit, 100);
    let b = PrsArgs::parse(&["--limit=25"]).expect("--limit= parse should succeed");
    assert_eq!(b.limit, 25);
    // Invalid limit silently ignored.
    let c = PrsArgs::parse(&["--limit", "not-a-num"])
        .expect("invalid --limit value should still parse (falls back to default)");
    assert_eq!(c.limit, 50);
}

#[test]
fn args_parse_pr_number_bareword() {
    let a = PrsArgs::parse(&["42"]).expect("bareword PR number should parse");
    assert_eq!(a.pr_number, Some(42));
    let b = PrsArgs::parse(&["#42"]).expect("#-prefixed PR number should parse");
    assert_eq!(b.pr_number, Some(42));
    // Non-numeric does not set pr_number.
    let c = PrsArgs::parse(&["abc"])
        .expect("non-numeric positional should still parse (pr_number stays None)");
    assert!(c.pr_number.is_none());
}

#[test]
fn args_parse_help_returns_none() {
    assert!(PrsArgs::parse(&["--help"]).is_none());
    assert!(PrsArgs::parse(&["-h"]).is_none());
}

// ── PrInfo helpers ──────────────────────────────────────────────────────────

fn make_pr(number: u64, base_branch: &str, days: i64) -> PrInfo {
    PrInfo {
        number,
        title: format!("PR {number}"),
        branch: format!("feature-{number}"),
        base_branch: base_branch.to_string(),
        author: "alice".to_string(),
        is_draft: false,
        review_decision: String::new(),
        ci_status: "SUCCESS".to_string(),
        updated_at: Utc::now() - Duration::days(days),
        expected_base: "main".to_string(),
        worktree_path: None,
        communities_touched: vec![],
        nodes_affected: 0,
        files_changed: vec![],
    }
}

// ── dashboard.rs ────────────────────────────────────────────────────────────

#[test]
fn render_dashboard_runs_with_empty_list() {
    render_dashboard(&[], "main", false);
}

#[test]
fn render_dashboard_with_prs() {
    let prs = vec![make_pr(1, "main", 1), make_pr(2, "main", 20)];
    render_dashboard(&prs, "main", false);
}

#[test]
fn render_dashboard_with_wrong_base_shown() {
    let mut pr = make_pr(1, "develop", 1);
    pr.expected_base = "main".to_string();
    let prs = vec![pr, make_pr(2, "main", 1)];
    render_dashboard(&prs, "main", true);
}

#[test]
fn render_worktrees_empty_map() {
    let wts: HashMap<String, String> = HashMap::new();
    render_worktrees(&[], &wts);
}

#[test]
fn render_worktrees_with_data() {
    let prs = vec![make_pr(1, "main", 1)];
    let mut wts = HashMap::new();
    wts.insert("feature-1".to_string(), "/path/to/wt".to_string());
    wts.insert("orphan-branch".to_string(), "/path/orphan".to_string());
    render_worktrees(&prs, &wts);
}

#[test]
fn render_conflicts_no_data() {
    render_conflicts(&[], "main", None);
}

#[test]
fn render_conflicts_no_overlap() {
    let mut pr = make_pr(1, "main", 1);
    pr.communities_touched = vec![1];
    pr.nodes_affected = 3;
    render_conflicts(&[pr], "main", None);
}

#[test]
fn render_conflicts_with_overlap_and_labels() {
    let mut a = make_pr(1, "main", 1);
    a.communities_touched = vec![1];
    a.nodes_affected = 3;
    let mut b = make_pr(2, "main", 1);
    b.communities_touched = vec![1];
    b.nodes_affected = 5;
    let mut labels: IndexMap<i64, Vec<String>> = IndexMap::new();
    labels.insert(1, vec!["auth".to_string(), "user".to_string()]);
    render_conflicts(&[a, b], "main", Some(&labels));
}

#[test]
fn render_pr_detail_with_full_data() {
    let mut pr = make_pr(99, "main", 3);
    pr.review_decision = "APPROVED".to_string();
    pr.worktree_path = Some("/path/wt".to_string());
    pr.communities_touched = vec![1, 2];
    pr.nodes_affected = 5;
    pr.files_changed = (0..15).map(|i| format!("file{i}.rs")).collect();
    render_pr_detail(&pr);
}

#[test]
fn format_prs_text_with_data() {
    let prs = vec![make_pr(1, "main", 1), make_pr(2, "develop", 1)];
    let text = format_prs_text(&prs, "main");
    assert!(text.contains("Open PRs"));
    assert!(text.contains("#1"));
    assert!(text.contains("(1 on wrong base"));
}

#[test]
fn format_prs_text_empty() {
    let text = format_prs_text(&[], "main");
    assert!(text.contains("Open PRs"));
}

// ── triage.rs ───────────────────────────────────────────────────────────────

#[test]
fn build_triage_prompt_includes_pr_data() {
    let pr = make_pr(7, "main", 1);
    let prompt = build_triage_prompt(&[&pr]);
    assert!(prompt.contains("#7"));
    assert!(prompt.contains("title:"));
    assert!(prompt.contains("rank them"));
}

#[test]
fn noop_triage_returns_ok() {
    let pr = make_pr(1, "main", 1);
    let backend = NoOpTriageBackend;
    backend.triage(&[&pr], "irrelevant").expect("noop ok");
}

// ── detect.rs / run.rs with fake clients ────────────────────────────────────

struct FakeGh {
    list_response: Option<Vec<u8>>,
    default_branch: Option<String>,
    files: Vec<String>,
}

impl GhClient for FakeGh {
    fn pr_list(&self, _repo: Option<&str>, _limit: usize) -> Result<Vec<u8>, PrsError> {
        self.list_response
            .clone()
            .ok_or_else(|| PrsError::GhFailed("no response".into()))
    }
    fn repo_default_branch(&self, _repo: Option<&str>) -> Option<String> {
        self.default_branch.clone()
    }
    fn pr_files(&self, _number: u64, _repo: Option<&str>) -> Vec<String> {
        self.files.clone()
    }
}

struct FakeGit {
    symbolic: Option<String>,
    worktrees: Option<String>,
}

impl GitClient for FakeGit {
    fn symbolic_ref_origin_head(&self) -> Option<String> {
        self.symbolic.clone()
    }
    fn worktree_list_porcelain(&self) -> Option<String> {
        self.worktrees.clone()
    }
}

#[test]
fn detect_default_branch_uses_gh_when_available() {
    let gh = FakeGh {
        list_response: None,
        default_branch: Some("trunk".into()),
        files: vec![],
    };
    let git = FakeGit {
        symbolic: None,
        worktrees: None,
    };
    assert_eq!(detect_default_branch(&gh, &git, None), "trunk");
}

#[test]
fn detect_default_branch_falls_back_to_git_symbolic() {
    let gh = FakeGh {
        list_response: None,
        default_branch: None,
        files: vec![],
    };
    let git = FakeGit {
        symbolic: Some("refs/remotes/origin/develop".into()),
        worktrees: None,
    };
    assert_eq!(detect_default_branch(&gh, &git, None), "develop");
}

#[test]
fn detect_default_branch_falls_back_to_main() {
    let gh = FakeGh {
        list_response: None,
        default_branch: None,
        files: vec![],
    };
    let git = FakeGit {
        symbolic: None,
        worktrees: None,
    };
    assert_eq!(detect_default_branch(&gh, &git, None), "main");
}

#[test]
fn detect_default_branch_empty_symbolic_falls_back() {
    let gh = FakeGh {
        list_response: None,
        default_branch: None,
        files: vec![],
    };
    let git = FakeGit {
        // Empty string → branch_from_symbolic_ref returns None.
        symbolic: Some(String::new()),
        worktrees: None,
    };
    assert_eq!(detect_default_branch(&gh, &git, None), "main");
}

// ── run_cmd_prs end-to-end ──────────────────────────────────────────────────

fn empty_pr_list_bytes() -> Vec<u8> {
    b"[]".to_vec()
}

#[test]
fn run_cmd_prs_dashboard_with_no_prs() {
    let gh = FakeGh {
        list_response: Some(empty_pr_list_bytes()),
        default_branch: Some("main".into()),
        files: vec![],
    };
    let git = FakeGit {
        symbolic: None,
        worktrees: None,
    };
    let triage = NoOpTriageBackend;
    let args = PrsArgs::default();
    run_cmd_prs(&gh, &git, &triage, &args).expect("run should succeed");
}

#[test]
fn run_cmd_prs_worktrees_view() {
    let gh = FakeGh {
        list_response: Some(empty_pr_list_bytes()),
        default_branch: Some("main".into()),
        files: vec![],
    };
    let git = FakeGit {
        symbolic: None,
        worktrees: Some(
            "worktree /tmp/wt\nHEAD abc123\nbranch refs/heads/feature-1\n\n".to_string(),
        ),
    };
    let triage = NoOpTriageBackend;
    let args = PrsArgs {
        do_worktrees: true,
        ..PrsArgs::default()
    };
    run_cmd_prs(&gh, &git, &triage, &args).expect("test invariant");
}

#[test]
fn run_cmd_prs_conflicts_view() {
    let gh = FakeGh {
        list_response: Some(empty_pr_list_bytes()),
        default_branch: Some("main".into()),
        files: vec![],
    };
    let git = FakeGit {
        symbolic: None,
        worktrees: None,
    };
    let triage = NoOpTriageBackend;
    let args = PrsArgs {
        do_conflicts: true,
        ..PrsArgs::default()
    };
    run_cmd_prs(&gh, &git, &triage, &args).expect("test invariant");
}

#[test]
fn run_cmd_prs_triage_view() {
    let gh = FakeGh {
        list_response: Some(empty_pr_list_bytes()),
        default_branch: Some("main".into()),
        files: vec![],
    };
    let git = FakeGit {
        symbolic: None,
        worktrees: None,
    };
    let triage = NoOpTriageBackend;
    let args = PrsArgs {
        do_triage: true,
        ..PrsArgs::default()
    };
    run_cmd_prs(&gh, &git, &triage, &args).expect("test invariant");
}

#[test]
fn run_cmd_prs_pr_not_found() {
    let gh = FakeGh {
        list_response: Some(empty_pr_list_bytes()),
        default_branch: Some("main".into()),
        files: vec![],
    };
    let git = FakeGit {
        symbolic: None,
        worktrees: None,
    };
    let triage = NoOpTriageBackend;
    let args = PrsArgs {
        pr_number: Some(999),
        ..PrsArgs::default()
    };
    let result = run_cmd_prs(&gh, &git, &triage, &args);
    assert!(matches!(result, Err(PrsError::PrNotFound(999))));
}

#[test]
fn run_cmd_prs_with_base_override() {
    let gh = FakeGh {
        list_response: Some(empty_pr_list_bytes()),
        default_branch: None,
        files: vec![],
    };
    let git = FakeGit {
        symbolic: None,
        worktrees: None,
    };
    let triage = NoOpTriageBackend;
    let args = PrsArgs {
        base: Some("v8".into()),
        ..PrsArgs::default()
    };
    run_cmd_prs(&gh, &git, &triage, &args).expect("test invariant");
}
