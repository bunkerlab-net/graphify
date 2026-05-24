//! Coverage tests for small public modules: `read_files` and `LlmResponse::to_value`.

#![allow(clippy::expect_used)]

use std::fs;
use std::path::PathBuf;

use graphify_llm::{LlmResponse, read_files};
use serde_json::json;

#[test]
fn read_files_formats_with_relative_paths() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let a = root.join("a.py");
    let b = root.join("sub").join("b.py");
    fs::create_dir_all(b.parent().expect("create_dir_all")).expect("test invariant");
    fs::write(&a, "print('a')").expect("test invariant");
    fs::write(&b, "print('b')").expect("test invariant");

    let out = read_files(&[a.clone(), b.clone()], root);
    assert!(out.contains("=== a.py ==="));
    assert!(out.contains("=== sub/b.py ===") || out.contains("=== sub\\b.py ==="));
    assert!(out.contains("print('a')"));
    assert!(out.contains("print('b')"));
}

#[test]
fn read_files_skips_missing_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = read_files(
        &[
            tmp.path().join("nope.py"),
            tmp.path().join("also_missing.py"),
        ],
        tmp.path(),
    );
    assert!(out.is_empty());
}

#[test]
fn read_files_caps_at_char_limit() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("big.py");
    let huge: String = "x".repeat(1_000_000);
    fs::write(&path, &huge).expect("write fixture");
    let out = read_files(&[path], tmp.path());
    // FILE_CHAR_CAP is around 100k; output should be capped well under 1M.
    assert!(out.len() < huge.len());
}

#[test]
fn read_files_handles_paths_outside_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let outside = tmp
        .path()
        .parent()
        .expect("test invariant")
        .join("graphify_test_outside.py");
    fs::write(&outside, "x = 1").expect("write fixture");
    let result = read_files(std::slice::from_ref(&outside), tmp.path());
    // strip_prefix fails, so the full path is used.
    assert!(result.contains("graphify_test_outside.py"));
    let _ = fs::remove_file(outside);
}

// ── LlmResponse::to_value ──────────────────────────────────────────────────

#[test]
fn llm_response_to_value_emits_expected_keys() {
    let r = LlmResponse {
        nodes: vec![json!({"id": "a"})],
        edges: vec![json!({"source": "a", "target": "b"})],
        hyperedges: vec![],
        input_tokens: 10,
        output_tokens: 20,
        model: "test-model".into(),
        finish_reason: "stop".into(),
        elapsed_seconds: 1.5,
        failed_chunk_indices: vec![3],
    };
    let v = r.to_value();
    assert_eq!(v["input_tokens"], 10);
    assert_eq!(v["output_tokens"], 20);
    assert_eq!(v["finish_reason"], "stop");
    assert_eq!(v["model"], "test-model");
    assert_eq!(v["nodes"].as_array().expect("array field").len(), 1);
}

// ── read_files chunk variants ──────────────────────────────────────────────

#[test]
fn read_files_empty_list_returns_empty() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out: Vec<PathBuf> = vec![];
    assert!(read_files(&out, tmp.path()).is_empty());
}

#[test]
fn read_files_separator_between_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let a = tmp.path().join("a.py");
    let b = tmp.path().join("b.py");
    fs::write(&a, "A").expect("write fixture");
    fs::write(&b, "B").expect("write fixture");
    let out = read_files(&[a, b], tmp.path());
    assert!(out.contains("=== a.py ===\nA\n\n=== b.py ===\nB"));
}
