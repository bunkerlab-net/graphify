//! Parity tests for LLM-backed community labeling (#1097).
//!
//! Mirrors `graphify-py/tests/test_labeling.py`. The Python tests monkeypatch
//! `_call_llm`; the Rust port injects the call via the `*_with` variants.
#![allow(clippy::expect_used)]

use std::cell::Cell;

use graphify_llm::{
    generate_community_labels, generate_community_labels_with, label_communities,
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
        |prompt, backend, _max| {
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
fn label_communities_partial_reply_fills_placeholder() {
    let (node_labels, communities) = graph();
    let gods = IndexSet::new();
    let labels = label_communities_with(&communities, &node_labels, &gods, "gemini", |_, _, _| {
        Ok(r#"{"0": "Order Management"}"#.to_string())
    })
    .expect("labeling succeeds");
    assert_eq!(labels[&0], "Order Management");
    assert_eq!(labels[&1], "Community 1"); // missing cid falls back
}

#[test]
fn label_communities_strips_code_fences() {
    let (node_labels, communities) = graph();
    let gods = IndexSet::new();
    let labels = label_communities_with(&communities, &node_labels, &gods, "gemini", |_, _, _| {
        Ok("```json\n{\"0\":\"Orders\",\"1\":\"Pay\"}\n```".to_string())
    })
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
    let labels = label_communities_with(&communities, &node_labels, &gods, "gemini", |_, _, _| {
        Ok("Here are the names: {\"0\":\"Orders\",\"1\":\"Pay\"} hope that helps".to_string())
    })
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
    let labels = label_communities_with(&communities, &node_labels, &gods, "gemini", |_, _, _| {
        Ok(r#"{"0":"Orders","1":"Pay"} hope that helps"#.to_string())
    })
    .expect("labeling succeeds");
    assert_eq!(labels[&0], "Orders");
    assert_eq!(labels[&1], "Pay");
}

#[test]
fn label_communities_malformed_errors() {
    let (node_labels, communities) = graph();
    let gods = IndexSet::new();
    let result = label_communities_with(&communities, &node_labels, &gods, "gemini", |_, _, _| {
        Ok("sorry, I cannot help".to_string())
    });
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
        true,
        |_, _, _| Ok("not json".to_string()),
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
    let (labels, source) = generate_community_labels(&communities, &node_labels, &gods, None, true);
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
        false,
        |_, _, _| Ok("not json".to_string()),
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

    let labels = label_communities(&communities, &node_labels, &gods, "labelprov").expect("labels");
    assert_eq!(labels[&0], "Orders");
    assert_eq!(labels[&1], "Payments");

    let (labels, source) =
        generate_community_labels(&communities, &node_labels, &gods, Some("labelprov"), true);
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
        true,
        |_, _, _| Ok(r#"{"0":"Orders","1":"Payments"}"#.to_string()),
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
    let labels = label_communities_with(&communities, &node_labels, &gods, "gemini", |_, _, _| {
        Ok(r#"{"0":"Orders","1":"Pay"}"#.to_string())
    })
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
    let labels = label_communities_with(&communities, &node_labels, &gods, "gemini", |_, _, _| {
        called.set(true);
        Ok("{}".to_string())
    })
    .expect("labeling succeeds");
    assert_eq!(labels[&0], "Community 0");
    assert!(!called.get());
}
