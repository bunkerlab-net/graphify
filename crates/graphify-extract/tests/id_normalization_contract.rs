//! Drift guard for the node-ID normalization contract (#811 / #1378).
//!
//! Ports the deterministic cases of `graphify-py/tests/test_id_normalization_contract.py`.
//! Three independent producers must agree on node IDs or the graph splits a
//! single entity into disconnected ghost nodes: the AST extractor
//! (`graphify_extract::make_id`), the LLM subagents (skill spec), and the graph
//! builder (`graphify_build::normalize_id`). Since `make_id` now delegates to
//! `normalize_id`, the single-part form must equal `normalize_id` char-for-char.
//!
//! The Python file's hypothesis property tests are Python-only and intentionally
//! skipped. `test_extraction_spec_ids.py::test_spec_node_id_examples_match_ast_extractor`
//! globs `extraction-spec.md` files that live only in the Python submodule
//! (`graphify-py/`), not in this crate's tree, so its intent is covered here by
//! the joined-spec example and the cautionary wrong-form assertions instead.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use graphify_build::normalize_id;
use graphify_extract::{make_id, make_id1};

/// Inputs that previously diverged or are easy to get wrong. The single-part
/// form of `make_id` must equal `normalize_id` for every one of these.
///
/// The two `café` entries are the composed (`U+00E9`) and decomposed
/// (`e` + `U+0301`) forms — NFKC must collapse them to the same ID (#811).
const CONTRACT_CASES: &[&str] = &[
    "Session_ValidateToken",  // casing
    "session.validate-token", // punctuation -> underscore
    "foo__bar..baz",          // repeated separators collapse
    "  Leading_Trailing__  ", // strip stray underscores/space
    "A/B\\C",                 // path separators both directions
    "MixedCASE",              // casefold
    "caf\u{e9}",              // composed accented Latin (NFKC)
    "cafe\u{301}",            // decomposed e + combining acute -> same as 'café'
    "日本語クラス",           // CJK letters survive, not collapsed
    "Кириллица",              // Cyrillic survives
    "na\u{ef}ve_\u{dc}ber",   // mixed accented Latin (naïve_Über)
    "x_c1",                   // must NOT be treated as a chunk suffix here
    "__dunder__",             // leading/trailing underscores stripped
    "tab\tnewline\nspace ",   // whitespace runs -> single underscore
];

/// The AST id-maker and the builder's reconciler must agree, char for char.
#[test]
fn make_id_matches_normalize_id() {
    for &raw in CONTRACT_CASES {
        assert_eq!(
            make_id1(raw),
            normalize_id(raw),
            "ID drift for {raw:?}: make_id -> {:?} but normalize_id -> {:?}",
            make_id1(raw),
            normalize_id(raw),
        );
    }
}

/// `normalize_id` is idempotent: re-normalising its own output is a no-op.
#[test]
fn normalize_id_is_idempotent() {
    for &raw in CONTRACT_CASES {
        let once = normalize_id(raw);
        assert_eq!(
            normalize_id(&once),
            once,
            "normalize_id not idempotent for {raw:?}"
        );
    }
}

/// Multi-part `make_id` == `normalize_id` of the joined parts (the builder only
/// ever sees the joined string, so these must coincide).
#[test]
fn make_id_joins_then_normalizes() {
    let parts = ["auth", "session.py", "ValidateToken"];
    assert_eq!(
        make_id(&parts),
        normalize_id("auth_session.py_ValidateToken")
    );
    // Documented spec example: src/auth/session.py + ValidateToken.
    assert_eq!(
        make_id(&["auth", "session", "ValidateToken"]),
        "auth_session_validatetoken"
    );
}

/// #811: non-ASCII identifiers must yield distinct, non-empty IDs rather than
/// collapsing to a single per-file node.
#[test]
fn unicode_identifiers_do_not_collapse_to_empty() {
    let a = make_id1("クラスА");
    let b = make_id1("クラスB");
    assert!(!a.is_empty() && !b.is_empty() && a != b);
}

/// Composed and decomposed Unicode forms collapse to the same ID via NFKC.
#[test]
fn composed_and_decomposed_forms_unify() {
    assert_eq!(make_id1("caf\u{e9}"), make_id1("cafe\u{301}"));
}

/// Output is lowercase and contains no path/punctuation separators.
#[test]
fn normalized_ids_are_safe_node_ids() {
    for &raw in CONTRACT_CASES {
        let out = normalize_id(raw);
        assert_eq!(out, out.to_lowercase());
        assert!(
            !out.chars()
                .any(|c| matches!(c, '.' | '/' | '\\') || c.is_whitespace()),
            "unsafe char in id {out:?}"
        );
        assert!(!out.starts_with('_') && !out.ends_with('_'));
    }
}

/// Guard against re-forking: `make_id` must round-trip through the shared
/// `normalize_id` core, including for multi-part Unicode input.
#[test]
fn both_callers_share_one_implementation() {
    assert_eq!(make_id1("Foo.Bar"), normalize_id("Foo.Bar"));
    assert_eq!(make_id(&["Foo.Bar", "baz"]), normalize_id("Foo.Bar_baz"));
    assert_eq!(make_id(&["Ångström", "Ⅳ"]), normalize_id("Ångström_Ⅳ"));
}

/// The canonical spec warns against the filename-only and full-path ID forms.
/// Lock those anti-examples to the code (mirrors `test_extraction_spec_ids.py`
/// `test_cautionary_wrong_forms_are_actually_wrong`).
#[test]
fn cautionary_wrong_forms_are_actually_wrong() {
    let correct = make_id(&["auth", "session", "ValidateToken"]);
    assert_eq!(correct, "auth_session_validatetoken");
    // filename-only (drops the parent dir) and full-path (keeps every segment).
    assert_ne!(make_id(&["session", "ValidateToken"]), correct);
    assert_ne!(
        make_id(&["src", "auth", "session", "ValidateToken"]),
        correct
    );
}
