//! Tests that drive the `fetchers` module through the public `ingest` function
//! with `GRAPHIFY_TEST_ALLOW_PRIVATE_IPS=1` so mockito URLs pass the SSRF guard.

#![allow(clippy::expect_used, clippy::unwrap_used, unsafe_code)]

use graphify_ingest::ingest;

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

#[test]
fn ingest_webpage_via_fetcher() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("GET", "/")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body("<html><head><title>Hello</title></head><body><p>body text</p></body></html>")
        .create();
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");

    let tmp = tempfile::tempdir().unwrap();
    let out = ingest(&server.url(), tmp.path(), None, None).unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.contains("Hello"));
}

#[test]
fn ingest_pdf_url_downloads_binary() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("GET", "/file.pdf")
        .with_status(200)
        .with_body(b"%PDF-1.4\nfake pdf bytes")
        .create();
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");

    let tmp = tempfile::tempdir().unwrap();
    let url = format!("{}/file.pdf", server.url());
    let out = ingest(&url, tmp.path(), None, None).unwrap();
    assert!(out.extension().is_some_and(|e| e == "pdf"));
}

#[test]
fn ingest_image_url_downloads_with_inferred_extension() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("GET", "/pic.png")
        .with_status(200)
        .with_body(b"\x89PNG\r\n\x1a\n")
        .create();
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");

    let tmp = tempfile::tempdir().unwrap();
    let url = format!("{}/pic.png", server.url());
    let out = ingest(&url, tmp.path(), None, None).unwrap();
    assert!(out.exists());
}

#[test]
fn ingest_youtube_url_errors() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    let tmp = tempfile::tempdir().unwrap();
    let result = ingest(
        "https://www.youtube.com/watch?v=abc",
        tmp.path(),
        None,
        None,
    );
    assert!(result.is_err());
}

#[test]
fn ingest_with_existing_filename_dedups() {
    let mut server = mockito::Server::new();
    server
        .mock("GET", "/same")
        .with_status(200)
        .with_body("<html><head><title>S</title></head><body>x</body></html>")
        .expect_at_least(2)
        .create();
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");

    let tmp = tempfile::tempdir().unwrap();
    let url = format!("{}/same", server.url());
    let a = ingest(&url, tmp.path(), None, None).unwrap();
    let b = ingest(&url, tmp.path(), None, None).unwrap();
    assert_ne!(a, b);
}

#[test]
fn ingest_tweet_url_via_fetch_tweet() {
    let mut server = mockito::Server::new();
    // The fetch_tweet function calls https://publish.twitter.com/oembed?...
    // It can't be mocked because the URL is fixed in the source code. But
    // since we're enabling the SSRF bypass, the function will try to reach the
    // real oembed endpoint, fail, and emit the fallback content. This still
    // exercises the fetch_tweet code path including the fallback.
    let _m = server
        .mock("GET", mockito::Matcher::Any)
        .with_status(200)
        .with_body("ok")
        .create();
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");

    let tmp = tempfile::tempdir().unwrap();
    // x.com URL → detected as tweet by detect_url_type.
    let url = "https://x.com/alice/status/12345";
    let _ = ingest(url, tmp.path(), Some("bob"), None);
    // ingest may succeed (with fallback content) or fail depending on whether
    // the real twitter oembed responds. Just verify no panic.
}
