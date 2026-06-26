//! Parity tests against `graphify-py/tests/test_security.py`.
#![allow(clippy::expect_used)]
// `std::env::set_var` is unsafe in edition 2024 — test-only, serialised below.
#![allow(unsafe_code)]

use std::time::Duration;

use graphify_security::{
    MAX_FETCH_BYTES, MAX_GRAPH_FILE_BYTES, MAX_TEXT_BYTES, METADATA_MAX_LIST_ITEMS,
    METADATA_MAX_VALUE_LEN, SecurityError, check_graph_file_size_cap,
    check_graph_file_size_cap_with, safe_fetch, sanitize_label, sanitize_metadata,
    sanitize_metadata_map, sanitize_metadata_string, sanitize_metadata_value, test_support,
    validate_graph_path, validate_url,
};
use serde_json::{Map, Value, json};

// ---------------------------------------------------------------------------
// validate_url
// ---------------------------------------------------------------------------

#[test]
fn validate_url_accepts_http() {
    let url = validate_url("http://example.com/page").expect("valid http");
    assert_eq!(url, "http://example.com/page");
}

#[test]
fn validate_url_accepts_https() {
    let url = validate_url("https://arxiv.org/abs/1706.03762").expect("valid https");
    assert_eq!(url, "https://arxiv.org/abs/1706.03762");
}

#[test]
fn validate_url_rejects_file() {
    let err = validate_url("file:///etc/passwd").expect_err("file should be rejected");
    assert!(matches!(err, SecurityError::BlockedScheme { .. }));
    assert!(format!("{err}").contains("file"));
}

#[test]
fn validate_url_rejects_ftp() {
    let err = validate_url("ftp://files.example.com/data.zip").expect_err("ftp should be rejected");
    assert!(matches!(err, SecurityError::BlockedScheme { .. }));
    assert!(format!("{err}").contains("ftp"));
}

#[test]
fn validate_url_rejects_data() {
    let err = validate_url("data:text/html,<script>alert(1)</script>")
        .expect_err("data should be rejected");
    assert!(matches!(err, SecurityError::BlockedScheme { .. }));
    assert!(format!("{err}").contains("data"));
}

#[test]
fn validate_url_rejects_empty_scheme() {
    assert!(validate_url("//no-scheme.example.com").is_err());
}

#[test]
fn validate_url_rejects_private_ipv4_literal() {
    let err = validate_url("http://10.0.0.5/").expect_err("private IPv4 should be blocked");
    assert!(matches!(err, SecurityError::BlockedPrivateIp { .. }));
}

#[test]
fn validate_url_rejects_loopback_ipv4_literal() {
    let err = validate_url("http://127.0.0.1/").expect_err("loopback should be blocked");
    assert!(matches!(err, SecurityError::BlockedPrivateIp { .. }));
}

#[test]
fn validate_url_rejects_link_local_ipv4_literal() {
    let err = validate_url("http://169.254.169.254/").expect_err("link-local should be blocked");
    // The IMDS literal is in both the metadata-host allowlist and the
    // link-local CIDR, so either error is acceptable as long as the URL
    // is rejected.
    assert!(matches!(
        err,
        SecurityError::BlockedPrivateIp { .. } | SecurityError::BlockedMetadataHost { .. }
    ));
}

#[test]
fn validate_url_rejects_cgn_ipv4_literal() {
    let err = validate_url("http://100.64.1.2/").expect_err("CGN range should be blocked");
    assert!(matches!(err, SecurityError::BlockedPrivateIp { .. }));
}

#[test]
fn validate_url_rejects_metadata_host() {
    let err = validate_url("http://metadata.google.internal/")
        .expect_err("metadata host should be blocked");
    assert!(matches!(err, SecurityError::BlockedMetadataHost { .. }));
}

#[test]
fn validate_url_rejects_ipv6_loopback() {
    let err = validate_url("http://[::1]/").expect_err("ipv6 loopback should be blocked");
    assert!(matches!(err, SecurityError::BlockedPrivateIp { .. }));
}

// ---------------------------------------------------------------------------
// safe_fetch (mocked network)
// ---------------------------------------------------------------------------

#[test]
fn safe_fetch_rejects_file_url() {
    let err = safe_fetch(
        "file:///etc/passwd",
        MAX_FETCH_BYTES,
        Duration::from_secs(5),
    )
    .expect_err("file should be rejected");
    assert!(format!("{err}").contains("file"));
}

#[test]
fn safe_fetch_rejects_ftp_url() {
    let err = safe_fetch(
        "ftp://example.com/file.zip",
        MAX_FETCH_BYTES,
        Duration::from_secs(5),
    )
    .expect_err("ftp should be rejected");
    assert!(format!("{err}").contains("ftp"));
}

#[test]
fn net_safe_fetch_returns_bytes() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/")
        .with_status(200)
        .with_body("hello world")
        .create();
    let url = server.url();
    let result = test_support::fetch_allow_private(&url, MAX_FETCH_BYTES, Duration::from_secs(5))
        .expect("fetch ok");
    assert_eq!(result, b"hello world");
    mock.assert();
}

#[test]
fn net_safe_fetch_raises_on_non_2xx() {
    let mut server = mockito::Server::new();
    let _mock = server.mock("GET", "/missing").with_status(404).create();
    let url = format!("{}/missing", server.url());
    let err = test_support::fetch_allow_private(&url, MAX_FETCH_BYTES, Duration::from_secs(5))
        .expect_err("404 should fail");
    assert!(matches!(err, SecurityError::HttpStatus { status: 404, .. }));
}

#[test]
fn net_safe_fetch_raises_on_size_exceeded() {
    let big = "x".repeat(70_000);
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("GET", "/huge")
        .with_status(200)
        .with_body(big)
        .create();
    let url = format!("{}/huge", server.url());
    let err = test_support::fetch_allow_private(&url, 65_536, Duration::from_secs(5))
        .expect_err("size limit should be enforced");
    assert!(matches!(err, SecurityError::SizeLimitExceeded { .. }));
}

#[test]
fn net_safe_fetch_text_decodes_utf8() {
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("GET", "/")
        .with_status(200)
        .with_body("héllo wörld".as_bytes())
        .create();
    let url = server.url();
    let text = test_support::fetch_text_allow_private(&url, MAX_TEXT_BYTES, Duration::from_secs(5))
        .expect("fetch ok");
    assert_eq!(text, "héllo wörld");
}

#[test]
fn net_safe_fetch_text_replaces_bad_bytes() {
    let bad: &[u8] = b"hello \xff world";
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("GET", "/")
        .with_status(200)
        .with_body(bad)
        .create();
    let url = server.url();
    let text = test_support::fetch_text_allow_private(&url, MAX_TEXT_BYTES, Duration::from_secs(5))
        .expect("fetch ok");
    assert!(text.contains("hello"));
    assert!(text.contains("world"));
    assert!(!text.contains('\u{ff}'));
}

// ---------------------------------------------------------------------------
// validate_graph_path
// ---------------------------------------------------------------------------

#[test]
fn validate_graph_path_allows_inside_base() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("graphify-out");
    std::fs::create_dir(&base).expect("mkdir");
    let graph = base.join("graph.json");
    std::fs::write(&graph, "{}").expect("write");
    let result = validate_graph_path(&graph, Some(&base)).expect("inside base should be allowed");
    assert_eq!(result, graph.canonicalize().expect("canonicalize"));
}

#[test]
fn validate_graph_path_blocks_traversal() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("graphify-out");
    std::fs::create_dir(&base).expect("mkdir");
    let evil = base.join("..").join("etc_passwd");
    let err = validate_graph_path(&evil, Some(&base)).expect_err("traversal should be blocked");
    assert!(matches!(err, SecurityError::PathEscape { .. }));
}

#[test]
fn validate_graph_path_requires_base_exists() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("graphify-out");
    let err = validate_graph_path(base.join("graph.json"), Some(&base))
        .expect_err("missing base should fail");
    assert!(matches!(err, SecurityError::BaseMissing(_)));
}

#[test]
fn validate_graph_path_raises_if_file_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("graphify-out");
    std::fs::create_dir(&base).expect("mkdir");
    let err = validate_graph_path(base.join("missing.json"), Some(&base))
        .expect_err("missing file should fail");
    assert!(matches!(err, SecurityError::GraphFileMissing(_)));
}

/// RAII guard that sets an env var and restores it on drop.
struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        // SAFETY: test-only, serialised via `#[serial_test::serial]`.
        unsafe { std::env::set_var(key, value) };
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            // SAFETY: test-only cleanup.
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

#[test]
#[serial_test::serial(graphify_out_env)]
fn validate_graph_path_default_base_discovers_output_dir() {
    // With base omitted, the output dir is discovered by walking the path's
    // parents for the configured output-dir name (default "graphify-out").
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("graphify-out");
    std::fs::create_dir(&base).expect("mkdir");
    let graph = base.join("graph.json");
    std::fs::write(&graph, "{}").expect("write");
    let resolved =
        validate_graph_path(&graph, None).expect("default base should discover graphify-out");
    assert_eq!(resolved, graph.canonicalize().expect("canonicalize"));
}

#[test]
#[serial_test::serial(graphify_out_env)]
fn validate_graph_path_default_base_honours_graphify_out_override() {
    // base=None discovery must honour GRAPHIFY_OUT, not the hardcoded literal,
    // so a renamed output dir validates against the right base (#1423).
    let _guard = EnvGuard::set("GRAPHIFY_OUT", "custom-out");
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = tmp.path().join("custom-out");
    std::fs::create_dir(&out).expect("mkdir");
    let graph = out.join("graph.json");
    std::fs::write(&graph, "{}").expect("write");
    let resolved =
        validate_graph_path(&graph, None).expect("override base should discover custom-out");
    assert_eq!(resolved, graph.canonicalize().expect("canonicalize"));
}

// ---------------------------------------------------------------------------
// sanitize_label
// ---------------------------------------------------------------------------

#[test]
fn sanitize_label_passthrough_html_chars() {
    // sanitize_label does NOT HTML-escape — callers that inject into HTML
    // must wrap with htmlescape themselves.
    assert_eq!(sanitize_label(Some("<script>")), "<script>");
    assert_eq!(sanitize_label(Some("foo & bar")), "foo & bar");
}

#[test]
fn sanitize_label_strips_control_chars() {
    let result = sanitize_label(Some("hello\x00\x1fworld"));
    assert!(!result.contains('\x00'));
    assert!(!result.contains('\x1f'));
    assert!(result.contains("helloworld"));
}

#[test]
fn sanitize_label_caps_at_256() {
    let long = "a".repeat(300);
    let result = sanitize_label(Some(&long));
    assert!(result.chars().count() <= 256);
}

#[test]
fn sanitize_label_safe_passthrough() {
    assert_eq!(sanitize_label(Some("MyClass")), "MyClass");
    assert_eq!(sanitize_label(Some("extract_python")), "extract_python");
}

#[test]
fn sanitize_label_none_returns_empty() {
    assert_eq!(sanitize_label(None), "");
}

// ---------------------------------------------------------------------------
// check_graph_file_size_cap
// ---------------------------------------------------------------------------

#[test]
fn graph_size_cap_default_is_512_mib() {
    assert_eq!(MAX_GRAPH_FILE_BYTES, 512 * 1024 * 1024);
}

#[test]
fn graph_size_cap_under_limit_returns_ok() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = tmp.path().join("graph.json");
    std::fs::write(&p, br#"{"nodes": [], "links": []}"#).expect("write");
    check_graph_file_size_cap(&p).expect("under limit should pass");
}

#[test]
fn graph_size_cap_over_limit_raises() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = tmp.path().join("graph.json");
    std::fs::write(&p, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").expect("write");
    let err = check_graph_file_size_cap_with(&p, 16).expect_err("over limit should fail");
    assert!(matches!(err, SecurityError::GraphFileTooLarge { .. }));
    let msg = format!("{err}");
    assert!(msg.contains("exceeds"), "msg: {msg}");
}

#[test]
fn graph_size_cap_error_message_includes_size_and_cap() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = tmp.path().join("graph.json");
    std::fs::write(&p, "AAAAAAAAAAAAAAAA").expect("write"); // 16 bytes
    let err = check_graph_file_size_cap_with(&p, 8).expect_err("over limit should fail");
    let msg = format!("{err}");
    assert!(msg.contains("16"), "msg: {msg}");
    assert!(msg.contains('8'), "msg: {msg}");
    assert!(msg.to_lowercase().contains("byte"), "msg: {msg}");
}

#[test]
fn graph_size_cap_at_boundary_passes_then_fails() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = tmp.path().join("graph.json");
    std::fs::write(&p, "A".repeat(32)).expect("write");
    check_graph_file_size_cap_with(&p, 32).expect("equal to cap allowed");
    let err = check_graph_file_size_cap_with(&p, 31).expect_err("strictly greater rejected");
    assert!(matches!(err, SecurityError::GraphFileTooLarge { .. }));
}

#[test]
fn graph_size_cap_missing_file_silently_returns() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing = tmp.path().join("does_not_exist.json");
    check_graph_file_size_cap(&missing).expect("missing file silently ok");
}

// ---------------------------------------------------------------------------
// sanitize_metadata
// ---------------------------------------------------------------------------

#[test]
fn metadata_string_strips_control_chars() {
    let result = sanitize_metadata_string("hello\x00\x1fworld");
    assert!(!result.contains('\x00'));
    assert!(!result.contains('\x1f'));
    assert!(result.contains("helloworld"));
}

#[test]
fn metadata_string_escapes_html() {
    let result = sanitize_metadata_string("<script>alert('x')</script>");
    assert!(result.contains("&lt;"));
    assert!(result.contains("&gt;"));
    assert!(!result.contains("<script>"));
}

#[test]
fn metadata_string_escapes_quotes() {
    let result = sanitize_metadata_string("a\"b'c");
    assert!(result.contains("&quot;"));
    assert!(result.contains("&#x27;"));
}

#[test]
fn metadata_string_caps_length() {
    let long = "a".repeat(METADATA_MAX_VALUE_LEN + 100);
    let result = sanitize_metadata_string(&long);
    assert!(result.chars().count() <= METADATA_MAX_VALUE_LEN);
}

#[test]
fn metadata_value_preserves_simple_types() {
    assert_eq!(sanitize_metadata_value(&json!(42)), json!(42));
    assert!(
        (sanitize_metadata_value(&json!(2.5))
            .as_f64()
            .expect("f64 field")
            - 2.5)
            .abs()
            < 1e-9
    );
    assert_eq!(sanitize_metadata_value(&json!(true)), json!(true));
    assert_eq!(sanitize_metadata_value(&json!(false)), json!(false));
    assert_eq!(sanitize_metadata_value(&Value::Null), Value::Null);
}

#[test]
fn metadata_value_recurses_into_dict() {
    let input = json!({ "k": "<script>x</script>" });
    let out = sanitize_metadata_value(&input);
    let obj = out.as_object().expect("dict");
    let v = obj.get("k").expect("k").as_str().expect("str");
    assert!(v.contains("&lt;"));
}

#[test]
fn metadata_value_recurses_into_list() {
    let input = json!(["<a>", "<b>", "<c>"]);
    let out = sanitize_metadata_value(&input);
    let arr = out.as_array().expect("array");
    assert!(
        arr.iter()
            .all(|v| v.as_str().expect("string field").contains("&lt;"))
    );
}

#[test]
fn metadata_value_caps_list_length() {
    let items: Vec<Value> = (0..METADATA_MAX_LIST_ITEMS * 3).map(|n| json!(n)).collect();
    let out = sanitize_metadata_value(&Value::Array(items));
    let arr = out.as_array().expect("array");
    assert_eq!(arr.len(), METADATA_MAX_LIST_ITEMS);
}

#[test]
fn metadata_none_returns_empty_map() {
    assert!(sanitize_metadata(None).is_empty());
}

#[test]
fn metadata_drops_empty_key() {
    let mut map = Map::new();
    map.insert("\x00".to_owned(), json!("v"));
    map.insert("k".to_owned(), json!("v2"));
    let out = sanitize_metadata_map(&map);
    assert!(!out.contains_key("\x00"));
    assert_eq!(out.get("k"), Some(&json!("v2")));
    assert_eq!(out.len(), 1);
}

#[test]
fn metadata_sanitizes_keys() {
    let mut map = Map::new();
    map.insert("<bad>".to_owned(), json!("v"));
    let out = sanitize_metadata_map(&map);
    assert!(!out.contains_key("<bad>"));
    assert!(out.keys().any(|k| k.contains("&lt;")));
}

#[test]
fn metadata_recursive_nested() {
    let raw = json!({
        "outer": {
            "inner": "<script>x</script>",
            "list": ["a", "<b>", 99, null, true],
        },
        "scalar": 42,
    });
    let map = raw.as_object().expect("obj").clone();
    let out = sanitize_metadata(Some(&map));
    let outer = out.get("outer").expect("outer").as_object().expect("obj");
    let inner = outer.get("inner").expect("inner").as_str().expect("str");
    assert!(inner.contains("&lt;"));
    let items = outer.get("list").expect("list").as_array().expect("arr");
    assert_eq!(items[0], json!("a"));
    assert!(items[1].as_str().expect("string field").contains("&lt;"));
    assert_eq!(items[2], json!(99));
    assert_eq!(items[3], Value::Null);
    assert_eq!(items[4], json!(true));
    assert_eq!(out.get("scalar"), Some(&json!(42)));
}

#[test]
fn metadata_bool_not_coerced_to_int() {
    let map = json!({"flag_t": true, "flag_f": false, "num": 1})
        .as_object()
        .expect("obj")
        .clone();
    let out = sanitize_metadata(Some(&map));
    assert_eq!(out.get("flag_t"), Some(&json!(true)));
    assert_eq!(out.get("flag_f"), Some(&json!(false)));
    assert_eq!(out.get("num"), Some(&json!(1)));
}
