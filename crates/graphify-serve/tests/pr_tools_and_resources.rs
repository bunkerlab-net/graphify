//! Coverage tests for the PR-related tool handlers (`tool_list_prs`,
//! `tool_get_pr_impact`, `tool_triage_prs`) and resource renderers
//! (`resource_audit`, `resource_surprises`, `resource_questions`).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use graphify_build::{Graph, build_from_json};
use graphify_prs::error::PrsError;
use graphify_prs::gh::GhClient;
use graphify_prs::git::GitClient;
use graphify_serve::tools::{
    load_community_labels, resource_audit, resource_questions, resource_surprises,
    tool_get_pr_impact_with_clients, tool_list_prs_with_clients, tool_triage_prs_with_clients,
};
use indexmap::IndexMap;
use serde_json::json;

// ── Fake clients ────────────────────────────────────────────────────────────

struct FakeGh {
    prs: Vec<u8>,
    files: Vec<String>,
    default_branch: Option<String>,
}

impl GhClient for FakeGh {
    fn pr_list(&self, _repo: Option<&str>, _limit: usize) -> Result<Vec<u8>, PrsError> {
        Ok(self.prs.clone())
    }
    fn repo_default_branch(&self, _repo: Option<&str>) -> Option<String> {
        self.default_branch.clone()
    }
    fn pr_files(&self, _number: u64, _repo: Option<&str>) -> Vec<String> {
        self.files.clone()
    }
}

struct FakeGit;

impl GitClient for FakeGit {
    fn symbolic_ref_origin_head(&self) -> Option<String> {
        None
    }
    fn worktree_list_porcelain(&self) -> Option<String> {
        None
    }
}

fn canned_prs() -> Vec<u8> {
    br#"[
        {
            "number": 42,
            "title": "feat: alpha",
            "headRefName": "feat/alpha",
            "baseRefName": "main",
            "author": {"login": "alice"},
            "isDraft": false,
            "reviewDecision": "APPROVED",
            "statusCheckRollup": [{"conclusion": "SUCCESS", "status": "COMPLETED"}],
            "updatedAt": "2025-01-01T00:00:00Z"
        },
        {
            "number": 7,
            "title": "wip",
            "headRefName": "wip",
            "baseRefName": "main",
            "author": {"login": "bob"},
            "isDraft": true,
            "reviewDecision": "",
            "statusCheckRollup": [{"conclusion": "FAILURE", "status": "COMPLETED"}],
            "updatedAt": "2025-01-01T00:00:00Z"
        }
    ]"#
    .to_vec()
}

fn small_graph() -> Graph {
    build_from_json(
        json!({
            "nodes": [
                {"id": "n1", "label": "A", "source_file": "src/a.py", "file_type": "code", "community": 0},
                {"id": "n2", "label": "B", "source_file": "src/b.py", "file_type": "code", "community": 0},
                {"id": "n3", "label": "C", "source_file": "src/c.py", "file_type": "code", "community": 1},
            ],
            "edges": [
                {"source": "n1", "target": "n2", "relation": "calls", "confidence": "EXTRACTED"},
                {"source": "n2", "target": "n3", "relation": "uses", "confidence": "INFERRED"},
            ]
        }),
        false,
        None,
    )
    .unwrap()
}

// ── tool_list_prs ──────────────────────────────────────────────────────────

#[test]
fn tool_list_prs_returns_count_and_prs() {
    let gh = FakeGh {
        prs: canned_prs(),
        files: vec![],
        default_branch: Some("main".into()),
    };
    let git = FakeGit;
    let args = json!({});
    let v = tool_list_prs_with_clients(&args, &gh, &git).unwrap();
    assert_eq!(v["count"], 2);
    assert!(v["prs"].as_array().unwrap().len() == 2);
}

#[test]
fn tool_list_prs_with_repo_and_base() {
    let gh = FakeGh {
        prs: canned_prs(),
        files: vec![],
        default_branch: Some("main".into()),
    };
    let git = FakeGit;
    let args = json!({"repo": "owner/repo", "base": "main", "limit": 1});
    let v = tool_list_prs_with_clients(&args, &gh, &git).unwrap();
    assert!(v["count"].as_u64().unwrap() >= 1);
}

#[test]
fn tool_list_prs_propagates_error() {
    struct FailingGh;
    impl GhClient for FailingGh {
        fn pr_list(&self, _: Option<&str>, _: usize) -> Result<Vec<u8>, PrsError> {
            Err(PrsError::GhFailed("boom".into()))
        }
        fn repo_default_branch(&self, _: Option<&str>) -> Option<String> {
            None
        }
        fn pr_files(&self, _: u64, _: Option<&str>) -> Vec<String> {
            vec![]
        }
    }
    let res = tool_list_prs_with_clients(&json!({}), &FailingGh, &FakeGit);
    assert!(res.is_err());
}

// ── tool_get_pr_impact ─────────────────────────────────────────────────────

#[test]
fn tool_get_pr_impact_returns_communities() {
    let gh = FakeGh {
        prs: canned_prs(),
        files: vec!["src/a.py".into(), "src/c.py".into()],
        default_branch: None,
    };
    let g = small_graph();
    let args = json!({"pr_number": 42});
    let v = tool_get_pr_impact_with_clients(&g, &args, &gh).unwrap();
    assert_eq!(v["pr_number"], 42);
    assert!(v["files_changed"].as_array().unwrap().len() == 2);
}

#[test]
fn tool_get_pr_impact_missing_pr_number() {
    let gh = FakeGh {
        prs: vec![],
        files: vec![],
        default_branch: None,
    };
    let g = small_graph();
    let args = json!({});
    assert!(tool_get_pr_impact_with_clients(&g, &args, &gh).is_err());
}

// ── tool_triage_prs ────────────────────────────────────────────────────────

#[test]
fn tool_triage_prs_returns_actionable() {
    let gh = FakeGh {
        prs: canned_prs(),
        files: vec![],
        default_branch: Some("main".into()),
    };
    let git = FakeGit;
    let args = json!({});
    let v = tool_triage_prs_with_clients(&args, &gh, &git).unwrap();
    assert!(v.is_array());
}

#[test]
fn tool_triage_prs_with_explicit_base() {
    let gh = FakeGh {
        prs: canned_prs(),
        files: vec![],
        default_branch: None,
    };
    let git = FakeGit;
    let args = json!({"base": "main", "limit": 5});
    let v = tool_triage_prs_with_clients(&args, &gh, &git).unwrap();
    assert!(v.is_array());
}

// ── resource_audit ─────────────────────────────────────────────────────────

#[test]
fn resource_audit_renders_percentages() {
    let g = small_graph();
    let out = resource_audit(&g);
    assert!(out.contains("Total edges"));
    assert!(out.contains("EXTRACTED"));
    assert!(out.contains("INFERRED"));
    assert!(out.contains("AMBIGUOUS"));
}

#[test]
fn resource_audit_handles_empty_graph() {
    let g = build_from_json(json!({"nodes": [], "edges": []}), false, None).unwrap();
    let out = resource_audit(&g);
    assert!(out.contains("Total edges"));
}

// ── resource_surprises ─────────────────────────────────────────────────────

#[test]
fn resource_surprises_renders_lines_or_empty_message() {
    let g = small_graph();
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec!["n1".into(), "n2".into()]);
    communities.insert(1, vec!["n3".into()]);
    let out = resource_surprises(&g, &communities);
    // Either lists surprises or returns the "no surprises" sentinel.
    assert!(
        out.starts_with("Surprising") || out == "No surprising connections found.",
        "unexpected output: {out}"
    );
}

// ── resource_questions ─────────────────────────────────────────────────────

#[test]
fn resource_questions_renders_lines_or_empty_message() {
    let g = small_graph();
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec!["n1".into(), "n2".into()]);
    communities.insert(1, vec!["n3".into()]);
    let labels: IndexMap<i64, String> = IndexMap::new();
    let out = resource_questions(&g, &communities, &labels);
    assert!(
        out.starts_with("Suggested") || out == "No suggested questions available.",
        "unexpected output: {out}"
    );
}

// ── load_community_labels ──────────────────────────────────────────────────

#[test]
fn load_community_labels_reads_existing_file() {
    let tmp = tempfile::tempdir().unwrap();
    let graph_path = tmp.path().join("graph.json");
    std::fs::write(&graph_path, "{}").unwrap();
    std::fs::write(
        tmp.path().join(".graphify_labels.json"),
        r#"{"0": "Auth", "1": "Worker"}"#,
    )
    .unwrap();

    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec![]);
    communities.insert(1, vec![]);

    let labels = load_community_labels(graph_path.to_str().unwrap(), &communities);
    assert_eq!(labels.get(&0).map(String::as_str), Some("Auth"));
    assert_eq!(labels.get(&1).map(String::as_str), Some("Worker"));
}

#[test]
fn load_community_labels_returns_empty_when_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let graph_path = tmp.path().join("graph.json");
    let communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    let labels = load_community_labels(graph_path.to_str().unwrap(), &communities);
    assert!(labels.is_empty());
}
