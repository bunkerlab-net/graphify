//! Parity tests for `graphify-dedup`.
//!
//! Ports every test in `graphify-py/tests/test_dedup.py`.

#![allow(clippy::expect_used, clippy::float_cmp)] // test file

use graphify_dedup::{
    DedupLlmBackend, JudgeResult, NoOpBackend, deduplicate_entities, defines_id, entropy,
    is_variant_pair, norm, numeric_tokens_differ, shingles, short_label_blocked,
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

// ── #1284: numbered siblings + cross-file file-anchored boilerplate ──────────

#[test]
fn test_numeric_tokens_differ_helper() {
    // `numeric_tokens_differ` compares digit runs as zero-padding-insensitive
    // multisets (#1284).
    assert!(numeric_tokens_differ(
        "adr 0011 d5 pipeline placement",
        "adr 0013 d4 pipeline placement"
    ));
    assert!(numeric_tokens_differ(
        "3 1 product goals",
        "1 1 product goals"
    ));
    assert!(numeric_tokens_differ("code block3", "code block13"));
    // zero-padding is not a difference
    assert!(!numeric_tokens_differ(
        "phase 09 overview",
        "phase 9 overview"
    ));
    assert!(!numeric_tokens_differ(
        "module layout wave 3",
        "module layouts wave 3"
    ));
    // digitless labels never differ on numbers
    assert!(!numeric_tokens_differ("graph extractor", "graph extractar"));
}

#[test]
fn test_dedup_does_not_merge_numbered_siblings() {
    // Long labels differing only in embedded numbers (ADR/section/issue ids)
    // must not merge — numbered siblings, not duplicates (#1284).
    let nodes = vec![
        json!({
            "id": "n1",
            "label": "Pipeline placement — 4 call sites (ADR 0013 D4)",
            "file_type": "document",
            "source_file": "docs/index-activity.md"
        }),
        json!({
            "id": "n2",
            "label": "Pipeline placement — 4 call sites (ADR 0011 §D5)",
            "file_type": "document",
            "source_file": "docs/schema-matcher.md"
        }),
    ];
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 2);
}

#[test]
fn test_dedup_does_not_merge_crossfile_rationale_boilerplate() {
    // Rationale nodes are file-anchored like code (#1205): parallel modules'
    // boilerplate docstrings differing by one word must not merge (#1284).
    let boiler = |name: &str| {
        format!(
            "Django app config for {name}. No business logic here. \
             Domain services live in services.py and adapters in providers."
        )
    };
    let nodes = vec![
        json!({
            "id": "r1",
            "label": boiler("apps.platform.cards"),
            "file_type": "rationale",
            "source_file": "apps/platform/cards/apps.py"
        }),
        json!({
            "id": "r2",
            "label": boiler("apps.platform.cores"),
            "file_type": "rationale",
            "source_file": "apps/platform/cores/apps.py"
        }),
    ];
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 2);
}

#[test]
fn test_dedup_does_not_merge_crossfile_document_headings() {
    // Document nodes are file-anchored too: near-identical headings in different
    // files are distinct sections, not duplicates (#1284).
    let nodes = vec![
        json!({
            "id": "d1",
            "label": "Getting Started Installation Guide",
            "file_type": "document",
            "source_file": "docs/a.md"
        }),
        json!({
            "id": "d2",
            "label": "Getting Started Installation Setup",
            "file_type": "document",
            "source_file": "docs/b.md"
        }),
    ];
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 2);
}

#[test]
fn test_dedup_still_merges_samefile_rationale_duplicates() {
    // The file-anchored guard only blocks cross-file pairs — near-identical
    // rationale duplicates within one file still merge (#1284 non-regression).
    let nodes = vec![
        json!({
            "id": "r1",
            "label": "Counts-only metrics export, a read-only aggregation service.",
            "file_type": "rationale",
            "source_file": "apps/schemas/metrics.py"
        }),
        json!({
            "id": "r2",
            "label": "Counts-only metrics export, the read-only aggregation service.",
            "file_type": "rationale",
            "source_file": "apps/schemas/metrics.py"
        }),
    ];
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 1);
}

// ── #1243: JaroWinkler prefix-bonus over-merge (cross-file) ──────────────────

#[test]
fn test_dedup_does_not_merge_crossfile_shared_prefix_divergence() {
    // Cross-file labels sharing a long prefix but diverging in a distinguishing
    // token ("…jest native" vs "…react native") get JaroWinkler's prefix bonus
    // past threshold but are distinct entities; scoring them on plain Jaro
    // blocks the merge (#1243).
    let nodes = vec![
        json!({
            "id": "p1",
            "label": "testing library jest native",
            "file_type": "concept",
            "source_file": "pkg-a/package.json"
        }),
        json!({
            "id": "p2",
            "label": "testing library react native",
            "file_type": "concept",
            "source_file": "pkg-b/package.json"
        }),
    ];
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 2);
}

#[test]
fn test_dedup_still_merges_crossfile_true_duplicates() {
    // The #1243 guard only drops the prefix bonus — a genuine cross-file
    // duplicate (high similarity on Jaro alone) must still merge.
    let nodes = vec![
        json!({"id": "g1", "label": "GraphExtractor", "file_type": "concept", "source_file": "a.md"}),
        json!({"id": "g2", "label": "Graph Extractor", "file_type": "concept", "source_file": "b.md"}),
    ];
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 1);
}

// ── #1504/#1851: cross-chunk node ID collision (definer-wins) ─────────────────
// The survivor of an ID collision is chosen by `collision_rank` (a total order),
// independent of arrival order: a node whose source_file defines the ID beats a
// mere cross-reference; among equally-defining nodes the shorter, more canonical
// label wins, then a lexical tiebreak on label then source_file. The stderr
// warning/note is emitted by `deduplicate_entities`; its exact text is not
// asserted here (in-process stderr capture is impractical for a library call).

#[test]
fn test_cross_chunk_id_collision_keeps_lexically_first_definer() {
    // Both READMEs define the id via the bare `readme` stem, so neither
    // out-defines the other; the lexically-first source_file wins the tiebreak
    // (module-a < module-b) regardless of arrival order.
    let nodes = vec![
        json!({"id": "readme_booking_service", "label": "Booking Service", "file_type": "concept", "source_file": "module-b/README.md"}),
        json!({"id": "readme_booking_service", "label": "Booking Service", "file_type": "concept", "source_file": "module-a/README.md"}),
    ];
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 1);
    assert_eq!(
        result_nodes[0].get("source_file").and_then(Value::as_str),
        Some("module-a/README.md"),
        "the lexically-first source path wins the collision, not the first-seen node"
    );
}

#[test]
fn test_same_id_same_source_file_keeps_shorter_label() {
    // Same file, two labels for one id: the shorter, more canonical label wins
    // by `collision_rank` even though it arrives SECOND (proving the survivor is
    // rank-driven, not first-writer-wins).
    let nodes = vec![
        json!({"id": "readme_booking_service", "label": "Booking Service (dupe)", "file_type": "concept", "source_file": "module-a/README.md"}),
        json!({"id": "readme_booking_service", "label": "Booking Service", "file_type": "concept", "source_file": "module-a/README.md"}),
    ];
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 1);
    assert_eq!(
        result_nodes[0].get("label").and_then(Value::as_str),
        Some("Booking Service")
    );
}

// ── #1504/#1851: definition vs cross-reference ────────────────────────────────

/// The defining node and a doc that merely mentions the entity. Both mint the ID
/// encoded from the *defining* file's path, so they collide by construction.
fn defining_node() -> Value {
    json!({"id": "agents_make_batch_fixtures_make_batch_fixtures",
           "label": "make-batch-fixtures agent", "file_type": "concept",
           "source_file": "agents/make-batch-fixtures.md"})
}
fn referencing_node() -> Value {
    json!({"id": "agents_make_batch_fixtures_make_batch_fixtures",
           "label": "make-batch-fixtures", "file_type": "concept",
           "source_file": "available/diagnose-issue/SKILL.md"})
}

#[test]
fn test_defining_file_wins_definition_first() {
    let nodes = vec![defining_node(), referencing_node()];
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 1);
    assert_eq!(
        result_nodes[0].get("source_file").and_then(Value::as_str),
        Some("agents/make-batch-fixtures.md")
    );
    assert_eq!(
        result_nodes[0].get("label").and_then(Value::as_str),
        Some("make-batch-fixtures agent")
    );
}

#[test]
fn test_defining_file_wins_reference_first() {
    let nodes = vec![referencing_node(), defining_node()];
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 1);
    assert_eq!(
        result_nodes[0].get("source_file").and_then(Value::as_str),
        Some("agents/make-batch-fixtures.md")
    );
    assert_eq!(
        result_nodes[0].get("label").and_then(Value::as_str),
        Some("make-batch-fixtures agent")
    );
}

#[test]
fn test_reference_collision_folds_edges_to_survivor() {
    // A cross-reference collapsing into the entity it references loses nothing —
    // edges are keyed by ID and rewire to the survivor. (The stderr silence is
    // asserted in Python via capsys; here we assert the edge survives.)
    let edges = vec![json!({
        "source": "agents_make_batch_fixtures_make_batch_fixtures",
        "target": "other",
        "relation": "relates_to"
    })];
    let nodes = vec![defining_node(), referencing_node()];
    let (result_nodes, result_edges) =
        deduplicate_entities(&nodes, &edges, &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 1);
    assert_eq!(result_edges.len(), 1);
}

#[test]
fn test_absolute_source_path_still_defines_id() {
    // source_file is absolute in some pipelines and repo-relative in others; the
    // defining file is recognised either way.
    let mut absolute = defining_node();
    absolute["source_file"] = json!("/home/u/proj/agents/make-batch-fixtures.md");
    let nodes = vec![referencing_node(), absolute];
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 1);
    assert_eq!(
        result_nodes[0].get("label").and_then(Value::as_str),
        Some("make-batch-fixtures agent")
    );
}

#[test]
fn test_same_file_relabel_is_deduped() {
    // Two labels for one ID from one file: the loser's label is discarded. Python
    // asserts the stderr `note`; here we assert the dedup outcome.
    let nodes = vec![
        json!({"id": "agents_make_batch_fixtures_make_batch_fixtures",
               "label": "make-batch-fixtures agent", "file_type": "concept",
               "source_file": "agents/make-batch-fixtures.md"}),
        json!({"id": "agents_make_batch_fixtures_make_batch_fixtures",
               "label": "make-batch-fixtures helper agent", "file_type": "concept",
               "source_file": "agents/make-batch-fixtures.md"}),
    ];
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 1);
    assert_eq!(
        result_nodes[0].get("label").and_then(Value::as_str),
        Some("make-batch-fixtures agent"),
        "the shorter, more canonical label survives the relabel"
    );
}

#[test]
fn test_collision_survivor_is_order_independent() {
    // #1851: definer + same-file relabel + cross-file reference. Across every
    // insertion order the SAME node (source_file AND label) must survive.
    let definer = json!({"id": "agents_make_batch_fixtures_make_batch_fixtures",
        "label": "make-batch-fixtures agent", "file_type": "concept",
        "source_file": "agents/make-batch-fixtures.md"});
    let relabel = json!({"id": "agents_make_batch_fixtures_make_batch_fixtures",
        "label": "make-batch-fixtures helper agent", "file_type": "concept",
        "source_file": "agents/make-batch-fixtures.md"});
    let xref = json!({"id": "agents_make_batch_fixtures_make_batch_fixtures",
        "label": "make-batch-fixtures", "file_type": "concept",
        "source_file": "available/diagnose-issue/SKILL.md"});
    let base = [definer, relabel, xref];
    let perms = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let mut survivors: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for perm in perms {
        let nodes: Vec<Value> = perm.iter().map(|&i| base[i].clone()).collect();
        let (out, _) =
            deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
        assert_eq!(out.len(), 1);
        survivors.insert((
            out[0]["source_file"].as_str().unwrap_or("").to_string(),
            out[0]["label"].as_str().unwrap_or("").to_string(),
        ));
    }
    assert_eq!(
        survivors.len(),
        1,
        "non-deterministic collision survivor: {survivors:?}"
    );
    assert!(survivors.contains(&(
        "agents/make-batch-fixtures.md".to_string(),
        "make-batch-fixtures agent".to_string()
    )));
}

#[test]
fn test_bare_file_node_defines_its_own_id() {
    // A file-level semantic node whose id is exactly the slugified path (no
    // `_entity` suffix) must be recognised as defining its id (#1851 tweak).
    assert!(defines_id(&json!({
        "id": "agents_make_batch_fixtures",
        "source_file": "agents/make-batch-fixtures.md"
    })));
}

#[test]
fn test_defines_id_helper() {
    assert!(defines_id(&defining_node()));
    assert!(!defines_id(&referencing_node()));
    // Pre-#1504 IDs keyed off the bare filename stem.
    assert!(defines_id(&json!({
        "id": "readme_booking_service",
        "source_file": "module-a/README.md"
    })));
    // A path that is merely a string-prefix of the ID's path does not define it.
    assert!(!defines_id(
        &json!({"id": "agents_foo", "source_file": "agent/foo.md"})
    ));
    assert!(!defines_id(
        &json!({"id": "docs_intro_foo", "source_file": ""})
    ));
}

// ── #1857: dedup summary breakdown (emitted to stderr in Rust) ─────────────────

#[test]
fn test_dedup_summary_fuzzy_only_run_merges() {
    // Two long, high-entropy, non-code labels on different files: Pass 1 (exact,
    // same-file) finds nothing; Pass 2 (Jaro-Winkler cross-file) merges them. The
    // summary line (stderr) reports only the fuzzy count. We assert the merge.
    let nodes = vec![
        json!({"id": "g1", "label": "GraphExtractor", "file_type": "concept", "source_file": "a.md"}),
        json!({"id": "g2", "label": "Graph Extractor", "file_type": "concept", "source_file": "b.md"}),
    ];
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 1);
}

#[test]
fn test_dedup_summary_exact_only_run_merges() {
    // Same file + same normalized label → Pass 1 exact merge; summary reports
    // only the exact count.
    let nodes = vec![
        json!({"id": "u1", "label": "User Service", "file_type": "concept", "source_file": "svc.md"}),
        json!({"id": "u2", "label": "user service", "file_type": "concept", "source_file": "svc.md"}),
    ];
    let (result_nodes, _) =
        deduplicate_entities(&nodes, &[], &empty_communities(), None).expect("dedup ok");
    assert_eq!(result_nodes.len(), 1);
}
