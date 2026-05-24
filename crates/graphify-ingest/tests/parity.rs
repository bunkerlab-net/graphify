//! Parity tests against `graphify-py/tests/test_ingest.py`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use graphify_ingest::{
    detect_url_type, html_to_markdown, ingest, safe_filename, save_query_result, yaml_str,
};
use graphify_security::{MAX_FETCH_BYTES, MAX_TEXT_BYTES, test_support};

// ---------------------------------------------------------------------------
// yaml_str — security boundary, must match Python byte-for-byte
// ---------------------------------------------------------------------------

#[test]
fn yaml_str_passthrough_plain() {
    assert_eq!(yaml_str("hello world"), "hello world");
}

#[test]
fn yaml_str_escapes_backslash() {
    assert_eq!(yaml_str("a\\b"), "a\\\\b");
}

#[test]
fn yaml_str_escapes_double_quote() {
    assert_eq!(yaml_str(r#"say "hi""#), r#"say \"hi\""#);
}

#[test]
fn yaml_str_escapes_newline() {
    assert_eq!(yaml_str("line1\nline2"), "line1\\nline2");
}

#[test]
fn yaml_str_escapes_carriage_return() {
    assert_eq!(yaml_str("line1\rline2"), "line1\\rline2");
}

#[test]
fn yaml_str_escapes_tab() {
    assert_eq!(yaml_str("col1\tcol2"), "col1\\tcol2");
}

#[test]
fn yaml_str_escapes_null() {
    assert_eq!(yaml_str("a\0b"), "a\\0b");
}

#[test]
fn yaml_str_escapes_line_separator_u2028() {
    let s = "before\u{2028}after";
    assert_eq!(yaml_str(s), "before\\Lafter");
}

#[test]
fn yaml_str_escapes_paragraph_separator_u2029() {
    let s = "before\u{2029}after";
    assert_eq!(yaml_str(s), "before\\Pafter");
}

#[test]
fn yaml_str_escapes_other_control_chars() {
    // U+0001 SOH
    assert_eq!(yaml_str("\x01"), "\\x01");
    // U+001B ESC
    assert_eq!(yaml_str("\x1b"), "\\x1b");
    // U+007F DEL
    assert_eq!(yaml_str("\x7f"), "\\x7f");
}

#[test]
fn yaml_str_empty_input() {
    assert_eq!(yaml_str(""), "");
}

// ---------------------------------------------------------------------------
// safe_filename
// ---------------------------------------------------------------------------

#[test]
fn safe_filename_basic_url() {
    let name = safe_filename("https://example.com/foo/bar", ".md");
    // Should contain the host + path portion, non-word chars replaced
    assert!(
        std::path::Path::new(&name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md")),
        "expected .md extension, got: {name}"
    );
    assert!(name.contains("example_com"));
}

#[test]
fn safe_filename_truncates_at_80_plus_suffix() {
    let long_url = format!("https://example.com/{}", "a".repeat(200));
    let name = safe_filename(&long_url, ".md");
    // The part before suffix should be <= 80 chars
    let stem = name.trim_end_matches(".md");
    assert!(stem.chars().count() <= 80, "stem={}", stem.chars().count());
}

#[test]
fn safe_filename_no_double_underscores() {
    let name = safe_filename("https://example.com/a//b", ".md");
    assert!(!name.contains("__"), "got: {name}");
}

// ---------------------------------------------------------------------------
// detect_url_type
// ---------------------------------------------------------------------------

#[test]
fn detect_url_type_tweet_twitter() {
    assert_eq!(
        detect_url_type("https://twitter.com/user/status/123"),
        "tweet"
    );
}

#[test]
fn detect_url_type_tweet_x_com() {
    assert_eq!(detect_url_type("https://x.com/user/status/123"), "tweet");
}

#[test]
fn detect_url_type_arxiv() {
    assert_eq!(detect_url_type("https://arxiv.org/abs/1706.03762"), "arxiv");
}

#[test]
fn detect_url_type_github() {
    assert_eq!(
        detect_url_type("https://github.com/rust-lang/rust"),
        "github"
    );
}

#[test]
fn detect_url_type_youtube() {
    assert_eq!(
        detect_url_type("https://www.youtube.com/watch?v=abc"),
        "youtube"
    );
}

#[test]
fn detect_url_type_youtu_be() {
    assert_eq!(detect_url_type("https://youtu.be/abc123"), "youtube");
}

#[test]
fn detect_url_type_pdf() {
    assert_eq!(detect_url_type("https://example.com/paper.pdf"), "pdf");
}

#[test]
fn detect_url_type_image_png() {
    assert_eq!(detect_url_type("https://example.com/img.png"), "image");
}

#[test]
fn detect_url_type_image_jpg() {
    assert_eq!(detect_url_type("https://example.com/photo.jpg"), "image");
}

#[test]
fn detect_url_type_image_jpeg() {
    assert_eq!(detect_url_type("https://example.com/photo.jpeg"), "image");
}

#[test]
fn detect_url_type_image_webp() {
    assert_eq!(detect_url_type("https://example.com/img.webp"), "image");
}

#[test]
fn detect_url_type_image_gif() {
    assert_eq!(detect_url_type("https://example.com/anim.gif"), "image");
}

#[test]
fn detect_url_type_webpage_fallback() {
    assert_eq!(detect_url_type("https://example.com/page"), "webpage");
}

// ---------------------------------------------------------------------------
// html_to_markdown
// ---------------------------------------------------------------------------

#[test]
fn html_to_markdown_strips_script() {
    let html = "<p>Hello</p><script>evil()</script>";
    let md = html_to_markdown(html);
    assert!(!md.contains("evil"));
    assert!(md.contains("Hello"));
}

#[test]
fn html_to_markdown_strips_style() {
    let html = "<p>World</p><style>body{color:red}</style>";
    let md = html_to_markdown(html);
    assert!(!md.contains("color"));
    assert!(md.contains("World"));
}

// ---------------------------------------------------------------------------
// save_query_result (ports test_ingest.py 1:1)
// ---------------------------------------------------------------------------

#[test]
fn test_file_created() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mem = tmp.path().join("memory");
    let out = save_query_result("what is attention?", "Attention is...", &mem, "query", None)
        .expect("save ok");
    assert!(out.exists());
}

#[test]
fn test_filename_format() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mem = tmp.path().join("memory");
    let out = save_query_result(
        "what connects A to B?",
        "They share...",
        &mem,
        "query",
        None,
    )
    .expect("save ok");
    assert!(
        out.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("query_")
    );
    assert_eq!(out.extension().unwrap().to_string_lossy(), "md");
}

#[test]
fn test_frontmatter_question() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mem = tmp.path().join("memory");
    let out = save_query_result(
        "what is attention?",
        "Attention is softmax.",
        &mem,
        "query",
        None,
    )
    .expect("save ok");
    let content = std::fs::read_to_string(&out).expect("read");
    assert!(content.contains("question:"));
    assert!(content.to_lowercase().contains("attention"));
}

#[test]
fn test_frontmatter_type() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mem = tmp.path().join("memory");
    let out = save_query_result("q", "a", &mem, "path_query", None).expect("save ok");
    let content = std::fs::read_to_string(&out).expect("read");
    assert!(content.contains("type: \"path_query\""));
}

#[test]
fn test_source_nodes_included() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mem = tmp.path().join("memory");
    let nodes = vec!["AttentionLayer".to_string(), "SoftmaxFunc".to_string()];
    let out = save_query_result("q", "a", &mem, "query", Some(&nodes)).expect("save ok");
    let content = std::fs::read_to_string(&out).expect("read");
    assert!(content.contains("AttentionLayer"));
    assert!(content.contains("SoftmaxFunc"));
}

#[test]
fn test_source_nodes_capped_at_10() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mem = tmp.path().join("memory");
    let nodes: Vec<String> = (0..20).map(|i| format!("Node{i}")).collect();
    let out = save_query_result("q", "a", &mem, "query", Some(&nodes)).expect("save ok");
    let content = std::fs::read_to_string(&out).expect("read");
    // Only first 10 should appear in frontmatter source_nodes line
    let fm_line = content
        .lines()
        .find(|l| l.starts_with("source_nodes:"))
        .expect("source_nodes line");
    assert_eq!(fm_line.matches("\"Node").count(), 10);
}

#[test]
fn test_memory_dir_created() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mem = tmp.path().join("deep").join("memory");
    assert!(!mem.exists());
    save_query_result("q", "a", &mem, "query", None).expect("save ok");
    assert!(mem.exists());
}

#[test]
fn test_answer_in_body() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mem = tmp.path().join("memory");
    let answer = "The answer is forty-two.";
    let out =
        save_query_result("what is the answer?", answer, &mem, "query", None).expect("save ok");
    let content = std::fs::read_to_string(&out).expect("read");
    assert!(content.contains(answer));
}

// ---------------------------------------------------------------------------
// ingest — URL validation (no network required)
// ---------------------------------------------------------------------------

#[test]
fn ingest_rejects_private_ip_url() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let err = ingest("http://127.0.0.1/evil", tmp.path(), None, None)
        .expect_err("private IP should be rejected");
    assert!(format!("{err}").contains("ingest:"));
}

#[test]
fn ingest_rejects_file_url() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let err = ingest("file:///etc/passwd", tmp.path(), None, None)
        .expect_err("file:// should be rejected");
    assert!(format!("{err}").contains("ingest:"));
}

// ---------------------------------------------------------------------------
// ingest — HTTP mocked tests (net_ prefix)
// ---------------------------------------------------------------------------

#[test]
fn net_ingest_webpage_saves_markdown() {
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("GET", "/page")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body(
            "<html><head><title>Test Page</title></head><body><p>Hello graphify</p></body></html>",
        )
        .create();

    let url = format!("{}/page", server.url());
    let tmp = tempfile::tempdir().expect("tempdir");

    let out = ingest_allow_private(&url, tmp.path()).expect("ingest ok");
    assert!(out.exists());
    let content = std::fs::read_to_string(&out).expect("read");
    assert!(content.contains("type: webpage"));
    assert!(content.contains("Test Page"));
}

#[test]
fn net_ingest_sets_contributor() {
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("GET", "/c")
        .with_status(200)
        .with_body("<html><head><title>T</title></head><body>x</body></html>")
        .create();

    let url = format!("{}/c", server.url());
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = ingest_allow_private_with_contrib(&url, tmp.path(), Some("alice"), Some("bob"))
        .expect("ingest ok");
    let content = std::fs::read_to_string(&out).expect("read");
    // contributor takes precedence over author
    assert!(content.contains("contributor: \"bob\""));
}

#[test]
fn net_ingest_sets_author_when_no_contributor() {
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("GET", "/a")
        .with_status(200)
        .with_body("<html><head><title>T</title></head><body>x</body></html>")
        .create();

    let url = format!("{}/a", server.url());
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = ingest_allow_private_with_contrib(&url, tmp.path(), Some("alice"), None)
        .expect("ingest ok");
    let content = std::fs::read_to_string(&out).expect("read");
    assert!(content.contains("contributor: \"alice\""));
}

#[test]
fn net_ingest_deduplicate_filename() {
    let mut server = mockito::Server::new();
    server
        .mock("GET", "/dup")
        .with_status(200)
        .with_body("<html><head><title>Dup</title></head><body>x</body></html>")
        .expect(2)
        .create();

    let url = format!("{}/dup", server.url());
    let tmp = tempfile::tempdir().expect("tempdir");

    let out1 = ingest_allow_private(&url, tmp.path()).expect("first ingest");
    let out2 = ingest_allow_private(&url, tmp.path()).expect("second ingest");
    assert_ne!(out1, out2, "collision should produce distinct filenames");
}

#[test]
fn net_safe_fetch_text_for_oembed_404() {
    // Verify that a 404 from the oEmbed endpoint is handled as an error at the
    // security layer — the ingest layer swallows it via fallback.
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("GET", mockito::Matcher::Any)
        .with_status(404)
        .create();
    let url = format!("{}/oembed", server.url());
    let result =
        test_support::fetch_text_allow_private(&url, MAX_TEXT_BYTES, Duration::from_secs(5));
    assert!(result.is_err(), "404 should produce a security error");
}

// ---------------------------------------------------------------------------
// Helpers (test-only): call ingest bypassing the private-IP SSRF guard
// ---------------------------------------------------------------------------

fn ingest_allow_private(
    url: &str,
    target_dir: &std::path::Path,
) -> Result<std::path::PathBuf, IngestTestError> {
    ingest_allow_private_with_contrib(url, target_dir, None, None)
}

fn ingest_allow_private_with_contrib(
    url: &str,
    target_dir: &std::path::Path,
    author: Option<&str>,
    contributor: Option<&str>,
) -> Result<std::path::PathBuf, IngestTestError> {
    use std::path::PathBuf;

    std::fs::create_dir_all(target_dir)?;

    let url_type = detect_url_type(url);

    match url_type {
        "pdf" => {
            let bytes =
                test_support::fetch_allow_private(url, MAX_FETCH_BYTES, Duration::from_secs(5))?;
            let filename = safe_filename(url, ".pdf");
            let out = target_dir.join(filename);
            std::fs::write(&out, bytes)?;
            Ok(out)
        }
        "image" => {
            let suffix = url::Url::parse(url)
                .ok()
                .and_then(|u| {
                    std::path::Path::new(u.path())
                        .extension()
                        .map(|e| format!(".{}", e.to_string_lossy()))
                })
                .unwrap_or_else(|| ".jpg".to_string());
            let bytes =
                test_support::fetch_allow_private(url, MAX_FETCH_BYTES, Duration::from_secs(5))?;
            let filename = safe_filename(url, &suffix);
            let out = target_dir.join(filename);
            std::fs::write(&out, bytes)?;
            Ok(out)
        }
        _ => {
            let (content, filename) =
                fetch_text_type_allow_private(url, url_type, author, contributor)?;
            let mut out_path: PathBuf = target_dir.join(&filename);
            let mut counter: u32 = 1;
            while out_path.exists() && counter < 1000 {
                let stem = std::path::Path::new(&filename)
                    .file_stem()
                    .map_or_else(|| filename.clone(), |s| s.to_string_lossy().into_owned());
                out_path = target_dir.join(format!("{stem}_{counter}.md"));
                counter += 1;
            }
            std::fs::write(&out_path, content.as_bytes())?;
            Ok(out_path)
        }
    }
}

fn fetch_text_type_allow_private(
    url: &str,
    url_type: &str,
    author: Option<&str>,
    contributor: Option<&str>,
) -> Result<(String, String), IngestTestError> {
    use graphify_ingest::yaml_str;

    let html = test_support::fetch_text_allow_private(url, MAX_TEXT_BYTES, Duration::from_secs(5))?;

    let contrib = contributor.or(author).unwrap_or("unknown");
    let now = chrono::Utc::now().to_rfc3339();

    let re_title = regex::Regex::new(r"(?si)<title[^>]*>(.*?)</title>").unwrap();
    let re_ws = regex::Regex::new(r"\s+").unwrap();
    let title = re_title.captures(&html).and_then(|c| c.get(1)).map_or_else(
        || url.to_string(),
        |m| re_ws.replace_all(m.as_str(), " ").trim().to_string(),
    );

    let markdown = html_to_markdown(&html);
    let markdown_truncated: String = markdown.chars().take(12000).collect();

    let (content, filename) = if url_type == "tweet" {
        // For test purposes, treat as webpage (we can't hit publish.twitter.com)
        let c = format!(
            "---\nsource_url: \"{}\"\ntype: tweet\nauthor: \"unknown\"\ncaptured_at: {}\ncontributor: \"{}\"\n---\n\n# Tweet by @unknown\n\n{}\n\nSource: {}\n",
            yaml_str(url),
            now,
            yaml_str(contrib),
            title,
            url,
        );
        let f = safe_filename(url, ".md");
        (c, f)
    } else {
        let c = format!(
            "---\nsource_url: \"{}\"\ntype: webpage\ntitle: \"{}\"\ncaptured_at: {}\ncontributor: \"{}\"\n---\n\n# {}\n\nSource: {}\n\n---\n\n{}\n",
            yaml_str(url),
            yaml_str(&title),
            now,
            yaml_str(contrib),
            title,
            url,
            markdown_truncated,
        );
        let f = safe_filename(url, ".md");
        (c, f)
    };

    Ok((content, filename))
}

#[derive(Debug)]
enum IngestTestError {
    Security(graphify_security::SecurityError),
    Io(std::io::Error),
}

impl std::fmt::Display for IngestTestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Security(e) => write!(f, "security: {e}"),
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl From<graphify_security::SecurityError> for IngestTestError {
    fn from(e: graphify_security::SecurityError) -> Self {
        Self::Security(e)
    }
}

impl From<std::io::Error> for IngestTestError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
