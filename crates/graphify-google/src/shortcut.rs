//! Parsing of `.gdoc` / `.gsheet` / `.gslides` shortcut JSON files.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use url::Url;

use crate::error::GoogleError;

/// File extensions recognised as Google Workspace shortcut files.
pub const GOOGLE_WORKSPACE_EXTENSIONS: &[&str] = &[".gdoc", ".gsheet", ".gslides"];

/// Metadata extracted from a shortcut file.
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

/// Return `true` when Google Workspace shortcut export is enabled.
///
/// Checks `value` if supplied, otherwise reads `GRAPHIFY_GOOGLE_WORKSPACE`
/// from the environment. The values `"1"`, `"true"`, `"yes"`, and `"on"`
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

/// Read a `.gdoc` / `.gsheet` / `.gslides` shortcut and return its export
/// metadata.
///
/// # Errors
///
/// - [`GoogleError::ReadShortcut`] if the file cannot be read or parsed as
///   JSON.
/// - [`GoogleError::MissingFileId`] if no Drive file ID can be located in
///   the shortcut.
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

// SAFETY: known-valid literal regex pattern.
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
    for (k, v) in parsed.query_pairs() {
        if k == "id" {
            return Some(v.into_owned());
        }
    }
    FILE_ID_RE
        .captures(parsed.path())
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Extract a Drive file ID from explicit data fields, then the URL, then
/// `resource_id` with `type:id` format.
///
/// Checks `doc_id`, `file_id`, `fileId`, `id` fields in order (matching the
/// Python reference) before falling back to URL parsing.
fn extract_file_id(url: &str, data: &Value) -> Option<String> {
    for key in &["doc_id", "file_id", "fileId", "id"] {
        if let Some(v) = data.get(*key).and_then(Value::as_str)
            && !v.is_empty()
        {
            return Some(v.to_string());
        }
    }
    if let Some(id) = extract_file_id_from_url(url) {
        return Some(id);
    }
    if let Some(rid) = data.get("resource_id").and_then(Value::as_str)
        && let Some((_left, right)) = rid.split_once(':')
        && !right.is_empty()
    {
        return Some(right.to_string());
    }
    None
}

/// Extract an optional Drive resource key from data fields or URL query
/// parameters.
///
/// Resource keys are needed for shared Drive items protected by link
/// access.
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
