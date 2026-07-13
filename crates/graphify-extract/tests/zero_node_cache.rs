//! #1666 — a zero-node extraction result must not be cached (a rerun self-heals),
//! but a normal (non-empty) result must still cache.
//!
//! Ports the portable case of `graphify-py/tests/test_zero_node_no_cache.py`. The
//! zero-node path itself is only reachable in Python via monkeypatching the
//! extractor (every real extractor emits at least a file node), and the warning
//! is stderr-only — neither is capturable in an in-process Rust test — so this
//! covers the guard-against-over-correction contract: a normal result caches.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use tempfile::tempdir;

#[test]
fn normal_file_still_cached() {
    let tmp = tempdir().expect("tempdir");
    let f = tmp.path().join("ok.rb");
    std::fs::write(&f, "class Bar\n  def baz; end\nend\n").expect("write");
    let out = tmp.path().join("out");

    let result = graphify_extract::extract(std::slice::from_ref(&f), Some(&out));
    assert!(!result.nodes.is_empty(), "a normal file must produce nodes");

    // The non-empty result must have been written to the AST cache.
    assert!(
        graphify_cache::load_cached(&f, &out, "ast").is_some(),
        "a non-empty result should be cached"
    );
}
