//! Source-file path relativization helper.
//!
//! Extracted from `rebuild.rs` so the path-rewriting logic that converts
//! absolute extraction paths to project-relative ones lives in isolation.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Convert absolute `source_file` paths in a payload to project-relative paths.
///
/// Iterates over `nodes`, `edges`, and `hyperedges` in `payload`.  Any item
/// whose `source_file` field is an absolute path that can be made relative to
/// `root` is rewritten in-place.  Items with relative or unresolvable paths
/// are left unchanged.
///
/// Ports `_relativize_source_files` from `watch.py:131-143`.
pub fn relativize_source_files(payload: &mut Value, root: &Path) {
    let Some(obj) = payload.as_object_mut() else {
        return;
    };

    for bucket in &["nodes", "edges", "hyperedges"] {
        let Some(Value::Array(items)) = obj.get_mut(*bucket) else {
            continue;
        };
        for item in items.iter_mut() {
            let Some(map) = item.as_object_mut() else {
                continue;
            };
            let Some(Value::String(source)) = map.get("source_file") else {
                continue;
            };
            let source_path = PathBuf::from(source);
            if !source_path.is_absolute() {
                continue;
            }
            // resolve() then relative_to() — mirrors Python's
            // source_path.resolve().relative_to(root)
            let resolved = source_path
                .canonicalize()
                .unwrap_or_else(|_| source_path.clone());
            if let Ok(rel) = resolved.strip_prefix(root) {
                map.insert(
                    "source_file".to_string(),
                    Value::String(rel.to_string_lossy().into_owned()),
                );
            }
        }
    }
}
