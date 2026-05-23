//! Conversion entry point: read a Google Workspace shortcut and write a
//! Markdown sidecar.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::GoogleError;
use crate::gws::run_gws_export_via;
use crate::shortcut::{GOOGLE_WORKSPACE_EXTENSIONS, ShortcutMetadata, read_google_shortcut};

/// Export a Google Workspace shortcut to a Markdown sidecar file.
///
/// Returns the path to the written Markdown file, or `None` when:
/// - `path` does not have a Google Workspace extension, or
/// - the exported content is empty/whitespace-only.
///
/// The `xlsx_to_markdown` callback is required when `path` has the
/// `.gsheet` extension; it receives the path of a temporary `.xlsx` file
/// and must return its Markdown representation.
///
/// `run_export` is the low-level export hook. Pass `None` to use the real
/// `gws` binary. It is exposed as a parameter so tests can inject a fake
/// without touching the filesystem or spawning processes.
///
/// # Errors
///
/// Returns [`GoogleError`] variants for I/O failures, missing `gws`, failed
/// exports, or a missing `xlsx_to_markdown` callback for `.gsheet` files.
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

/// Compute the sidecar Markdown path for a given shortcut file.
///
/// Uses an 8-character SHA-256 prefix of the absolute path to disambiguate
/// shortcuts with the same stem in different directories.
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

/// Escape a string for safe embedding as a YAML double-quoted scalar value.
fn safe_yaml_str(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\n', '\r'], " ")
}

/// Create a temporary file in `dir` with the given suffix and return its
/// path.
///
/// The file is created empty; the caller is responsible for writing content
/// to and removing it when done.
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
