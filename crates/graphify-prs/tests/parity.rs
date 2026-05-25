//! Parity tests for `graphify-prs` — ports of `graphify-py/tests/test_prs.py`.
//!
//! All tests use injected fake clients; no live `gh` or `git` processes.

#![allow(clippy::expect_used)]

use std::collections::HashMap;

use chrono::{Duration, Utc};

use graphify_prs::{
    classify,
    dashboard::format_prs_text,
    error::PrsError,
    fetch_worktrees,
    gh::{GhClient, parse_pr_list},
    git::{GitClient, branch_from_symbolic_ref, parse_worktree_porcelain},
    graph::{build_community_labels, build_file_index, compute_pr_impact},
    model::{CheckRun, PrInfo, parse_ci, path_match},
};
use serde_json::{Value, json};

// ── Test helpers ─────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn make_pr(
    number: u64,
    title: &str,
    branch: &str,
    base_branch: &str,
    author: &str,
    is_draft: bool,
    review_decision: &str,
    ci_status: &str,
    days_ago: i64,
    expected_base: &str,
) -> PrInfo {
    let updated_at = Utc::now() - Duration::days(days_ago);
    PrInfo {
        number,
        title: title.to_string(),
        branch: branch.to_string(),
        base_branch: base_branch.to_string(),
        author: author.to_string(),
        is_draft,
        review_decision: review_decision.to_string(),
        ci_status: ci_status.to_string(),
        updated_at,
        expected_base: expected_base.to_string(),
        worktree_path: None,
        communities_touched: vec![],
        nodes_affected: 0,
        files_changed: vec![],
    }
}

fn default_pr() -> PrInfo {
    make_pr(
        1, "Test PR", "feature", "v8", "alice", false, "", "SUCCESS", 1, "v8",
    )
}

// ── Fake GhClient ─────────────────────────────────────────────────────────────

struct FakeGhClient {
    pr_list_response: Option<Vec<u8>>,
    default_branch: Option<String>,
    pr_files_response: Vec<String>,
}

impl GhClient for FakeGhClient {
    fn pr_list(&self, _repo: Option<&str>, _limit: usize) -> Result<Vec<u8>, PrsError> {
        self.pr_list_response
            .clone()
            .ok_or_else(|| PrsError::GhFailed("no response configured".to_string()))
    }

    fn repo_default_branch(&self, _repo: Option<&str>) -> Option<String> {
        self.default_branch.clone()
    }

    fn pr_files(&self, _number: u64, _repo: Option<&str>) -> Vec<String> {
        self.pr_files_response.clone()
    }
}

// ── Fake GitClient ────────────────────────────────────────────────────────────

struct FakeGitClient {
    worktree_porcelain: Option<String>,
    symbolic_ref: Option<String>,
}

impl GitClient for FakeGitClient {
    fn worktree_list_porcelain(&self) -> Option<String> {
        self.worktree_porcelain.clone()
    }

    fn symbolic_ref_origin_head(&self) -> Option<String> {
        self.symbolic_ref.clone()
    }
}

// ── _classify ─────────────────────────────────────────────────────────────────

#[test]
fn test_classify_ready() {
    let pr = default_pr();
    assert_eq!(classify(&pr, "v8"), "READY");
}

#[test]
fn test_classify_ci_fail() {
    let pr = make_pr(1, "T", "b", "v8", "a", false, "", "FAILURE", 1, "v8");
    assert_eq!(classify(&pr, "v8"), "CI-FAIL");
}

#[test]
fn test_classify_changes_req() {
    let pr = make_pr(
        1,
        "T",
        "b",
        "v8",
        "a",
        false,
        "CHANGES_REQUESTED",
        "SUCCESS",
        1,
        "v8",
    );
    assert_eq!(classify(&pr, "v8"), "CHANGES-REQ");
}

#[test]
fn test_classify_draft() {
    let pr = make_pr(1, "T", "b", "v8", "a", true, "", "SUCCESS", 1, "v8");
    assert_eq!(classify(&pr, "v8"), "DRAFT");
}

#[test]
fn test_classify_stale() {
    let pr = make_pr(1, "T", "b", "v8", "a", false, "", "SUCCESS", 20, "v8");
    assert_eq!(classify(&pr, "v8"), "STALE");
}

#[test]
fn test_classify_draft_not_marked_stale() {
    // Drafts show as DRAFT even when old.
    let pr = make_pr(1, "T", "b", "v8", "a", true, "", "SUCCESS", 20, "v8");
    assert_eq!(classify(&pr, "v8"), "DRAFT");
}

#[test]
fn test_classify_pending() {
    let pr = make_pr(1, "T", "b", "v8", "a", false, "", "PENDING", 1, "v8");
    assert_eq!(classify(&pr, "v8"), "PENDING");
}

#[test]
fn test_classify_wrong_base() {
    // WRONG-BASE takes precedence over everything.
    let pr = make_pr(1, "T", "b", "master", "a", false, "", "FAILURE", 1, "v8");
    assert_eq!(classify(&pr, "v8"), "WRONG-BASE");
}

// ── _parse_ci ─────────────────────────────────────────────────────────────────

fn check_run(conclusion: Option<&str>, status: &str) -> CheckRun {
    CheckRun {
        conclusion: conclusion.map(str::to_string),
        status: Some(status.to_string()),
    }
}

#[test]
fn test_parse_ci_empty_returns_none() {
    assert_eq!(parse_ci(&[]), "NONE");
}

#[test]
fn test_parse_ci_failure() {
    let rollup = vec![check_run(Some("FAILURE"), "COMPLETED")];
    assert_eq!(parse_ci(&rollup), "FAILURE");
}

#[test]
fn test_parse_ci_cancelled_is_failure() {
    let rollup = vec![check_run(Some("CANCELLED"), "COMPLETED")];
    assert_eq!(parse_ci(&rollup), "FAILURE");
}

#[test]
fn test_parse_ci_timed_out_is_failure() {
    let rollup = vec![check_run(Some("TIMED_OUT"), "COMPLETED")];
    assert_eq!(parse_ci(&rollup), "FAILURE");
}

#[test]
fn test_parse_ci_in_progress_is_pending() {
    let rollup = vec![check_run(None, "IN_PROGRESS")];
    assert_eq!(parse_ci(&rollup), "PENDING");
}

#[test]
fn test_parse_ci_success() {
    let rollup = vec![check_run(Some("SUCCESS"), "COMPLETED")];
    assert_eq!(parse_ci(&rollup), "SUCCESS");
}

#[test]
fn test_parse_ci_mixed_success_and_failure_is_failure() {
    let rollup = vec![
        check_run(Some("SUCCESS"), "COMPLETED"),
        check_run(Some("FAILURE"), "COMPLETED"),
    ];
    assert_eq!(parse_ci(&rollup), "FAILURE");
}

// ── _path_match ───────────────────────────────────────────────────────────────

#[test]
fn test_path_match_exact() {
    assert!(path_match("src/auth/api.py", "src/auth/api.py"));
}

#[test]
fn test_path_match_graph_path_longer() {
    assert!(path_match("src/auth/api.py", "api.py"));
}

#[test]
fn test_path_match_no_false_positive_on_partial_filename() {
    assert!(!path_match("config.py", "g.py"));
    assert!(!path_match("g.py", "config.py"));
}

#[test]
fn test_path_match_both_directions() {
    assert!(path_match("api.py", "src/auth/api.py"));
    assert!(path_match("src/auth/api.py", "api.py"));
}

// ── compute_pr_impact ─────────────────────────────────────────────────────────

fn make_graph_nodes() -> Vec<Value> {
    vec![
        json!({"source_file": "src/auth/api.py", "community": 0}),
        json!({"source_file": "src/auth/api.py", "community": 0}),
        json!({"source_file": "src/utils/helpers.py", "community": 1}),
    ]
}

#[test]
fn test_compute_pr_impact_single_file() {
    let nodes = make_graph_nodes();
    let index = build_file_index(&nodes);
    let (comms, n) = compute_pr_impact(&["src/auth/api.py".to_string()], &index);
    assert_eq!(comms, vec![0]);
    assert_eq!(n, 2);
}

#[test]
fn test_compute_pr_impact_both_files() {
    let nodes = make_graph_nodes();
    let index = build_file_index(&nodes);
    let (comms, n) = compute_pr_impact(
        &[
            "src/auth/api.py".to_string(),
            "src/utils/helpers.py".to_string(),
        ],
        &index,
    );
    assert_eq!(comms, vec![0, 1]);
    assert_eq!(n, 3);
}

#[test]
fn test_compute_pr_impact_empty_files() {
    let nodes = make_graph_nodes();
    let index = build_file_index(&nodes);
    let (comms, n) = compute_pr_impact(&[], &index);
    assert_eq!(comms, Vec::<i64>::new());
    assert_eq!(n, 0);
}

#[test]
fn test_compute_pr_impact_no_matching_files() {
    let nodes = make_graph_nodes();
    let index = build_file_index(&nodes);
    let (comms, n) = compute_pr_impact(&["docs/README.md".to_string()], &index);
    assert_eq!(comms, Vec::<i64>::new());
    assert_eq!(n, 0);
}

#[test]
fn test_compute_pr_impact_no_double_counting_distinct_paths() {
    // "src/auth/api.py" should NOT match "src/admin/api.py" via exact path.
    let nodes = vec![
        json!({"source_file": "src/auth/api.py", "community": 0}),
        json!({"source_file": "src/admin/api.py", "community": 1}),
    ];
    let index = build_file_index(&nodes);
    let (comms, n) = compute_pr_impact(&["src/auth/api.py".to_string()], &index);
    assert_eq!(n, 1);
    assert_eq!(comms, vec![0]);
}

#[test]
fn test_compute_pr_impact_no_double_counting_same_graph_file() {
    // If PR diff lists both "api.py" and "src/auth/api.py", the graph node for
    // src/auth/api.py should only be counted once.
    let nodes = vec![
        json!({"source_file": "src/auth/api.py", "community": 0}),
        json!({"source_file": "src/auth/api.py", "community": 0}),
    ];
    let index = build_file_index(&nodes);
    let (comms, n) = compute_pr_impact(
        &["src/auth/api.py".to_string(), "api.py".to_string()],
        &index,
    );
    assert_eq!(n, 2); // 2 nodes, counted once
    assert_eq!(comms, vec![0]);
}

// ── fetch_worktrees ───────────────────────────────────────────────────────────

#[test]
fn test_fetch_worktrees_normal_case() {
    let porcelain = "worktree /home/user/proj\n\
                     HEAD abc123\n\
                     branch refs/heads/main\n\
                     \n\
                     worktree /home/user/proj-feature\n\
                     HEAD def456\n\
                     branch refs/heads/feature-x\n\
                     \n";
    let client = FakeGitClient {
        worktree_porcelain: Some(porcelain.to_string()),
        symbolic_ref: None,
    };
    let mapping = fetch_worktrees(&client);
    let mut expected = HashMap::new();
    expected.insert("main".to_string(), "/home/user/proj".to_string());
    expected.insert(
        "feature-x".to_string(),
        "/home/user/proj-feature".to_string(),
    );
    assert_eq!(mapping, expected);
}

#[test]
fn test_fetch_worktrees_detached_head_does_not_leak() {
    let porcelain = "worktree /home/user/detached\n\
                     HEAD abc123\n\
                     detached\n\
                     \n\
                     worktree /home/user/proj-feature\n\
                     HEAD def456\n\
                     branch refs/heads/feature-x\n\
                     \n";
    let client = FakeGitClient {
        worktree_porcelain: Some(porcelain.to_string()),
        symbolic_ref: None,
    };
    let mapping = fetch_worktrees(&client);
    let mut expected = HashMap::new();
    expected.insert(
        "feature-x".to_string(),
        "/home/user/proj-feature".to_string(),
    );
    assert_eq!(mapping, expected);
    assert!(!mapping.values().any(|v| v == "/home/user/detached"));
}

#[test]
fn test_fetch_worktrees_empty_output() {
    let client = FakeGitClient {
        worktree_porcelain: Some(String::new()),
        symbolic_ref: None,
    };
    assert!(fetch_worktrees(&client).is_empty());
}

#[test]
fn test_fetch_worktrees_client_returns_none() {
    let client = FakeGitClient {
        worktree_porcelain: None,
        symbolic_ref: None,
    };
    assert!(fetch_worktrees(&client).is_empty());
}

// ── parse_worktree_porcelain (unit) ───────────────────────────────────────────

#[test]
fn test_parse_worktree_porcelain_direct() {
    let text = "worktree /home/user/proj\nHEAD abc\nbranch refs/heads/main\n\n";
    let m = parse_worktree_porcelain(text);
    assert_eq!(m.get("main"), Some(&"/home/user/proj".to_string()));
}

// ── branch_from_symbolic_ref ──────────────────────────────────────────────────

#[test]
fn test_branch_from_symbolic_ref_main() {
    assert_eq!(
        branch_from_symbolic_ref("refs/remotes/origin/main\n"),
        Some("main".to_string())
    );
}

#[test]
fn test_branch_from_symbolic_ref_develop() {
    assert_eq!(
        branch_from_symbolic_ref("refs/remotes/origin/develop\n"),
        Some("develop".to_string())
    );
}

#[test]
fn test_branch_from_symbolic_ref_empty() {
    assert_eq!(branch_from_symbolic_ref(""), None);
    assert_eq!(branch_from_symbolic_ref("   \n"), None);
}

// ── detect_default_branch ────────────────────────────────────────────────────

#[test]
fn test_detect_default_branch_gh_returns_main() {
    use graphify_prs::detect_default_branch;
    let gh = FakeGhClient {
        pr_list_response: None,
        default_branch: Some("main".to_string()),
        pr_files_response: vec![],
    };
    let git = FakeGitClient {
        worktree_porcelain: None,
        symbolic_ref: None,
    };
    assert_eq!(detect_default_branch(&gh, &git, None), "main");
}

#[test]
fn test_detect_default_branch_falls_back_to_git() {
    use graphify_prs::detect_default_branch;
    let gh = FakeGhClient {
        pr_list_response: None,
        default_branch: None,
        pr_files_response: vec![],
    };
    let git = FakeGitClient {
        worktree_porcelain: None,
        symbolic_ref: Some("refs/remotes/origin/develop\n".to_string()),
    };
    assert_eq!(detect_default_branch(&gh, &git, None), "develop");
}

#[test]
fn test_detect_default_branch_both_fail_returns_main() {
    use graphify_prs::detect_default_branch;
    let gh = FakeGhClient {
        pr_list_response: None,
        default_branch: None,
        pr_files_response: vec![],
    };
    let git = FakeGitClient {
        worktree_porcelain: None,
        symbolic_ref: None,
    };
    assert_eq!(detect_default_branch(&gh, &git, None), "main");
}

#[test]
fn test_detect_default_branch_gh_returns_empty_dict_falls_back() {
    use graphify_prs::detect_default_branch;
    let gh = FakeGhClient {
        pr_list_response: None,
        default_branch: None, // gh returns no branch (empty / missing)
        pr_files_response: vec![],
    };
    let git = FakeGitClient {
        worktree_porcelain: None,
        symbolic_ref: Some("refs/remotes/origin/trunk\n".to_string()),
    };
    assert_eq!(detect_default_branch(&gh, &git, None), "trunk");
}

// ── format_prs_text ───────────────────────────────────────────────────────────

#[test]
fn test_format_prs_text_contains_metadata_and_count() {
    let prs = vec![
        make_pr(
            101,
            "Add awesome feature",
            "feat",
            "v8",
            "alice",
            false,
            "",
            "SUCCESS",
            1,
            "v8",
        ),
        make_pr(
            102,
            "Fix flaky test",
            "fix",
            "v8",
            "bob",
            false,
            "",
            "FAILURE",
            1,
            "v8",
        ),
        make_pr(
            103,
            "Wrong base PR",
            "wbr",
            "master",
            "charlie",
            false,
            "",
            "SUCCESS",
            1,
            "v8",
        ),
    ];
    let out = format_prs_text(&prs, "v8");

    assert!(out.contains("Open PRs targeting v8: 2"));
    assert!(out.contains("(1 on wrong base, not shown)"));
    assert!(out.contains("#101"));
    assert!(out.contains("Add awesome feature"));
    assert!(out.contains("#102"));
    assert!(out.contains("Fix flaky test"));
    assert!(out.contains("[READY]"));
    assert!(out.contains("[CI-FAIL]"));
    // Wrong-base PR must be filtered out.
    assert!(!out.contains("#103"));
}

#[test]
fn test_format_prs_text_empty() {
    let out = format_prs_text(&[], "v8");
    assert!(out.contains("Open PRs targeting v8: 0"));
    assert!(out.contains("(0 on wrong base, not shown)"));
}

// ── build_community_labels ────────────────────────────────────────────────────

#[test]
fn test_build_community_labels_basic() {
    let data = json!({
        "nodes": [
            {"id": "a", "label": "Alpha", "community": 0},
            {"id": "b", "label": "Beta",  "community": 0},
            {"id": "c", "label": "Gamma", "community": 1},
        ]
    });
    let labels = build_community_labels(&data, 4);
    let comm0: std::collections::HashSet<&str> = labels[&0].iter().map(String::as_str).collect();
    assert_eq!(comm0, ["Alpha", "Beta"].iter().copied().collect());
    assert_eq!(labels[&1], vec!["Gamma"]);
}

#[test]
fn test_build_community_labels_top_n_capped() {
    let nodes: Vec<Value> = (0..10_i64)
        .map(|i| json!({"id": i.to_string(), "label": format!("Node{i}"), "community": 0}))
        .collect();
    let data = json!({"nodes": nodes});
    let labels = build_community_labels(&data, 4);
    assert_eq!(labels[&0].len(), 4);
}

#[test]
fn test_build_community_labels_no_community_field_skipped() {
    let data = json!({"nodes": [{"id": "x", "label": "X"}]});
    assert!(build_community_labels(&data, 4).is_empty());
}

#[test]
fn test_build_community_labels_empty_nodes() {
    let data1 = json!({});
    let data2 = json!({"nodes": []});
    assert!(build_community_labels(&data1, 4).is_empty());
    assert!(build_community_labels(&data2, 4).is_empty());
}

// ── parse_pr_list JSON deserialization ────────────────────────────────────────

#[test]
fn test_parse_pr_list_basic() {
    let json_bytes = br#"[
        {
            "number": 42,
            "title": "Add thing",
            "headRefName": "feat/thing",
            "baseRefName": "main",
            "author": {"login": "alice"},
            "isDraft": false,
            "reviewDecision": "APPROVED",
            "statusCheckRollup": [{"conclusion": "SUCCESS", "status": "COMPLETED"}],
            "updatedAt": "2024-01-15T10:00:00Z"
        }
    ]"#;
    let prs = parse_pr_list(json_bytes, "main").expect("test invariant");
    assert_eq!(prs.len(), 1);
    let pr = &prs[0];
    assert_eq!(pr.number, 42);
    assert_eq!(pr.title, "Add thing");
    assert_eq!(pr.author, "alice");
    assert_eq!(pr.review_decision, "APPROVED");
    assert_eq!(pr.ci_status, "SUCCESS");
}

#[test]
fn test_parse_pr_list_null_author() {
    let json_bytes = br#"[
        {
            "number": 1,
            "title": "T",
            "headRefName": "b",
            "baseRefName": "main",
            "author": null,
            "isDraft": false,
            "reviewDecision": null,
            "statusCheckRollup": null,
            "updatedAt": "2024-01-15T10:00:00Z"
        }
    ]"#;
    let prs = parse_pr_list(json_bytes, "main").expect("test invariant");
    assert_eq!(prs[0].author, "?");
    assert_eq!(prs[0].review_decision, "");
    assert_eq!(prs[0].ci_status, "NONE");
}
