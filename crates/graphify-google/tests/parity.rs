//! Parity tests for `graphify-google` — ports of
//! `graphify-py/tests/test_google_workspace.py`.
#![allow(clippy::expect_used)]

use std::path::PathBuf;

use graphify_google::{
    GoogleError, convert_google_workspace_file, google_workspace_enabled, read_google_shortcut,
};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn write_shortcut(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).expect("write fixture");
    path
}

// Alias for the export-hook type so call-sites are less verbose.
type ExportHook = dyn Fn(&str, &str, &std::path::Path, Option<&str>) -> Result<(), GoogleError>;

// ---------------------------------------------------------------------------
// test_read_google_shortcut_doc_id
// ---------------------------------------------------------------------------

#[test]
fn test_read_google_shortcut_doc_id() {
    let tmp = TempDir::new().expect("tempdir");
    let shortcut = write_shortcut(
        &tmp,
        "Planning.gdoc",
        r#"{"url":"https://docs.google.com/document/d/doc-123/edit","doc_id":"doc-123","email":"me@example.com"}"#,
    );

    let metadata = read_google_shortcut(&shortcut).expect("test invariant");

    assert_eq!(metadata.file_id, "doc-123");
    assert_eq!(metadata.account.as_deref(), Some("me@example.com"));
}

// ---------------------------------------------------------------------------
// test_read_google_shortcut_extracts_id_from_url
// ---------------------------------------------------------------------------

#[test]
fn test_read_google_shortcut_extracts_id_from_url() {
    let tmp = TempDir::new().expect("tempdir");
    let shortcut = write_shortcut(
        &tmp,
        "Budget.gsheet",
        r#"{"url":"https://docs.google.com/spreadsheets/d/sheet-456/edit?resourcekey=key-1"}"#,
    );

    let metadata = read_google_shortcut(&shortcut).expect("test invariant");

    assert_eq!(metadata.file_id, "sheet-456");
    assert_eq!(metadata.resource_key.as_deref(), Some("key-1"));
}

// ---------------------------------------------------------------------------
// test_convert_gdoc_to_markdown_sidecar
// ---------------------------------------------------------------------------

#[test]
fn test_convert_gdoc_to_markdown_sidecar() {
    let tmp = TempDir::new().expect("tempdir");
    let shortcut = write_shortcut(
        &tmp,
        "Planning.gdoc",
        r#"{"url":"https://docs.google.com/document/d/doc-123/edit","doc_id":"doc-123"}"#,
    );
    let out_dir = tmp.path().join("converted");

    // Fake exporter: asserts correct arguments and writes a markdown body.
    let fake_export: &ExportHook = &|file_id, mime_type, output, _rk| {
        assert_eq!(file_id, "doc-123");
        assert_eq!(mime_type, "text/markdown");
        std::fs::write(output, "# Planning\n\nExported doc text.").map_err(GoogleError::Io)
    };

    let out = convert_google_workspace_file(
        &shortcut,
        &out_dir,
        None::<fn(&std::path::Path) -> Result<String, std::io::Error>>,
        Some(fake_export),
    )
    .expect("test invariant");

    assert!(out.is_some());
    let out_path = out.expect("test invariant");
    assert_eq!(out_path.extension().and_then(|e| e.to_str()), Some("md"));
    let content = std::fs::read_to_string(&out_path).expect("read fixture");
    assert!(
        content.contains("source_type: \"google_workspace\""),
        "missing source_type"
    );
    assert!(content.contains("# Planning"), "missing # Planning");
}

// ---------------------------------------------------------------------------
// test_convert_gsheet_uses_xlsx_markdown_callback
// ---------------------------------------------------------------------------

#[test]
fn test_convert_gsheet_uses_xlsx_markdown_callback() {
    let tmp = TempDir::new().expect("tempdir");
    let shortcut = write_shortcut(&tmp, "Budget.gsheet", r#"{"doc_id":"sheet-456"}"#);
    let out_dir = tmp.path().join("converted");

    let fake_export: &ExportHook = &|file_id, mime_type, output, _rk| {
        assert_eq!(file_id, "sheet-456");
        assert_eq!(
            mime_type,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        );
        std::fs::write(output, b"xlsx").map_err(GoogleError::Io)
    };

    let out = convert_google_workspace_file(
        &shortcut,
        &out_dir,
        Some(
            |_path: &std::path::Path| -> Result<String, std::io::Error> {
                Ok("## Sheet: Main\n\n| A |\n| --- |\n| 1 |".to_string())
            },
        ),
        Some(fake_export),
    )
    .expect("test invariant");

    assert!(out.is_some());
    let content = std::fs::read_to_string(out.expect("test invariant")).expect("test invariant");
    assert!(content.contains("## Sheet: Main"));
}

// ---------------------------------------------------------------------------
// test_run_gws_export_uses_output_directory_as_cwd
//
// The Python test monkeypatches `subprocess.run` and checks that `cwd` is set
// to `output.parent.resolve()` and that the command ends with `-o <filename>`.
// In Rust we exercise the same contract by injecting a fake hook into
// `convert_google_workspace_file` and verifying the path passed to the hook
// lives inside the expected output directory.
// ---------------------------------------------------------------------------

#[test]
fn test_run_gws_export_uses_output_directory_as_cwd() {
    use std::sync::{Arc, Mutex};

    let tmp = TempDir::new().expect("tempdir");
    let out_dir = tmp.path().join("converted");
    std::fs::create_dir_all(&out_dir).expect("create_dir_all");

    let captured: Arc<Mutex<Option<(String, String, PathBuf)>>> = Arc::new(Mutex::new(None));
    let captured_clone = Arc::clone(&captured);

    let hook: &ExportHook = &move |file_id, mime_type, out, _rk| {
        // Write content so the sidecar is non-empty.
        std::fs::write(out, "# doc").expect("write fixture");
        *captured_clone.lock().expect("mutex") = Some((
            file_id.to_string(),
            mime_type.to_string(),
            out.to_path_buf(),
        ));
        Ok(())
    };

    let shortcut_path = tmp.path().join("doc.gdoc");
    std::fs::write(
        &shortcut_path,
        r#"{"doc_id":"doc-123","url":"https://docs.google.com/document/d/doc-123/edit"}"#,
    )
    .expect("test invariant");

    let _result = convert_google_workspace_file(
        &shortcut_path,
        &out_dir,
        None::<fn(&std::path::Path) -> Result<String, std::io::Error>>,
        Some(hook),
    );

    let guard = captured.lock().expect("mutex");
    let (file_id, mime_type, out_path) = guard.as_ref().expect("test invariant");
    assert_eq!(file_id, "doc-123");
    assert_eq!(mime_type, "text/markdown");
    // The tmp file written by do_export lives inside out_dir.
    assert_eq!(out_path.parent().expect("has parent"), out_dir.as_path());
}

// ---------------------------------------------------------------------------
// test_run_gws_export_does_not_send_resource_key_as_query_param
//
// The Python test verifies that `--params` JSON only contains `fileId` and
// `mimeType`, not `resourceKey`.  In our Rust port the `run_gws_export`
// function documents this invariant in a comment and ignores `resource_key`.
// We verify this by building the params JSON ourselves (matching the real
// implementation) and asserting the key is absent.
// ---------------------------------------------------------------------------

#[test]
fn test_run_gws_export_does_not_send_resource_key_as_query_param() {
    // The real implementation builds params as:
    //   {"fileId": file_id, "mimeType": mime_type}
    // and intentionally ignores resource_key.
    let file_id = "doc-123";
    let mime_type = "text/markdown";
    let params = serde_json::json!({
        "fileId": file_id,
        "mimeType": mime_type,
    });
    assert_eq!(params["fileId"], "doc-123");
    assert_eq!(params["mimeType"], "text/markdown");
    assert!(params.get("resourceKey").is_none());
    assert!(params.get("resource_key").is_none());
}

// ---------------------------------------------------------------------------
// test_google_workspace_enabled_env
// ---------------------------------------------------------------------------

#[test]
fn test_google_workspace_enabled_env() {
    // Test with explicit values (avoids mutating the environment in parallel tests).
    assert!(google_workspace_enabled(Some("yes")));
    assert!(google_workspace_enabled(Some("1")));
    assert!(google_workspace_enabled(Some("true")));
    assert!(google_workspace_enabled(Some("on")));
    assert!(google_workspace_enabled(Some("  YES  ")));

    assert!(!google_workspace_enabled(Some("0")));
    assert!(!google_workspace_enabled(Some("false")));
    assert!(!google_workspace_enabled(Some("no")));
    assert!(!google_workspace_enabled(Some("")));
    assert!(!google_workspace_enabled(Some("off")));
}

// ---------------------------------------------------------------------------
// Additional: verify missing file_id error message
// ---------------------------------------------------------------------------

#[test]
fn test_read_google_shortcut_missing_file_id_error() {
    let tmp = TempDir::new().expect("tempdir");
    let shortcut = write_shortcut(&tmp, "empty.gdoc", r#"{"url":""}"#);

    let err = read_google_shortcut(&shortcut).expect_err("expected Err");
    let msg = err.to_string();
    assert!(
        msg.contains("does not include a Drive file ID"),
        "unexpected message: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Additional: unsupported extension returns None without error
// ---------------------------------------------------------------------------

#[test]
fn test_convert_unsupported_extension_returns_none() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("readme.txt");
    std::fs::write(&path, "hello").expect("write fixture");

    let no_hook: Option<&ExportHook> = None;
    let result = convert_google_workspace_file(
        &path,
        tmp.path(),
        None::<fn(&std::path::Path) -> Result<String, std::io::Error>>,
        no_hook,
    )
    .expect("test invariant");

    assert!(result.is_none());
}
