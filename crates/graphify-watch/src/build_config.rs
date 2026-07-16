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
    if let Ok(text) = serde_json::to_string(&payload) {
        let _ = std::fs::write(out_dir.join(BUILD_CONFIG_FILENAME), text);
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
