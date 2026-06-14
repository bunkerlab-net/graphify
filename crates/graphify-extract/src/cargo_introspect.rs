//! Cargo manifest introspection for workspace-internal crate dependencies.
//!
//! Mirrors `graphify-py/graphify/cargo_introspect.py`. Walks a Cargo workspace
//! (or single package), emitting one `crate:<name>` node per package and a
//! `crate_depends_on` edge for each dependency that resolves to another
//! workspace-internal crate. Registry-only dependencies (e.g. `serde`) are not
//! crates in the graph and are skipped.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// Confidence tier stamped on every emitted edge.
const CONFIDENCE_EXTRACTED: &str = "EXTRACTED";

/// Error raised while reading or parsing a Cargo manifest.
#[derive(Debug, thiserror::Error)]
pub enum CargoIntrospectError {
    /// The manifest could not be read from disk.
    #[error("cannot read {path}: {source}")]
    Io {
        /// Path of the manifest that failed to read.
        path: String,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The manifest was not valid TOML.
    #[error("TOML parse error in {path}: {source}")]
    Toml {
        /// Path of the manifest that failed to parse.
        path: String,
        /// Underlying TOML parse error.
        source: toml::de::Error,
    },
}

/// Crate nodes and internal dependency edges discovered from Cargo manifests.
#[derive(Debug, Default)]
pub struct CargoIntrospection {
    /// One `crate:<name>` node per workspace-internal package.
    pub nodes: Vec<Value>,
    /// One `crate_depends_on` edge per internal dependency.
    pub edges: Vec<Value>,
}

/// Read and parse a single `Cargo.toml`.
fn load_toml(path: &Path) -> Result<toml::Value, CargoIntrospectError> {
    let text = std::fs::read_to_string(path).map_err(|source| CargoIntrospectError::Io {
        path: path.to_string_lossy().into_owned(),
        source,
    })?;
    toml::from_str::<toml::Value>(&text).map_err(|source| CargoIntrospectError::Toml {
        path: path.to_string_lossy().into_owned(),
        source,
    })
}

/// Resolve a path relative to `root` as a forward-slash string, mirroring
/// Python's `Path.relative_to(root).as_posix()`.
fn relative_posix(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Collect the manifests of every package in the workspace: the root package (if
/// the root manifest declares one) plus each member matched by the workspace
/// `members` glob patterns. Mirrors `_member_manifest_paths`.
fn member_manifest_paths(root: &Path, root_data: &toml::Value) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if root_data
        .get("package")
        .and_then(toml::Value::as_table)
        .is_some()
    {
        paths.push(root.join("Cargo.toml"));
    }

    let Some(members) = root_data
        .get("workspace")
        .and_then(toml::Value::as_table)
        .and_then(|w| w.get("members"))
        .and_then(toml::Value::as_array)
    else {
        return paths;
    };

    for pattern in members {
        let Some(pattern) = pattern.as_str() else {
            continue;
        };
        let glob_pat = root.join(pattern).to_string_lossy().into_owned();
        let Ok(matches) = glob::glob(&glob_pat) else {
            continue;
        };
        let mut members: Vec<PathBuf> = matches.filter_map(Result::ok).collect();
        members.sort();
        for member in members {
            let manifest = member.join("Cargo.toml");
            if manifest.is_file() && !paths.contains(&manifest) {
                paths.push(manifest);
            }
        }
    }
    paths
}

/// Return crate nodes and internal dependency edges from the Cargo manifests
/// rooted at `root`. Mirrors `introspect_cargo`.
///
/// # Errors
///
/// Returns [`CargoIntrospectError`] if the root manifest (or any discovered
/// member manifest) cannot be read or is not valid TOML.
pub fn introspect_cargo(root: &Path) -> Result<CargoIntrospection, CargoIntrospectError> {
    let root_path = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let root_manifest = root_path.join("Cargo.toml");
    let root_data = load_toml(&root_manifest)?;

    let manifests = member_manifest_paths(&root_path, &root_data);

    // name → (crate_id, manifest, data). Sorted by name on emission, matching
    // the reference's `sorted(crates.items())`.
    let mut crates: Vec<(String, String, PathBuf, toml::Value)> = Vec::new();
    for manifest in manifests {
        let data = if manifest == root_manifest {
            root_data.clone()
        } else {
            load_toml(&manifest)?
        };
        let Some(package) = data.get("package").and_then(toml::Value::as_table) else {
            continue;
        };
        let Some(name) = package.get("name").and_then(toml::Value::as_str) else {
            continue;
        };
        let name = name.to_string();
        // Last package wins for a duplicate name, mirroring the Python dict.
        if let Some(existing) = crates.iter_mut().find(|(n, ..)| *n == name) {
            *existing = (name.clone(), format!("crate:{name}"), manifest, data);
        } else {
            crates.push((name.clone(), format!("crate:{name}"), manifest, data));
        }
    }
    crates.sort_by(|a, b| a.0.cmp(&b.0));

    let nodes: Vec<Value> = crates
        .iter()
        .map(|(name, crate_id, manifest, _data)| {
            json!({
                "id": crate_id,
                "label": name,
                "source_file": relative_posix(manifest, &root_path),
                "source_location": "L1",
            })
        })
        .collect();

    let known: std::collections::HashSet<&str> =
        crates.iter().map(|(name, ..)| name.as_str()).collect();

    let mut edges: Vec<Value> = Vec::new();
    for (_name, source_id, manifest, data) in &crates {
        let Some(dependencies) = data.get("dependencies").and_then(toml::Value::as_table) else {
            continue;
        };
        let source_file = relative_posix(manifest, &root_path);
        let mut dep_names: Vec<&String> = dependencies.keys().collect();
        dep_names.sort();
        for dep_name in dep_names {
            if known.contains(dep_name.as_str()) {
                edges.push(json!({
                    "source": source_id,
                    "target": format!("crate:{dep_name}"),
                    "relation": "crate_depends_on",
                    "context": "cargo_dependency",
                    "weight": 1.0,
                    "confidence": CONFIDENCE_EXTRACTED,
                    "source_file": source_file,
                    "source_location": "L1",
                }));
            }
        }
    }

    Ok(CargoIntrospection { nodes, edges })
}
