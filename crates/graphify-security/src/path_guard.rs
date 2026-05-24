//! Path containment check that prevents reads outside the
//! `graphify-out/` base directory.

use std::path::{Component, Path, PathBuf};

use crate::error::SecurityError;

/// Resolve `path` and verify it stays inside `base`. `base` defaults to the
/// `graphify-out/` directory: first walking up from the hint path to find
/// one, then falling back to `<cwd>/graphify-out`.
///
/// On success returns the fully resolved absolute path.
///
/// # Errors
///
/// - [`SecurityError::BaseMissing`] if the base directory does not exist.
/// - [`SecurityError::PathEscape`] if the path resolves outside the base.
/// - [`SecurityError::GraphFileMissing`] if the file itself does not exist.
pub fn validate_graph_path<P: AsRef<Path>>(
    path: P,
    base: Option<&Path>,
) -> Result<PathBuf, SecurityError> {
    let path = path.as_ref();

    let base_path = if let Some(b) = base {
        b.to_path_buf()
    } else {
        let hint = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mut found: Option<PathBuf> = None;
        let mut cur = Some(hint.as_path());
        while let Some(c) = cur {
            if c.file_name().is_some_and(|n| n == "graphify-out") {
                found = Some(c.to_path_buf());
                break;
            }
            cur = c.parent();
        }
        found.unwrap_or_else(|| {
            std::env::current_dir().map_or_else(
                |_| PathBuf::from("graphify-out"),
                |cwd| cwd.join("graphify-out"),
            )
        })
    };

    // `canonicalize` already fails when the path doesn't exist, so the
    // separate `exists()` check that used to follow is redundant.
    let Ok(base_resolved) = base_path.canonicalize() else {
        return Err(SecurityError::BaseMissing(base_path));
    };

    let resolved = resolve_logical(path);

    if resolved.strip_prefix(&base_resolved).is_err() {
        return Err(SecurityError::PathEscape {
            path: path.to_path_buf(),
            base: base_resolved,
        });
    }

    if !resolved.exists() {
        return Err(SecurityError::GraphFileMissing(resolved));
    }

    Ok(resolved)
}

/// Resolve `path` to an absolute path by collapsing `.` and `..` components
/// without requiring the full path to exist, then canonicalise the deepest
/// existing prefix so symlinks are followed for that portion.
///
/// Mirrors Python's `Path.resolve()` semantics for nonexistent leaves.
fn resolve_logical(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut out = PathBuf::new();
    for comp in absolute.components() {
        match comp {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(std::path::MAIN_SEPARATOR.to_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(p) => out.push(p),
        }
    }
    let mut existing: Option<PathBuf> = None;
    let mut tail: Vec<&std::ffi::OsStr> = Vec::new();
    let components: Vec<_> = out.components().collect();
    for (idx, _) in components.iter().enumerate().rev() {
        let candidate: PathBuf = components[..=idx].iter().collect();
        if candidate.exists() {
            existing = Some(candidate);
            for c in &components[idx + 1..] {
                if let Component::Normal(name) = c {
                    tail.push(name);
                }
            }
            break;
        }
    }
    if let Some(ex) = existing
        && let Ok(canon) = ex.canonicalize()
    {
        let mut result = canon;
        for t in tail {
            result.push(t);
        }
        return result;
    }
    out
}
