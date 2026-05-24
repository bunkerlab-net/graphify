//! Wrapper around the external `gws` CLI used to export Google Workspace
//! files to Markdown / plain text / xlsx.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::GoogleError;

/// Run the real `gws` export subcommand.
///
/// Exposed publicly so callers that build their own export pipeline can
/// invoke it directly.
///
/// # Errors
///
/// - [`GoogleError::GwsMissing`] if `gws` is not on `PATH`.
/// - [`GoogleError::GwsFailed`] if the process exits with a non-zero status.
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

/// Run the export, using `hook` if supplied, otherwise spawning the real
/// `gws` process. The hook seam lets tests stub the export without spawning
/// subprocesses.
pub(crate) fn run_gws_export_via<F>(
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

    let exe = which_gws().ok_or(GoogleError::GwsMissing)?;

    let output_resolved = output.canonicalize().unwrap_or_else(|_| {
        let parent = output.parent().map_or_else(
            || PathBuf::from("."),
            |p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()),
        );
        // When `output` is something like `..` it has no file_name; in that
        // case fall back to the resolved parent so we never construct a path
        // that ends in an empty component.
        output
            .file_name()
            .map_or_else(|| parent.clone(), |name| parent.join(name))
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

/// Locate the `gws` binary on `PATH`.
///
/// Returns `None` if the binary cannot be found.
fn which_gws() -> Option<PathBuf> {
    let exe_name = if cfg!(windows) { "gws.exe" } else { "gws" };
    std::env::var_os("PATH").and_then(|path_var| {
        std::env::split_paths(&path_var).find_map(|dir| {
            let candidate = dir.join(exe_name);
            if candidate.is_file() {
                Some(candidate)
            } else {
                None
            }
        })
    })
}
