//! Parity tests for `graphify-dedup`.
//!
//! Ports every test in `graphify-py/tests/test_dedup.py`.

#![allow(clippy::expect_used, clippy::float_cmp)] // test file

use graphify_dedup::{
    DedupLlmBackend, JudgeResult, NoOpBackend, deduplicate_entities, entropy, is_variant_pair,
    norm, shingles, short_label_blocked,
};
use indexmap::IndexMap;
use serde_json::{Value, json};

// ── helpers ───────────────────────────────────────────────────────────────────

fn make_nodes(labels: &[&str]) -> Vec<Value> {
    labels
        .iter()
        .map(|label| {
            let id = label.to_lowercase().replace(' ', "_");
            json!({
                "id": id,
                "label": label,
                "source_file": "test.md"
            })
        })
        .collect()
}

fn empty_communities() -> IndexMap<String, i64> {
    IndexMap::new()
}

// ── entropy gate ──────────────────────────────────────────────────────────────

#[test]
fn test_entropy_short_label_low() {
    assert!(entropy("AI") < 2.5);
}

#[test]
fn test_entropy_normal_label_high() {
    assert!(entropy("AuthenticationManager") >= 2.5);
}

#[test]
fn test_entropy_empty_string() {
    assert_eq!(entropy(""), 0.0);
}

// ── norm: NFKC + Unicode-aware (#937) ────────────────────────────────────────

#[test]
fn norm_preserves_cjk_word_chars() {
    // CJK letters must survive the non-word collapse — they used to be stripped
    // by the old `[^a-z0-9]+` regex, which silently zero-ed the dedup key.
    assert_eq!(norm("認証"), "認証");
    assert_eq!(norm("身份验证 API"), "身份验证 api");
}

#[test]
fn norm_collapses_underscores_and_punctuation() {
    assert_eq!(norm("foo___bar"), "foo bar");
    assert_eq!(norm("foo--bar"), "foo bar");
}

#[test]
fn norm_nfkc_normalizes_fullwidth() {
    // Full-width Latin A "Ａ" (U+FF21) folds to ASCII "a" under NFKC + lower.
    assert_eq!(norm("ＡＢＣ"), "abc");
}

#[test]
fn norm_casefold_matches_python_for_german_sharp_s() {
    // Python's str.casefold() maps "ß" -> "ss"; str::to_lowercase() does
    // not. Pinning the casefold contract here so a regression to
    // `to_lowercase` is caught immediately.
    assert_eq!(norm("Straße"), "strasse");
}

// ── shingles ─────────────────────────────────────────────────────────────────

#[test]
fn test_shingles_produces_trigrams() {
    let s = shingles("hello", 3);
    assert!(s.contains(&"hel".to_string()));
    assert!(s.contains(&"ell".to_string()));
    assert!(s.contains(&"llo".to_string()));
}

#[test]
fn test_shingles_short_string() {
    let s = shingles("ab", 3);
    assert_eq!(s, vec!["ab".to_string()]);
}

// ── full pipeline ─────────────────────────────────────────────────────────────

#[test]
fn test_exact_duplicates_merged() {
    let nodes = make_nodes(&["UserService", "userservice", "User Service"]);
    let (result_nodes, _result_edges) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 1);
}

#[test]
fn test_typo_merged() {
    let nodes = make_nodes(&["GraphExtractor", "Graph Extractor"]);
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 1);
}

#[test]
fn test_unrelated_not_merged() {
    let nodes = make_nodes(&["UserService", "OrderService"]);
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 2);
}

#[test]
fn test_short_low_entropy_not_merged() {
    let nodes = make_nodes(&["AI", "ML"]);
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 2);
}

#[test]
fn test_edges_rewired_after_merge() {
    let nodes = make_nodes(&["GraphExtractor", "Graph Extractor", "Parser"]);
    let edges = vec![json!({
        "source": "graph_extractor",
        "target": "parser",
        "relation": "uses"
    })];
    let (result_nodes, result_edges) =
        deduplicate_entities(&nodes, &edges, &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 2); // merged + Parser
    assert_eq!(result_edges.len(), 1); // edge rewired, still present
}

#[test]
fn test_self_loops_dropped_after_merge() {
    let nodes = make_nodes(&["GraphExtractor", "Graph Extractor"]);
    let edges = vec![json!({
        "source": "graphextractor",
        "target": "graph_extractor",
        "relation": "same"
    })];
    let (_, result_edges) =
        deduplicate_entities(&nodes, &edges, &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_edges, Vec::<Value>::new());
}

#[test]
fn test_community_boost_aids_merge() {
    let nodes = make_nodes(&["AuthManager", "Auth Manager"]);
    let mut communities_same = IndexMap::new();
    communities_same.insert("authmanager".to_string(), 1i64);
    communities_same.insert("auth_manager".to_string(), 1i64);
    let (result_with, _) =
        deduplicate_entities(&nodes, &[], &communities_same, None).expect("dedup ok");

    let mut communities_diff = IndexMap::new();
    communities_diff.insert("authmanager".to_string(), 1i64);
    communities_diff.insert("auth_manager".to_string(), 2i64);
    let (result_without, _) =
        deduplicate_entities(&nodes, &[], &communities_diff, None).expect("dedup ok");

    assert!(result_with.len() <= result_without.len());
}

#[test]
fn test_empty_inputs() {
    let (result_nodes, result_edges) =
        deduplicate_entities(&[], &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes, Vec::<Value>::new());
    assert_eq!(result_edges, Vec::<Value>::new());
}

#[test]
fn test_single_node_no_crash() {
    let nodes = make_nodes(&["UserService"]);
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 1);
}

#[test]
fn test_dedup_llm_flag_accepted() {
    let nodes = make_nodes(&["UserService", "OrderService"]);
    let backend = NoOpBackend;
    let (result_nodes, _) = deduplicate_entities(
        &nodes,
        &[],
        &empty_communities(),
        Some(&backend as &dyn DedupLlmBackend),
    )
    .expect("dedup ok");
    assert_eq!(result_nodes.len(), 2);
}

// ── #878 regression: fuzzy false merges on short/variant labels ───────────────

#[test]
fn test_dedup_does_not_merge_numeric_variants() {
    let nodes = make_nodes(&["ASR1603", "ASR1605"]);
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(
        result_nodes.len(),
        2,
        "ASR1603 and ASR1605 are distinct chip models"
    );
}

#[test]
fn test_dedup_does_not_merge_short_insertion_variants() {
    let nodes = make_nodes(&["cranel", "cranelr"]);
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 2, "cranel and cranelr are distinct");
}

#[test]
fn test_dedup_does_not_merge_model_with_suffix() {
    let nodes = make_nodes(&["M1", "M1 Pro"]);
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(
        result_nodes.len(),
        2,
        "M1 and M1 Pro are distinct Apple chip variants"
    );
}

#[test]
fn test_dedup_still_merges_real_typos() {
    let a = "graphextractor";
    let b = "graphextractar";
    let score = strsim::jaro_winkler(a, b) * 100.0;
    assert!(!is_variant_pair(a, b), "not a variant pair");
    assert!(
        !short_label_blocked(a, b, score),
        "long enough, should not be blocked"
    );
}

#[test]
fn test_variant_pair_helper() {
    assert!(is_variant_pair("asr1603", "asr1605"));
    assert!(is_variant_pair("cortex a55", "cortex a55x"));
    assert!(!is_variant_pair("graphextractor", "graphextracter"));
    assert!(!is_variant_pair("foo", "foo"));
}

// ── LLM backend injection ─────────────────────────────────────────────────────

/// A fake backend that always votes "Merge".
struct AlwaysMerge;

impl DedupLlmBackend for AlwaysMerge {
    fn judge(&self, _a: &str, _b: &str) -> JudgeResult {
        JudgeResult::Merge
    }
}

#[test]
fn test_llm_backend_merge_verdict_applied() {
    // "AuthService" vs "Auth Service" score is in the LLM zone for the mock.
    // With AlwaysMerge, they should be merged.
    let nodes = make_nodes(&["AuthService", "Auth Service"]);
    let backend = AlwaysMerge;
    let (result_with_llm, _) = deduplicate_entities(
        &nodes,
        &[],
        &empty_communities(),
        Some(&backend as &dyn DedupLlmBackend),
    )
    .expect("dedup ok");

    // No-op backend should produce ≥ as many nodes.
    let noop = NoOpBackend;
    let (result_noop, _) = deduplicate_entities(
        &nodes,
        &[],
        &empty_communities(),
        Some(&noop as &dyn DedupLlmBackend),
    )
    .expect("dedup ok");

    assert!(result_with_llm.len() <= result_noop.len());
}

#[test]
fn test_llm_backend_distinct_verdict_preserves_nodes() {
    struct AlwaysDistinct;
    impl DedupLlmBackend for AlwaysDistinct {
        fn judge(&self, _a: &str, _b: &str) -> JudgeResult {
            JudgeResult::Distinct
        }
    }

    // Use two ambiguous-ish labels and ensure Distinct keeps them separate.
    let nodes = make_nodes(&["UserService", "OrderService"]);
    let backend = AlwaysDistinct;
    let (result_nodes, _) = deduplicate_entities(
        &nodes,
        &[],
        &empty_communities(),
        Some(&backend as &dyn DedupLlmBackend),
    )
    .expect("dedup ok");
    assert_eq!(result_nodes.len(), 2);
}

#[test]
fn test_multiple_repos_error() {
    let nodes = vec![
        json!({"id": "a", "label": "A", "repo": "repo1"}),
        json!({"id": "b", "label": "B", "repo": "repo2"}),
    ];
    let result = deduplicate_entities(&nodes, &[], &empty_communities(), None);
    assert!(result.is_err());
    let err = result.expect_err("expected Err");
    let msg = err.to_string();
    assert!(msg.contains("multiple repos"), "error message was: {msg}");
}

#[test]
fn test_identical_labels_in_different_files_not_merged() {
    // Regression guard for graphify-py #1046: two high-entropy nodes that share
    // an identical label but live in different source files are distinct symbols
    // (e.g. a trait impl and its wrapper) and must NOT be merged. Pass 1
    // partitions by `source_file`, and Pass 2's unique-by-norm candidate set
    // prevents cross-file collapse of identical labels.
    let nodes = vec![
        json!({
            "id": "a_authenticateusersession",
            "label": "AuthenticateUserSession",
            "source_file": "auth/a.rs"
        }),
        json!({
            "id": "b_authenticateusersession",
            "label": "AuthenticateUserSession",
            "source_file": "auth/b.rs"
        }),
    ];
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(
        result_nodes.len(),
        2,
        "identical labels in different files must not be merged"
    );
}

// ── #1201 / #1247: prefix-extension guard + verified-pair winner ────────────

#[test]
fn prefix_extension_symbols_not_merged() {
    // Strict prefix-extension pairs score very high JW but are distinct symbols.
    let pairs = [
        ("getActiveSession", "getActiveSessions"),
        ("parseConfig", "parseConfigFile"),
        ("load", "loadAll"),
        ("handleRequest", "handleRequestTimeout"),
    ];
    for (a, b) in pairs {
        let nodes = vec![
            json!({"id": format!("{a}_id"), "label": a, "type": "CODE", "source_file": "api.py"}),
            json!({"id": format!("{b}_id"), "label": b, "type": "CODE", "source_file": "api.py"}),
        ];
        let edges = vec![json!({
            "source": format!("{a}_id"), "target": format!("{b}_id"),
            "relation": "calls", "confidence": 1.0, "weight": 1.0,
        })];
        let mut communities: IndexMap<String, i64> = IndexMap::new();
        communities.insert(format!("{a}_id"), 0);
        communities.insert(format!("{b}_id"), 0);
        let (out_nodes, _) =
            deduplicate_entities(&nodes, &edges, &communities, None).expect("dedup");
        let labels: std::collections::HashSet<&str> = out_nodes
            .iter()
            .filter_map(|n| n["label"].as_str())
            .collect();
        assert!(
            labels.contains(a) && labels.contains(b),
            "#1201 regression: '{a}' and '{b}' were merged"
        );
    }
}

#[test]
fn pass2_winner_union_does_not_pull_in_uncompared_same_label_nodes() {
    // A ("Session Manager", auth.md) and B ("Session Manager", billing.md) are
    // kept distinct by the cross-file guards. When the A-C fuzzy match fires,
    // the winner must come from the verified pair only — B must not be absorbed.
    let nodes = vec![
        json!({"id": "session_manager_auth", "label": "Session Manager", "source_file": "auth.md"}),
        json!({"id": "sm", "label": "Session Manager", "source_file": "billing.md"}),
        json!({"id": "session_managr_notes", "label": "Session Managr", "source_file": "notes.md"}),
    ];
    let communities: IndexMap<String, i64> = IndexMap::new();
    let (result_nodes, _) = deduplicate_entities(&nodes, &[], &communities, None).expect("dedup");
    let ids: std::collections::HashSet<&str> = result_nodes
        .iter()
        .filter_map(|n| n["id"].as_str())
        .collect();
    assert!(
        ids.contains("sm"),
        "uncompared cross-file node 'sm' was absorbed via pass-2 winner-union"
    );
    assert_eq!(result_nodes.len(), 2);
}

#[test]
fn prefix_guard_does_not_block_same_length_typos() {
    // Same-length pairs (graphextractor / graphextractar) are not strict prefix
    // extensions, so the guard must not fire.
    let a = norm("GraphExtractor");
    let b = norm("GraphExtractar");
    let (lo, hi) = if a.len() <= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    assert!(
        !(hi.starts_with(lo.as_str()) && hi != lo),
        "prefix guard fires on same-length pair ({a:?}, {b:?})"
    );
}

#[test]
fn prefix_guard_fires_for_extension_pairs() {
    for (a_raw, b_raw) in [
        ("getActiveSession", "getActiveSessions"),
        ("parseConfig", "parseConfigFile"),
        ("load", "loadAll"),
    ] {
        let a = norm(a_raw);
        let b = norm(b_raw);
        let (lo, hi) = if a.len() <= b.len() {
            (&a, &b)
        } else {
            (&b, &a)
        };
        assert!(
            hi.starts_with(lo.as_str()) && hi != lo,
            "prefix guard should fire for ({a:?}, {b:?})"
        );
    }
}
