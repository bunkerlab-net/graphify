//! Parity tests for LLM-backed community labeling (#1097).
//!
//! Mirrors `graphify-py/tests/test_labeling.py`. The Python tests monkeypatch
//! `_call_llm`; the Rust port injects the call via the `*_with` variants.
#![allow(clippy::expect_used, clippy::float_cmp, unsafe_code)]

use std::cell::Cell;

use graphify_llm::{
    generate_community_labels, generate_community_labels_with, label_communities_with,
};
use indexmap::{IndexMap, IndexSet};

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
fn generate_community_labels_no_backend() {
    // With backend=None and no env keys / no providers.json, detect_backend()
    // returns None and the real wrapper degrades to placeholders without any LLM
    // call. nextest runs each test in its own process, so env edits are isolated.
    let home = tempfile::tempdir().expect("tempdir");
    let mut g = EnvGuard::new();
    g.set("HOME", &home.path().to_string_lossy());
    for key in [
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "MOONSHOT_API_KEY",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "DEEPSEEK_API_KEY",
        "OLLAMA_BASE_URL",
        "AWS_PROFILE",
        "AWS_REGION",
        "AWS_DEFAULT_REGION",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_WEB_IDENTITY_TOKEN_FILE",
        "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
    ] {
        g.unset(key);
    }

    let (node_labels, communities) = graph();
    let gods = IndexSet::new();
    let (labels, source) = generate_community_labels(&communities, &node_labels, &gods, None, true);
    assert_eq!(source, "placeholder");
    assert_eq!(labels[&0], "Community 0");
    assert_eq!(labels[&1], "Community 1");
}

/// RAII guard that sets/restores env vars.
struct EnvGuard {
    saved: Vec<(String, Option<String>)>,
}
impl EnvGuard {
    fn new() -> Self {
        Self { saved: vec![] }
    }
    fn set(&mut self, k: &str, v: &str) -> &mut Self {
        self.saved.push((k.to_string(), std::env::var(k).ok()));
        unsafe { std::env::set_var(k, v) };
        self
    }
    fn unset(&mut self, k: &str) -> &mut Self {
        self.saved.push((k.to_string(), std::env::var(k).ok()));
        unsafe { std::env::remove_var(k) };
        self
    }
}
impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, prev) in self.saved.drain(..).rev() {
            match prev {
                Some(v) => unsafe { std::env::set_var(&k, &v) },
                None => unsafe { std::env::remove_var(&k) },
            }
        }
    }
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
