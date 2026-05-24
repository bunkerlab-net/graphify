//! pnpm workspace package resolution for JS/TS imports.
//!
//! Ports the `_find_workspace_root`, `_workspace_globs`,
//! `_load_workspace_packages`, `_package_entry_candidates`,
//! `_resolve_workspace_import`, and `_WORKSPACE_PACKAGE_CACHE` helpers from
//! `graphify-py/graphify/extract.py`. Used by the JS import resolver to
//! turn a bare specifier like `@scope/pkg` into a concrete file path
//! inside a pnpm-managed monorepo.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use indexmap::IndexMap;
use serde_json::Value;

/// Process-wide cache: workspace-root path → `package_name → package_dir`.
///
/// Wrapped in a `Mutex` because parallel file extraction may resolve
/// imports from multiple threads. Mirrors the Python module-level
/// `_WORKSPACE_PACKAGE_CACHE` dict (single-threaded there, but the
/// semantic intent is the same: load each workspace once).
#[allow(clippy::expect_used)] // static lock; cannot panic at construction time
static WORKSPACE_PACKAGE_CACHE: Mutex<Option<HashMap<PathBuf, IndexMap<String, PathBuf>>>> =
    Mutex::new(None);

/// Find the closest ancestor of `start_dir` that contains a
/// `pnpm-workspace.yaml`. Returns `None` when none of the ancestors do.
#[must_use]
pub fn find_workspace_root(start_dir: &Path) -> Option<PathBuf> {
    let current = start_dir
        .canonicalize()
        .unwrap_or_else(|_| start_dir.to_path_buf());
    let mut candidate: &Path = &current;
    loop {
        if candidate.join("pnpm-workspace.yaml").is_file() {
            return Some(candidate.to_path_buf());
        }
        candidate = candidate.parent()?;
    }
}

/// Parse the `packages:` list out of a `pnpm-workspace.yaml`.
///
/// Hand-rolled (avoids the YAML dependency) — only handles the common
/// shape `packages:\n  - 'glob/*'\n  - 'apps/*'`. Negation entries
/// (`!exclude`) are skipped.
#[must_use]
pub fn workspace_globs(workspace_file: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(workspace_file) else {
        return Vec::new();
    };
    let mut globs: Vec<String> = Vec::new();
    let mut in_packages = false;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("packages:") {
            in_packages = true;
            continue;
        }
        if in_packages && line.starts_with('-') {
            let value = line[1..].trim().trim_matches(|c| c == '\'' || c == '"');
            if !value.is_empty() && !value.starts_with('!') {
                globs.push(value.to_string());
            }
            continue;
        }
        // Stop when we see a non-indented line after the packages key.
        if in_packages && !raw_line.starts_with(' ') && !raw_line.starts_with('\t') {
            break;
        }
    }
    globs
}

/// Resolve a `packages:` glob to the matching `package.json` directories,
/// then build the `package_name → package_dir` index.
///
/// Cached per workspace root.
#[must_use]
pub fn load_workspace_packages(start_dir: &Path) -> IndexMap<String, PathBuf> {
    let Some(root) = find_workspace_root(start_dir) else {
        return IndexMap::new();
    };
    if let Ok(guard) = WORKSPACE_PACKAGE_CACHE.lock()
        && let Some(map) = guard.as_ref()
        && let Some(packages) = map.get(&root)
    {
        return packages.clone();
    }

    let mut packages: IndexMap<String, PathBuf> = IndexMap::new();
    let workspace_file = root.join("pnpm-workspace.yaml");
    for pattern in workspace_globs(&workspace_file) {
        for package_dir in glob_workspace_pattern(&root, &pattern) {
            let manifest = package_dir.join("package.json");
            if !manifest.is_file() {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&manifest) else {
                continue;
            };
            let Ok(data) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if let Some(name) = data.get("name").and_then(Value::as_str)
                && !name.is_empty()
            {
                packages.insert(name.to_string(), package_dir);
            }
        }
    }

    if let Ok(mut guard) = WORKSPACE_PACKAGE_CACHE.lock() {
        let entry = guard.get_or_insert_with(HashMap::new);
        entry.insert(root, packages.clone());
    }
    packages
}

/// Expand a single `packages:` glob pattern against `root`. Supports the
/// pnpm-typical `dir/*`, `dir/**`, and bare-directory entries — the
/// minimum needed to handle real-world monorepos. Patterns we don't
/// understand resolve to the empty list.
fn glob_workspace_pattern(root: &Path, pattern: &str) -> Vec<PathBuf> {
    let pattern = pattern.trim_end_matches('/');
    if let Some(prefix) = pattern.strip_suffix("/**") {
        // recursive glob — walk every subdirectory
        let base = root.join(prefix);
        let mut out: Vec<PathBuf> = Vec::new();
        walk_subdirs(&base, &mut out);
        return out;
    }
    if let Some(prefix) = pattern.strip_suffix("/*") {
        // single-level glob — list immediate subdirectories
        let base = root.join(prefix);
        let Ok(entries) = std::fs::read_dir(&base) else {
            return Vec::new();
        };
        return entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect();
    }
    // Treat as a literal directory.
    let p = root.join(pattern);
    if p.is_dir() { vec![p] } else { Vec::new() }
}

fn walk_subdirs(base: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            out.push(path.clone());
            walk_subdirs(&path, out);
        }
    }
}

/// Pick the most likely entry-point file for a workspace package.
///
/// When `subpath` is non-empty (i.e. the import was `pkg/foo/bar`),
/// return `package_dir/subpath` directly. Otherwise read `package.json`
/// and prefer (in order) `exports["."]` (string or object), then
/// `svelte` / `module` / `main` / `types`, then `src/index` / `index`
/// fallbacks. Mirrors `_package_entry_candidates` in Python.
#[must_use]
pub fn package_entry_candidates(package_dir: &Path, subpath: &str) -> Vec<PathBuf> {
    let manifest_path = package_dir.join("package.json");
    let manifest_data: Value = std::fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .unwrap_or(Value::Null);

    if !subpath.is_empty() {
        return vec![package_dir.join(subpath)];
    }

    if let Some(exports) = manifest_data.get("exports") {
        if let Some(s) = exports.as_str() {
            return vec![package_dir.join(s)];
        }
        if let Some(obj) = exports.as_object()
            && let Some(dot) = obj.get(".")
        {
            if let Some(s) = dot.as_str() {
                return vec![package_dir.join(s)];
            }
            if let Some(dot_obj) = dot.as_object() {
                for key in ["types", "import", "default", "svelte"] {
                    if let Some(s) = dot_obj.get(key).and_then(Value::as_str) {
                        return vec![package_dir.join(s)];
                    }
                }
            }
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    for key in ["svelte", "module", "main", "types"] {
        if let Some(s) = manifest_data.get(key).and_then(Value::as_str) {
            candidates.push(package_dir.join(s));
        }
    }
    candidates.push(package_dir.join("src/index"));
    candidates.push(package_dir.join("index"));
    candidates
}

/// Resolve a bare specifier (`@scope/pkg` or `@scope/pkg/subpath`) by
/// matching it against the workspace package map and probing the
/// package's entry-point candidates with the standard file-extension
/// resolver. Returns the first candidate that exists on disk.
#[must_use]
pub fn resolve_workspace_import(raw: &str, start_dir: &Path) -> Option<PathBuf> {
    let packages = load_workspace_packages(start_dir);
    for (package_name, package_dir) in &packages {
        let subpath = if raw == package_name {
            String::new()
        } else {
            let with_slash = format!("{package_name}/");
            if let Some(rest) = raw.strip_prefix(&with_slash) {
                rest.to_string()
            } else {
                continue;
            }
        };
        for candidate in package_entry_candidates(package_dir, &subpath) {
            let resolved = crate::tsconfig::resolve_js_module_path(&candidate);
            if resolved.is_file() {
                return Some(resolved);
            }
        }
    }
    None
}
