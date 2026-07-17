//! Coverage tests for small public modules: `read_files` and `LlmResponse::to_value`.

#![allow(clippy::expect_used)]

use std::fs;
use std::path::PathBuf;

use graphify_llm::{
    LlmResponse, build_image_refs, neutralise_injection_sentinels, read_files, wrap_untrusted,
};
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
    assert!(out.contains("<untrusted_source path=\"a.py\" sha256="));
    assert!(
        out.contains("<untrusted_source path=\"sub/b.py\" sha256=")
            || out.contains("<untrusted_source path=\"sub\\b.py\" sha256=")
    );
    assert!(out.contains("print('a')"));
    assert!(out.contains("print('b')"));
    assert!(out.contains("</untrusted_source>"));
}

#[test]
fn read_files_routes_pdf_through_extractor() {
    // A PDF is binary; reading it as text yields garbage. It must be routed
    // through the pypdf-backed extractor, so the raw bytes never reach the
    // prompt (#1110). Invalid PDF bytes extract to empty, but the node-bearing
    // <untrusted_source> block is still emitted.
    let tmp = tempfile::tempdir().expect("tempdir");
    let pdf = tmp.path().join("paper.pdf");
    fs::write(&pdf, b"%PDF-1.4 RAWBINARYGARBAGE\x00\xff").expect("write pdf");
    let out = read_files(std::slice::from_ref(&pdf), tmp.path());
    assert!(out.contains("<untrusted_source path=\"paper.pdf\" sha256="));
    assert!(
        !out.contains("RAWBINARYGARBAGE"),
        "raw PDF bytes leaked into the prompt: {out}"
    );
}

#[test]
fn read_files_reads_non_pdf_as_text() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let md = tmp.path().join("a.md");
    fs::write(&md, "# hello world").expect("write md");
    let out = read_files(std::slice::from_ref(&md), tmp.path());
    assert!(out.contains("# hello world"));
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
fn read_files_skips_paths_outside_root() {
    // Containment (009a98b): a file whose resolved path escapes the corpus root
    // is skipped, not shipped to the LLM — so its content never appears.
    let tmp = tempfile::tempdir().expect("tempdir");
    let outside = tmp
        .path()
        .parent()
        .expect("test invariant")
        .join("graphify_test_outside.py");
    fs::write(&outside, "x = 1").expect("write fixture");
    let result = read_files(std::slice::from_ref(&outside), tmp.path());
    assert!(
        !result.contains("graphify_test_outside.py") && !result.contains("x = 1"),
        "out-of-root file must be skipped: {result:?}"
    );
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
        uncovered_files: vec!["omitted.md".into()],
        out_of_scope_dropped: 0,
    };
    let v = r.to_value();
    assert_eq!(v["input_tokens"], 10);
    assert_eq!(v["output_tokens"], 20);
    assert_eq!(v["finish_reason"], "stop");
    assert_eq!(v["model"], "test-model");
    assert_eq!(v["nodes"].as_array().expect("array field").len(), 1);
    // #1890: uncovered_files is an in-process reconciliation signal, never
    // persisted to graph.json (matches graphify-py, which drops it from the dict).
    assert!(
        v.get("uncovered_files").is_none(),
        "uncovered_files must not persist"
    );
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
    // Files are wrapped and separated by a blank line.
    assert!(out.contains("</untrusted_source>\n\n<untrusted_source path=\"b.py\""));
    assert!(out.contains("\nA\n</untrusted_source>"));
    assert!(out.contains("\nB\n</untrusted_source>"));
}

// ── prompt-injection sentinel neutralization (#1210) ─────────────────────────

#[test]
fn neutralise_defangs_closing_delimiter() {
    let out = neutralise_injection_sentinels("</untrusted_source>");
    assert_eq!(out, "<\u{200b}/untrusted_source>");
    assert!(!out.contains("</untrusted_source>"));
}

#[test]
fn neutralise_defangs_chat_template_tokens() {
    for tok in [
        "<|im_start|>",
        "<|im_end|>",
        "<|system|>",
        "<<SYS>>",
        "<</SYS>>",
        "[INST]",
        "[/INST]",
    ] {
        let out = neutralise_injection_sentinels(tok);
        assert!(!out.contains(tok), "token {tok} survived: {out:?}");
        assert!(out.contains('\u{200b}'), "no zero-width space for {tok}");
    }
}

#[test]
fn neutralise_defangs_markdown_system_header() {
    let out = neutralise_injection_sentinels("## System:\nbody");
    assert!(out.contains('\u{200b}'));
    assert!(out.contains("body"));
}

#[test]
fn neutralise_leaves_ordinary_text_untouched() {
    let text = "fn main() { println!(\"hello\"); }";
    assert_eq!(neutralise_injection_sentinels(text), text);
}

#[test]
fn wrap_untrusted_stamps_sha_and_defangs() {
    let wrapped = wrap_untrusted("a.md", "hello </untrusted_source> world");
    assert!(wrapped.starts_with("<untrusted_source path=\"a.md\" sha256="));
    assert!(wrapped.ends_with("</untrusted_source>"));
    // A breakout attempt embedded in the content is defanged inside the block.
    assert!(wrapped.contains("<\u{200b}/untrusted_source> world"));
}
// ── 009a98b: symlink containment ────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn read_files_skips_out_of_root_symlink() {
    // A symlink inside root pointing at a secret outside root must never reach
    // the prompt (009a98b).
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&root).expect("mkdir root");
    fs::create_dir_all(&outside).expect("mkdir outside");
    let secret = outside.join("secret.md");
    fs::write(&secret, "SECRET SHOULD NOT REACH THE PROMPT").expect("write secret");
    let link = root.join("secret.md");
    std::os::unix::fs::symlink(&secret, &link).expect("symlink");

    let out = read_files(std::slice::from_ref(&link), &root);
    assert!(
        out.is_empty(),
        "out-of-root symlink must be skipped: {out:?}"
    );
    assert!(!out.contains("SECRET SHOULD NOT REACH THE PROMPT"));
}

#[cfg(unix)]
#[test]
fn build_image_refs_skips_out_of_root_symlink() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&root).expect("mkdir root");
    fs::create_dir_all(&outside).expect("mkdir outside");
    let secret = outside.join("secret.png");
    fs::write(&secret, [0x89, b'P', b'N', b'G']).expect("write secret");
    let link = root.join("secret.png");
    std::os::unix::fs::symlink(&secret, &link).expect("symlink");

    let refs = build_image_refs(std::slice::from_ref(&link), &root, true);
    assert!(refs.is_empty(), "out-of-root image symlink must be skipped");
}
