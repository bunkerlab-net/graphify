//! Source-file path relativization helper.
//!
//! Extracted from `rebuild.rs` so the path-rewriting logic that converts
//! absolute extraction paths to project-relative ones lives in isolation.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Convert absolute `source_file` paths in a payload to `root`-relative paths.
///
/// Iterates over `nodes`, `edges`, and `hyperedges` in `payload`. Any item
/// whose `source_file` is an absolute path that can be made relative to `root`
/// is rewritten in-place. When `scope` is `Some`, an absolute path whose
/// resolved form lies OUTSIDE `scope` is left untouched — so a preserved node
/// from a sibling project (identity outside the watched root) is not
/// mis-relativised against this run's root (#8d8d2b8). Items with relative or
/// unresolvable paths are left unchanged.
///
/// Ports `_relativize_source_files` (with the `scope` keyword).
pub fn relativize_source_files(payload: &mut Value, root: &Path, scope: Option<&Path>) {
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
            // `scope` gate: skip a resolved path outside the watched subtree.
            if let Some(scope) = scope
                && resolved.strip_prefix(scope).is_err()
            {
                continue;
            }
            if let Ok(rel) = resolved.strip_prefix(root) {
                // `.as_posix()` parity: emit forward slashes so graph.json paths
                // are stable across platforms.
                map.insert(
                    "source_file".to_string(),
                    Value::String(rel.to_string_lossy().replace('\\', "/")),
                );
            }
        }
    }
}
