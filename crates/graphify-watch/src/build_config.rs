//! Persisted build options that survive into later rebuilds (#1886).
//!
//! The initial `extract` scan honours `--exclude`, but `update`/`watch`/hook
//! rebuilds re-run detection and would silently re-include excluded paths unless
//! the patterns are persisted. They are stored in a sidecar beside the graph so
//! any rebuild driver can re-apply them.

use std::path::Path;

use serde_json::Value;

/// Sidecar filename storing build options under the output directory.
pub const BUILD_CONFIG_FILENAME: &str = ".graphify_build.json";

/// Per-process sequence for unique build-config temp-file names (atomic write).
static BUILD_CONFIG_TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Persist build options (currently `--exclude` patterns) under `out_dir`.
///
/// Best-effort and non-clobbering: with `None`/empty excludes it leaves any
/// existing file untouched, so a plain rebuild never erases patterns a prior
/// extract recorded.
pub fn write_build_config(out_dir: &Path, excludes: Option<&[String]>) {
    let Some(excludes) = excludes.filter(|e| !e.is_empty()) else {
        return;
    };
    if std::fs::create_dir_all(out_dir).is_err() {
        return;
    }
    let payload = serde_json::json!({ "excludes": excludes });
    let Ok(text) = serde_json::to_string(&payload) else {
        return;
    };
    // Write atomically: a torn write would leave a corrupt sidecar that
    // `read_build_excludes` silently discards, dropping the persisted excludes.
    // Stage a per-process/per-call unique sibling temp then rename over the
    // destination (replace-capable on every platform, incl. Windows via
    // `MoveFileExW`); clean up on failure. The unique suffix means two writers
    // to the same dir never clobber each other's staging file.
    let dest = out_dir.join(BUILD_CONFIG_FILENAME);
    let seq = BUILD_CONFIG_TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut tmp = dest.clone().into_os_string();
    tmp.push(format!(".{}.{seq}.tmp", std::process::id()));
    let tmp = std::path::PathBuf::from(tmp);
    if std::fs::write(&tmp, text.as_bytes())
        .and_then(|()| std::fs::rename(&tmp, &dest))
        .is_err()
    {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Return the persisted `--exclude` patterns for this graph, or an empty list
/// when the sidecar is missing or invalid.
#[must_use]
pub fn read_build_excludes(out_dir: &Path) -> Vec<String> {
    let path = out_dir.join(BUILD_CONFIG_FILENAME);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    map.get("excludes")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
