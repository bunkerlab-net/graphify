//! Parity tests for LLM-backed community labeling (#1097).
//!
//! Mirrors `graphify-py/tests/test_labeling.py`. The Python tests monkeypatch
//! `_call_llm`; the Rust port injects the call via the `*_with` variants.
#![allow(clippy::expect_used)]

use std::cell::Cell;

use graphify_llm::{
    LabelOptions, generate_community_labels, generate_community_labels_with, label_communities,
    label_communities_with,
};
use indexmap::{IndexMap, IndexSet};
use serial_test::serial;

mod common;
use common::EnvGuard;

/// community 0 = ordering, community 1 = payments.
fn graph() -> (IndexMap<String, String>, IndexMap<i64, Vec<String>>) {
    let mut node_labels = IndexMap::new();
    node_labels.insert("order_place".to_string(), "place_order".to_string());
    node_labels.insert("order_repo".to_string(), "OrderRepository".to_string());
    node_labels.insert("pay_charge".to_string(), "charge_card".to_string());
    node_labels.insert("pay_stripe".to_string(), "StripeClient".to_string());
    let mut communities = IndexMap::new();
    communities.insert(0, vec!["order_place".to_string(), "order_repo".to_string()]);
    communities.insert(1, vec!["pay_charge".to_string(), "pay_stripe".to_string()]);
    (node_labels, communities)
}

/// `n_communities` two-node communities, ids `0..n`, all equal size so the
/// ordering inside `label_communities` is stable insertion order. Mirrors
/// Python's `_wide_graph`.
fn wide_graph(n: i64) -> (IndexMap<String, String>, IndexMap<i64, Vec<String>>) {
    let mut node_labels = IndexMap::new();
    let mut communities = IndexMap::new();
    for cid in 0..n {
        let a = format!("c{cid}_a");
        let b = format!("c{cid}_b");
        node_labels.insert(a.clone(), format!("node_{cid}_a"));
        node_labels.insert(b.clone(), format!("node_{cid}_b"));
        communities.insert(cid, vec![a, b]);
    }
    (node_labels, communities)
}

/// Parse the community ids a prompt batch asks about (lines `Community <id>: …`),
/// mirroring the Python fakes' prompt-scraping.
fn cids_in_prompt(prompt: &str) -> Vec<i64> {
    prompt
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("Community ")?;
            let id = rest.split_once(':')?.0.trim();
            id.parse::<i64>().ok()
        })
        .collect()
}

#[test]
fn label_communities_happy_path() {
    let (node_labels, communities) = graph();
    let gods = IndexSet::new();
    let captured: Cell<Option<(String, String)>> = Cell::new(None);

    let labels = label_communities_with(
        &communities,
        &node_labels,
        &gods,
        "gemini",
        LabelOptions::default(),
        |prompt, backend, _max, _model| {
            captured.set(Some((prompt.to_string(), backend.to_string())));
            Ok(r#"{"0": "Order Management", "1": "Payment Flow"}"#.to_string())
        },
    )
    .expect("labeling succeeds");

    assert_eq!(labels[&0], "Order Management");
    assert_eq!(labels[&1], "Payment Flow");
    let (prompt, backend) = captured.take().expect("call invoked");
    assert!(prompt.contains("place_order"));
    assert!(prompt.contains("StripeClient"));
    assert_eq!(backend, "gemini");
}

#[test]
fn label_communities_passes_model_override() {
    // The model override threads through to the injected call (#b304331).
    let (node_labels, communities) = graph();
    let gods = IndexSet::new();
    let captured: Cell<Option<(String, Option<String>)>> = Cell::new(None);

    let opts = LabelOptions {
        model: Some("gemini-3.1-flash-lite"),
        ..LabelOptions::default()
    };
    let labels = label_communities_with(
        &communities,
        &node_labels,
        &gods,
        "gemini",
        opts,
        |_prompt, backend, _max, model| {
            captured.set(Some((backend.to_string(), model.map(str::to_string))));
            Ok(r#"{"0": "Order Management", "1": "Payment Flow"}"#.to_string())
        },
    )
    .expect("labeling succeeds");

    assert_eq!(labels[&0], "Order Management");
    assert_eq!(labels[&1], "Payment Flow");
    let (backend, model) = captured.take().expect("call invoked");
    assert_eq!(backend, "gemini");
    assert_eq!(model.as_deref(), Some("gemini-3.1-flash-lite"));
}

#[test]
fn label_communities_partial_reply_fills_placeholder() {
    let (node_labels, communities) = graph();
    let gods = IndexSet::new();
    let labels = label_communities_with(
        &communities,
        &node_labels,
        &gods,
        "gemini",
        LabelOptions::default(),
        |_, _, _, _| Ok(r#"{"0": "Order Management"}"#.to_string()),
    )
    .expect("labeling succeeds");
    assert_eq!(labels[&0], "Order Management");
    assert_eq!(labels[&1], "Community 1"); // missing cid falls back
}

#[test]
fn label_communities_strips_code_fences() {
    let (node_labels, communities) = graph();
    let gods = IndexSet::new();
    let labels = label_communities_with(
        &communities,
        &node_labels,
        &gods,
        "gemini",
        LabelOptions::default(),
        |_, _, _, _| Ok("```json\n{\"0\":\"Orders\",\"1\":\"Pay\"}\n```".to_string()),
    )
    .expect("labeling succeeds");
    assert_eq!(labels[&0], "Orders");
    assert_eq!(labels[&1], "Pay");
}

#[test]
fn label_communities_extracts_json_from_surrounding_prose() {
    // A reply that wraps the JSON in prose is salvaged by slicing the first `{`
    // to the last `}`.
    let (node_labels, communities) = graph();
    let gods = IndexSet::new();
    let labels = label_communities_with(
        &communities,
        &node_labels,
        &gods,
        "gemini",
        LabelOptions::default(),
        |_, _, _, _| {
            Ok("Here are the names: {\"0\":\"Orders\",\"1\":\"Pay\"} hope that helps".to_string())
        },
    )
    .expect("labeling succeeds");
    assert_eq!(labels[&0], "Orders");
    assert_eq!(labels[&1], "Pay");
}

#[test]
fn label_communities_strips_trailing_prose_after_json() {
    // A reply that leads with valid JSON but appends prose
    // (`{"0":"x"} hope that helps`) must still parse: the sanitizer slices the
    // first `{` … last `}` span even when the text already starts with `{`.
    let (node_labels, communities) = graph();
    let gods = IndexSet::new();
    let labels = label_communities_with(
        &communities,
        &node_labels,
        &gods,
        "gemini",
        LabelOptions::default(),
        |_, _, _, _| Ok(r#"{"0":"Orders","1":"Pay"} hope that helps"#.to_string()),
    )
    .expect("labeling succeeds");
    assert_eq!(labels[&0], "Orders");
    assert_eq!(labels[&1], "Pay");
}

#[test]
fn label_communities_malformed_errors() {
    let (node_labels, communities) = graph();
    let gods = IndexSet::new();
    let result = label_communities_with(
        &communities,
        &node_labels,
        &gods,
        "gemini",
        LabelOptions::default(),
        |_, _, _, _| Ok("sorry, I cannot help".to_string()),
    );
    assert!(result.is_err());
}

#[test]
fn generate_community_labels_degrades_on_error() {
    let (node_labels, communities) = graph();
    let gods = IndexSet::new();
    let (labels, source) = generate_community_labels_with(
        &communities,
        &node_labels,
        &gods,
        Some("gemini"),
        None,
        true,
        |_, _, _, _| Ok("not json".to_string()),
    );
    assert_eq!(source, "placeholder");
    assert_eq!(labels[&0], "Community 0");
    assert_eq!(labels[&1], "Community 1");
}

#[test]
#[serial]
fn generate_community_labels_no_backend() {
    // With backend=None and no env keys / no providers.json, detect_backend()
    // returns None and the real wrapper degrades to placeholders without any LLM
    // call. nextest runs each test in its own process, so env edits are isolated.
    let home = tempfile::tempdir().expect("tempdir");
    let mut g = EnvGuard::new();
    g.set("HOME", &home.path().to_string_lossy());
    g.scrub_backends();

    let (node_labels, communities) = graph();
    let gods = IndexSet::new();
    let (labels, source) =
        generate_community_labels(&communities, &node_labels, &gods, None, None, true);
    assert_eq!(source, "placeholder");
    assert_eq!(labels[&0], "Community 0");
    assert_eq!(labels[&1], "Community 1");
}

#[test]
fn generate_community_labels_degrades_loud() {
    // quiet=false exercises the warning branch; result is still placeholders.
    let (node_labels, communities) = graph();
    let gods = IndexSet::new();
    let (labels, source) = generate_community_labels_with(
        &communities,
        &node_labels,
        &gods,
        Some("gemini"),
        None,
        false,
        |_, _, _, _| Ok("not json".to_string()),
    );
    assert_eq!(source, "placeholder");
    assert_eq!(labels[&0], "Community 0");
}

#[test]
#[serial]
fn label_communities_real_path_via_custom_provider() {
    // Exercise the public (non-`_with`) wrappers end-to-end: a custom provider
    // pointed at a mock server drives `call_llm`, and both `label_communities`
    // and `generate_community_labels` parse the reply.
    let mut server = mockito::Server::new();
    let mock = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(
            serde_json::json!({
                "choices": [{
                    "message": {"content": "{\"0\":\"Orders\",\"1\":\"Payments\"}"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 3, "completion_tokens": 4}
            })
            .to_string(),
        )
        .expect_at_least(1)
        .create();

    let home = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(home.path().join(".graphify")).expect("mkdir");
    std::fs::write(
        home.path().join(".graphify").join("providers.json"),
        format!(
            r#"{{"labelprov": {{"base_url": "{}", "default_model": "m", "env_key": "LABELPROV_KEY"}}}}"#,
            server.url()
        ),
    )
    .expect("write providers.json");

    let mut g = EnvGuard::new();
    // Hermetic setup: clear built-in backend keys so this stays a pure
    // custom-provider labeling test. (`LABELPROV_KEY` is a registry env_key,
    // untouched by scrub.)
    g.scrub_backends();
    g.set("HOME", &home.path().to_string_lossy());
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("LABELPROV_KEY", "secret");

    let (node_labels, communities) = graph();
    let gods = IndexSet::new();

    let labels = label_communities(
        &communities,
        &node_labels,
        &gods,
        "labelprov",
        LabelOptions::default(),
    )
    .expect("labels");
    assert_eq!(labels[&0], "Orders");
    assert_eq!(labels[&1], "Payments");

    let (labels, source) = generate_community_labels(
        &communities,
        &node_labels,
        &gods,
        Some("labelprov"),
        None,
        true,
    );
    assert_eq!(source, "llm");
    assert_eq!(labels[&0], "Orders");

    // Enforce that the mock endpoint was actually hit, rather than relying on
    // the label assertions alone.
    mock.assert();
}

#[test]
fn generate_community_labels_success() {
    let (node_labels, communities) = graph();
    let gods = IndexSet::new();
    let (labels, source) = generate_community_labels_with(
        &communities,
        &node_labels,
        &gods,
        Some("gemini"),
        None,
        true,
        |_, _, _, _| Ok(r#"{"0":"Orders","1":"Payments"}"#.to_string()),
    );
    assert_eq!(source, "llm");
    assert_eq!(labels[&0], "Orders");
    assert_eq!(labels[&1], "Payments");
}

#[test]
fn gods_as_ids_do_not_crash() {
    // god_nodes() ids are pre-resolved to a set at the CLI boundary; passing
    // them must not change the labels for these small communities.
    let (node_labels, communities) = graph();
    let mut gods = IndexSet::new();
    gods.insert("order_repo".to_string());
    let labels = label_communities_with(
        &communities,
        &node_labels,
        &gods,
        "gemini",
        LabelOptions::default(),
        |_, _, _, _| Ok(r#"{"0":"Orders","1":"Pay"}"#.to_string()),
    )
    .expect("labeling succeeds");
    assert_eq!(labels[&0], "Orders");
    assert_eq!(labels[&1], "Pay");
}

#[test]
fn empty_communities_returns_placeholders() {
    let node_labels = IndexMap::new();
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec![]);
    let gods = IndexSet::new();
    let called = Cell::new(false);
    // community with no resolvable nodes -> no prompt line -> no backend call.
    let labels = label_communities_with(
        &communities,
        &node_labels,
        &gods,
        "gemini",
        LabelOptions::default(),
        |_, _, _, _| {
            called.set(true);
            Ok("{}".to_string())
        },
    )
    .expect("labeling succeeds");
    assert_eq!(labels[&0], "Community 0");
    assert!(!called.get());
}

// ---------------------------------------------------------------------------
// Multi-batch labeling (#7477b46): a single prompt with >100 communities
// overflows the 16k context window of self-hosted reasoning models.
// label_communities now splits into batches so coverage stays complete.
// ---------------------------------------------------------------------------

#[test]
fn label_communities_batches_when_over_batch_size() {
    let (node_labels, communities) = wide_graph(250);
    let gods = IndexSet::new();
    let calls: std::cell::RefCell<Vec<usize>> = std::cell::RefCell::new(Vec::new());

    let opts = LabelOptions {
        batch_size: 100,
        ..LabelOptions::default()
    };
    let labels = label_communities_with(
        &communities,
        &node_labels,
        &gods,
        "gemini",
        opts,
        |prompt, _backend, _max, _model| {
            let cids = cids_in_prompt(prompt);
            calls.borrow_mut().push(cids.len());
            let body = cids
                .iter()
                .map(|c| format!("\"{c}\": \"Cluster {c}\""))
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!("{{{body}}}"))
        },
    )
    .expect("labeling succeeds");

    // 250 communities / 100 per batch -> 3 batches (100, 100, 50).
    assert_eq!(*calls.borrow(), vec![100, 100, 50]);
    // Every community got a real name, none left as a placeholder.
    assert_eq!(labels.len(), 250);
    assert!(
        labels.values().all(|name| name.starts_with("Cluster ")),
        "some communities still have placeholders"
    );
}

#[test]
fn label_communities_partial_batch_failure_keeps_successful_batches() {
    let (node_labels, communities) = wide_graph(150);
    let gods = IndexSet::new();
    let n_calls = Cell::new(0u32);

    let opts = LabelOptions {
        batch_size: 50,
        ..LabelOptions::default()
    };
    let labels = label_communities_with(
        &communities,
        &node_labels,
        &gods,
        "gemini",
        opts,
        |prompt, _backend, _max, _model| {
            n_calls.set(n_calls.get() + 1);
            let cids = cids_in_prompt(prompt);
            if n_calls.get() == 2 {
                return Err(graphify_llm::LlmError::Http(
                    "simulated transient backend failure".to_string(),
                ));
            }
            let body = cids
                .iter()
                .map(|c| format!("\"{c}\": \"Named {c}\""))
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!("{{{body}}}"))
        },
    )
    .expect("labeling succeeds despite one batch failing");

    // 3 batches; the second fails. First and third produce real labels; the
    // failed batch's cids stay as placeholders.
    let real = labels
        .values()
        .filter(|name| name.starts_with("Named "))
        .count();
    let placeholder = labels
        .values()
        .filter(|name| name.starts_with("Community "))
        .count();
    assert_eq!(
        real, 100,
        "expected 100 real labels from 2 successful batches"
    );
    assert_eq!(
        placeholder, 50,
        "expected 50 placeholders from the failed batch"
    );
}

#[test]
fn label_communities_all_batches_fail_raises() {
    let (node_labels, communities) = wide_graph(150);
    let gods = IndexSet::new();
    let opts = LabelOptions {
        batch_size: 50,
        ..LabelOptions::default()
    };
    // Every batch fails -> propagate so generate_community_labels can degrade.
    let result = label_communities_with(
        &communities,
        &node_labels,
        &gods,
        "gemini",
        opts,
        |_, _, _, _| Err(graphify_llm::LlmError::Http("backend down".to_string())),
    );
    let err = result.expect_err("all batches failed");
    assert!(err.to_string().contains("backend down"));
}

#[test]
fn label_communities_max_communities_caps_total() {
    // Backwards compat: explicit max_communities still caps the total labeled.
    let (node_labels, communities) = wide_graph(150);
    let gods = IndexSet::new();
    let captured: std::cell::RefCell<Vec<i64>> = std::cell::RefCell::new(Vec::new());

    let opts = LabelOptions {
        max_communities: Some(40),
        batch_size: 100,
        ..LabelOptions::default()
    };
    label_communities_with(
        &communities,
        &node_labels,
        &gods,
        "gemini",
        opts,
        |prompt, _backend, _max, _model| {
            let cids = cids_in_prompt(prompt);
            captured.borrow_mut().extend(&cids);
            let body = cids
                .iter()
                .map(|c| format!("\"{c}\": \"X{c}\""))
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!("{{{body}}}"))
        },
    )
    .expect("labeling succeeds");

    // Only 40 communities should have been sent to the backend.
    assert_eq!(captured.borrow().len(), 40);
}

// ---------------------------------------------------------------------------
// Adaptive split-and-retry on parse failure (#1278): a full batch that returns
// malformed JSON is bisected and each half retried, so no community is dropped.
// ---------------------------------------------------------------------------

#[test]
fn label_batch_recovers_via_split_on_invalid_json() {
    // One batch of 4 communities; the first (full-batch) call returns broken
    // JSON, forcing a 2+2 split. Both halves return valid JSON, so every
    // community ends up labeled — none silently dropped.
    let (node_labels, communities) = wide_graph(4);
    let gods = IndexSet::new();
    let n_calls = Cell::new(0u32);
    let labels = label_communities_with(
        &communities,
        &node_labels,
        &gods,
        "gemini",
        LabelOptions::default(),
        |prompt, _backend, _max, _model| {
            n_calls.set(n_calls.get() + 1);
            if n_calls.get() == 1 {
                // Broken JSON on the full batch triggers the split-and-retry.
                return Ok("{this is not valid json, missing quotes".to_string());
            }
            // Each retried half returns a clean object for its own cids.
            let body = cids_in_prompt(prompt)
                .iter()
                .map(|c| format!("\"{c}\": \"Label {c}\""))
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!("{{{body}}}"))
        },
    )
    .expect("labeling recovers via split");

    for cid in 0i64..4 {
        assert_eq!(labels[&cid], format!("Label {cid}"));
    }
    assert!(n_calls.get() >= 2, "expected a split into at least 2 calls");
}
