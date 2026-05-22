//! Google Workspace shortcut export support.
//!
//! Google Drive for desktop stores native Docs, Sheets, and Slides as small
//! JSON shortcut files (`.gdoc`, `.gsheet`, `.gslides`). Those files are
//! pointers, not document content. This module exports them to Markdown
//! sidecars via the `gws` CLI so Graphify can extract their actual contents.
//!
//! Ports `graphify-py/graphify/google_workspace.py`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;

/// File extensions that are Google Workspace shortcut files.
pub const GOOGLE_WORKSPACE_EXTENSIONS: &[&str] = &[".gdoc", ".gsheet", ".gslides"];

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by Google Workspace shortcut operations.
#[derive(Debug, Error)]
pub enum GoogleError {
    /// The shortcut file could not be read or parsed.
    #[error("could not read Google Workspace shortcut {path}: {source}")]
    ReadShortcut {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The shortcut file does not contain a Drive file ID.
    #[error("Google Workspace shortcut {path} does not include a Drive file ID")]
    MissingFileId { path: PathBuf },

    /// The `gws` binary is missing from PATH.
    #[error(
        "gws is required for Google Workspace export. Install it from \
        https://github.com/googleworkspace/cli and run `gws auth login -s drive`."
    )]
    GwsMissing,

    /// `gws export` exited with a non-zero return code.
    #[error("gws export failed for {file_id}: {stderr}")]
    GwsFailed { file_id: String, stderr: String },

    /// A filesystem I/O error occurred.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// The `xlsx_to_markdown` callback is required for `.gsheet` but was not
    /// provided.
    #[error(
        "Google Sheets export requires the office extra: \
        pip install graphifyy[office,google]"
    )]
    XlsxCallbackMissing,
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Metadata extracted from a `.gdoc` / `.gsheet` / `.gslides` shortcut file.
#[derive(Debug, Clone)]
pub struct ShortcutMetadata {
    /// Drive file ID (e.g. `"1BxiMVs0XRA5nFMdKvBdBZjgmUUqptlbs74OgVE2upms"`).
    pub file_id: String,
    /// Original URL stored in the shortcut, if any.
    pub url: Option<String>,
    /// Drive resource key for shared-drive files, if any.
    pub resource_key: Option<String>,
    /// Google account email address stored in the shortcut, if any.
    pub account: Option<String>,
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Return `true` when Google Workspace shortcut export is enabled.
///
/// Checks `value` if supplied, otherwise reads `GRAPHIFY_GOOGLE_WORKSPACE`
/// from the environment.  The values `"1"`, `"true"`, `"yes"`, and `"on"`
/// (case-insensitive, leading/trailing whitespace stripped) are truthy.
#[must_use]
pub fn google_workspace_enabled(value: Option<&str>) -> bool {
    let raw = value.map_or_else(
        || std::env::var("GRAPHIFY_GOOGLE_WORKSPACE").unwrap_or_default(),
        str::to_string,
    );
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Read a `.gdoc` / `.gsheet` / `.gslides` shortcut and return export
/// metadata.
///
/// # Errors
///
/// Returns [`GoogleError::ReadShortcut`] if the file cannot be read or parsed
/// as JSON, or [`GoogleError::MissingFileId`] if no Drive file ID can be
/// found.
pub fn read_google_shortcut(path: &Path) -> Result<ShortcutMetadata, GoogleError> {
    let text = std::fs::read_to_string(path).map_err(|e| GoogleError::ReadShortcut {
        path: path.to_path_buf(),
        source: Box::new(e),
    })?;
    let data: Value = serde_json::from_str(&text).map_err(|e| GoogleError::ReadShortcut {
        path: path.to_path_buf(),
        source: Box::new(e),
    })?;

    let url = data
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let file_id = extract_file_id(&url, &data).ok_or_else(|| GoogleError::MissingFileId {
        path: path.to_path_buf(),
    })?;

    let resource_key = extract_resource_key(&url, &data);
    let account = data
        .get("email")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    Ok(ShortcutMetadata {
        file_id,
        url: if url.is_empty() { None } else { Some(url) },
        resource_key,
        account,
    })
}

/// Export a Google Workspace shortcut to a Markdown sidecar file.
///
/// Returns the path to the written Markdown file, or `None` when:
/// - `path` does not have a Google Workspace extension, or
/// - the exported content is empty/whitespace-only.
///
/// The `xlsx_to_markdown` callback is required when `path` has the `.gsheet`
/// extension; it receives the path of a temporary `.xlsx` file and must return
/// its Markdown representation.
///
/// `run_export` is the low-level export hook.  Pass `None` to use the real
/// `gws` binary.  It is exposed as a parameter so tests can inject a fake
/// without touching the filesystem or spawning processes.
///
/// # Errors
///
/// Returns [`GoogleError`] for I/O failures, missing `gws`, failed exports, or
/// a missing `xlsx_to_markdown` callback for `.gsheet` files.
pub fn convert_google_workspace_file<F, X, E>(
    path: &Path,
    out_dir: &Path,
    xlsx_to_markdown: Option<X>,
    run_export: Option<&F>,
) -> Result<Option<PathBuf>, GoogleError>
where
    F: Fn(&str, &str, &Path, Option<&str>) -> Result<(), GoogleError> + ?Sized,
    X: Fn(&Path) -> Result<String, E>,
    E: std::error::Error + Send + Sync + 'static,
{
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default()
        .to_ascii_lowercase();

    if !GOOGLE_WORKSPACE_EXTENSIONS.contains(&ext.as_str()) {
        return Ok(None);
    }

    let shortcut = read_google_shortcut(path)?;
    std::fs::create_dir_all(out_dir)?;
    let out_path = sidecar_path(path, out_dir);

    // Closure that runs an export and returns the resulting file body.
    let do_export = |mime: &str, tmp_suffix: &str| -> Result<String, GoogleError> {
        let tmp_path = tmp_file(out_dir, tmp_suffix)?;
        let result = run_gws_export_via(
            &shortcut.file_id,
            mime,
            &tmp_path,
            shortcut.resource_key.as_deref(),
            run_export,
        );
        let body = std::fs::read_to_string(&tmp_path).unwrap_or_default();
        let _ = std::fs::remove_file(&tmp_path);
        result?;
        Ok(body)
    };

    match ext.as_str() {
        ".gdoc" => {
            let body = do_export("text/markdown", ".md")?;
            if body.trim().is_empty() {
                return Ok(None);
            }
            let content = with_frontmatter(path, &shortcut, &body, "text/markdown");
            std::fs::write(&out_path, content)?;
            Ok(Some(out_path))
        }
        ".gslides" => {
            let body = do_export("text/plain", ".txt")?;
            if body.trim().is_empty() {
                return Ok(None);
            }
            let content = with_frontmatter(path, &shortcut, &body, "text/plain");
            std::fs::write(&out_path, content)?;
            Ok(Some(out_path))
        }
        ".gsheet" => {
            let cb = xlsx_to_markdown.ok_or(GoogleError::XlsxCallbackMissing)?;
            let tmp_path = tmp_file(out_dir, ".xlsx")?;
            let export_result = run_gws_export_via(
                &shortcut.file_id,
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                &tmp_path,
                shortcut.resource_key.as_deref(),
                run_export,
            );
            if let Err(e) = export_result {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(e);
            }
            let body = cb(&tmp_path).map_err(|e| GoogleError::ReadShortcut {
                path: tmp_path.clone(),
                source: Box::new(e),
            })?;
            let _ = std::fs::remove_file(&tmp_path);
            if body.trim().is_empty() {
                return Ok(None);
            }
            let content = with_frontmatter(
                path,
                &shortcut,
                &body,
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            );
            std::fs::write(&out_path, content)?;
            Ok(Some(out_path))
        }
        _ => Ok(None),
    }
}

/// Run the real `gws` export subcommand.
///
/// Exposed publicly so callers that build their own export pipeline can invoke
/// it directly.
///
/// # Errors
///
/// Returns [`GoogleError::GwsMissing`] if `gws` is not on `PATH`, or
/// [`GoogleError::GwsFailed`] if the process exits with a non-zero status.
pub fn run_gws_export(
    file_id: &str,
    mime_type: &str,
    output: &Path,
    resource_key: Option<&str>,
) -> Result<(), GoogleError> {
    run_gws_export_via(
        file_id,
        mime_type,
        output,
        resource_key,
        None::<&fn(&str, &str, &Path, Option<&str>) -> Result<(), GoogleError>>,
    )
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Escape a string for safe embedding as a YAML double-quoted scalar value.
fn safe_yaml_str(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\n', '\r'], " ")
}

// SAFETY: This is a known-valid literal regex pattern.
#[allow(clippy::unwrap_used)]
static FILE_ID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"/(?:document|spreadsheets|presentation|file)/d/([^/?#]+)").unwrap()
});

/// Extract a Drive file ID from common Google Docs/Drive URL shapes.
fn extract_file_id_from_url(url: &str) -> Option<String> {
    if url.is_empty() {
        return None;
    }
    let parsed = Url::parse(url).ok()?;
    // Try `?id=…` query parameter first.
    for (k, v) in parsed.query_pairs() {
        if k == "id" {
            return Some(v.into_owned());
        }
    }
    // Fall back to path pattern.
    FILE_ID_RE
        .captures(parsed.path())
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Extract a Drive file ID from explicit data fields or from the URL.
///
/// Checks `doc_id`, `file_id`, `fileId`, `id` fields first (in that order),
/// then falls back to URL parsing and finally to `resource_id` with `type:id` format.
fn extract_file_id(url: &str, data: &Value) -> Option<String> {
    // Explicit field names take priority, matching Python order.
    for key in &["doc_id", "file_id", "fileId", "id"] {
        if let Some(v) = data.get(*key).and_then(Value::as_str)
            && !v.is_empty()
        {
            return Some(v.to_string());
        }
    }
    // Try URL extraction.
    if let Some(id) = extract_file_id_from_url(url) {
        return Some(id);
    }
    // Last resort: `resource_id` field with `type:id` format.
    if let Some(rid) = data.get("resource_id").and_then(Value::as_str)
        && let Some((_left, right)) = rid.split_once(':')
        && !right.is_empty()
    {
        return Some(right.to_string());
    }
    None
}

/// Extract an optional Drive resource key from data fields or URL query parameters.
///
/// Resource keys are needed for shared Drive items protected by link access.
fn extract_resource_key(url: &str, data: &Value) -> Option<String> {
    for key in &["resource_key", "resourceKey"] {
        if let Some(v) = data.get(*key).and_then(Value::as_str)
            && !v.is_empty()
        {
            return Some(v.to_string());
        }
    }
    if url.is_empty() {
        return None;
    }
    let parsed = Url::parse(url).ok()?;
    for (k, v) in parsed.query_pairs() {
        if k == "resourcekey" {
            return Some(v.into_owned());
        }
    }
    None
}

/// Compute the sidecar Markdown path for a given shortcut file.
fn sidecar_path(path: &Path, out_dir: &Path) -> PathBuf {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let digest = hex::encode(hasher.finalize());
    let name_hash = &digest[..8];
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    out_dir.join(format!("{stem}_{name_hash}.md"))
}

/// Build the YAML front-matter + body string for a sidecar file.
fn with_frontmatter(
    path: &Path,
    shortcut: &ShortcutMetadata,
    body: &str,
    exported_mime_type: &str,
) -> String {
    let source_url = shortcut.url.as_deref().unwrap_or("");
    let account_line = if let Some(ref account) = shortcut.account {
        let mut hasher = Sha256::new();
        hasher.update(account.as_bytes());
        let digest = hex::encode(hasher.finalize());
        let account_hash = &digest[..12];
        format!("google_account_hash: \"{account_hash}\"\n")
    } else {
        String::new()
    };

    format!(
        "---\n\
        source_file: \"{source_file}\"\n\
        source_type: \"google_workspace\"\n\
        google_file_id: \"{file_id}\"\n\
        google_export_mime_type: \"{mime}\"\n\
        source_url: \"{url}\"\n\
        {account_line}\
        ---\n\n\
        <!-- converted from Google Workspace shortcut: {name} -->\n\n\
        {body}\n",
        source_file = safe_yaml_str(&path.to_string_lossy()),
        file_id = safe_yaml_str(&shortcut.file_id),
        mime = safe_yaml_str(exported_mime_type),
        url = safe_yaml_str(source_url),
        account_line = account_line,
        name = path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
        body = body.trim(),
    )
}

/// Create a temporary file in `dir` with the given suffix and return its path.
///
/// The file is created empty; the caller is responsible for writing content
/// and removing it when done.
fn tmp_file(dir: &Path, suffix: &str) -> Result<PathBuf, GoogleError> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| {
            // Intentional truncation: we only need a unique-enough suffix.
            #[allow(clippy::cast_possible_truncation)]
            let v: u64 = d.as_nanos() as u64;
            v
        });
    let name = format!("graphify_gws_{nanos:016x}{suffix}");
    let path = dir.join(name);
    std::fs::write(&path, b"")?;
    Ok(path)
}

/// Internal: run the export, using `hook` if supplied, otherwise spawning
/// the real `gws` process.
fn run_gws_export_via<F>(
    file_id: &str,
    mime_type: &str,
    output: &Path,
    resource_key: Option<&str>,
    hook: Option<&F>,
) -> Result<(), GoogleError>
where
    F: Fn(&str, &str, &Path, Option<&str>) -> Result<(), GoogleError> + ?Sized,
{
    if let Some(f) = hook {
        return f(file_id, mime_type, output, resource_key);
    }

    // Locate the real `gws` binary.
    let exe = which_gws().ok_or(GoogleError::GwsMissing)?;

    let output_resolved = output.canonicalize().unwrap_or_else(|_| {
        // Output may not exist yet; resolve its parent instead.
        let parent = output.parent().map_or_else(
            || PathBuf::from("."),
            |p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()),
        );
        parent.join(output.file_name().unwrap_or_default())
    });

    let cwd = output_resolved
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    std::fs::create_dir_all(&cwd)?;

    let params = serde_json::json!({
        "fileId": file_id,
        "mimeType": mime_type,
    });
    // Drive resource keys must be sent via X-Goog-Drive-Resource-Keys headers;
    // the current `gws export` command has no custom-header flag so we do not
    // pass resourceKey as a query parameter (matches Python behaviour).
    let _ = resource_key;

    let out_name = output_resolved
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("out.md")
        .to_string();

    let timeout_secs: u64 = std::env::var("GRAPHIFY_GOOGLE_WORKSPACE_TIMEOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);

    let result = Command::new(&exe)
        .args([
            "drive",
            "files",
            "export",
            "--params",
            &serde_json::to_string(&params).unwrap_or_default(),
            "-o",
            &out_name,
        ])
        .current_dir(&cwd)
        .env(
            "GRAPHIFY_GOOGLE_WORKSPACE_TIMEOUT",
            timeout_secs.to_string(),
        )
        .output()
        .map_err(GoogleError::Io)?;

    if !result.status.success() {
        let stderr_raw = if result.stderr.is_empty() {
            result.stdout.clone()
        } else {
            result.stderr.clone()
        };
        let mut stderr = String::from_utf8_lossy(&stderr_raw).trim().to_string();
        if stderr.len() > 1200 {
            stderr.truncate(1200);
            stderr.push_str("...");
        }
        return Err(GoogleError::GwsFailed {
            file_id: file_id.to_string(),
            stderr,
        });
    }

    Ok(())
}

/// Locate the `gws` binary on PATH.  Returns `None` if not found.
fn which_gws() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path_var| {
        std::env::split_paths(&path_var).find_map(|dir| {
            let candidate = dir.join("gws");
            if candidate.is_file() {
                Some(candidate)
            } else {
                None
            }
        })
    })
}
