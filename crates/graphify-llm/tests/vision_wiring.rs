//! Vision-wiring tests: `extract_files_direct` routes raster images into the
//! per-backend vision payload (#1110). Each backend's `GRAPHIFY_<NAME>_BASE_URL`
//! override points the HTTP call at a mockito server whose `match_body` asserts
//! the image reached the request in the right shape.

#![allow(clippy::expect_used, clippy::unwrap_used, unsafe_code)]

use std::path::PathBuf;

use graphify_llm::extract_files_direct;
use serde_json::json;

/// RAII guard that sets/restores env vars (mirrors `per_backend_http.rs`).
struct EnvGuard {
    saved: Vec<(String, Option<String>)>,
}

impl EnvGuard {
    fn new() -> Self {
        Self { saved: vec![] }
    }
    fn set(&mut self, k: &str, v: &str) -> &mut Self {
        let prev = std::env::var(k).ok();
        unsafe { std::env::set_var(k, v) };
        self.saved.push((k.to_string(), prev));
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

/// Temp dir holding one `.png` and one `.txt`; returns `(dir, files, root)`.
/// The PNG content is arbitrary — `build_image_refs` reads bytes, it does not
/// validate the format.
fn fixture() -> (tempfile::TempDir, Vec<PathBuf>, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let img = root.join("diagram.png");
    std::fs::write(&img, b"\x89PNG\r\n\x1a\nFAKEPIXELS").expect("write png");
    let txt = root.join("notes.txt");
    std::fs::write(&txt, "hello world").expect("write txt");
    (dir, vec![img, txt], root)
}

fn openai_mock_body() -> String {
    json!({
        "choices": [{
            "message": {"content": "{\"nodes\":[{\"id\":\"x\"}],\"edges\":[]}"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 5, "completion_tokens": 8}
    })
    .to_string()
}

#[test]
fn openai_vision_sends_image_url_data_uri() {
    let (_dir, files, root) = fixture();
    let mut server = mockito::Server::new();
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::AllOf(vec![
            mockito::Matcher::Regex("image_url".into()),
            mockito::Matcher::Regex("data:image/png;base64,".into()),
        ]))
        .with_status(200)
        .with_body(openai_mock_body())
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());

    let resp = extract_files_direct(&files, "openai", Some("key"), None, &root).expect("extract");
    assert_eq!(resp.nodes.len(), 1);
    m.assert();
}

#[test]
fn claude_vision_sends_image_block() {
    let (_dir, files, root) = fixture();
    let mut server = mockito::Server::new();
    let body = json!({
        "content": [{"text": "{\"nodes\":[{\"id\":\"c\"}],\"edges\":[]}"}],
        "usage": {"input_tokens": 4, "output_tokens": 9},
        "stop_reason": "end_turn"
    });
    let m = server
        .mock("POST", "/v1/messages")
        .match_body(mockito::Matcher::AllOf(vec![
            // The body is serialized with whitespace after colons, so the
            // matchers tolerate it.
            mockito::Matcher::Regex(r#""type":\s*"image""#.into()),
            mockito::Matcher::Regex("base64".into()),
        ]))
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_CLAUDE_BASE_URL", &server.url());

    let resp = extract_files_direct(&files, "claude", Some("key"), None, &root).expect("extract");
    assert_eq!(resp.nodes.len(), 1);
    m.assert();
}

#[test]
fn deepseek_non_vision_sends_text_note() {
    // deepseek has no vision support: the image is stripped to a pixel-free ref
    // and surfaces only as the `=== IMAGES ===` text note, so it still becomes a
    // graph node. The matcher would not fire if pixels were inlined as a separate
    // request shape, and the absence of `image_url` parts is covered by the
    // `openai_content` unit tests in `vision.rs`.
    let (_dir, files, root) = fixture();
    let mut server = mockito::Server::new();
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::Regex("=== IMAGES ===".into()))
        .with_status(200)
        .with_body(openai_mock_body())
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_DEEPSEEK_BASE_URL", &server.url());

    let resp = extract_files_direct(&files, "deepseek", Some("key"), None, &root).expect("extract");
    assert_eq!(resp.nodes.len(), 1);
    m.assert();
}
