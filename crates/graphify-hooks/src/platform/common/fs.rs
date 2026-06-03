//! Filesystem helpers shared across platform installers.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::HooksError;

/// Idempotently update or append a graphify-owned section in shared markdown
/// files.
///
/// Mirrors Python `_replace_or_append_section` byte-for-byte.
///
/// If `marker` is not in `content`, appends `new_section` (with a blank-line
/// separator if existing content is non-empty). If `marker` IS present,
/// replaces the existing section in place (from the first line containing
/// `marker` to the line before the next `## ` heading or EOF).
#[must_use]
pub fn replace_or_append_section(content: &str, marker: &str, new_section: &str) -> String {
    if !content.contains(marker) {
        if content.trim().is_empty() {
            return new_section.trim_start().to_string();
        }
        return format!("{}\n\n{}", content.trim_end(), new_section.trim_start());
    }

    let lines: Vec<&str> = content.split('\n').collect();
    let Some(start) = lines.iter().position(|l| l.contains(marker)) else {
        return format!("{}\n\n{}", content.trim_end(), new_section.trim_start());
    };

    let end = lines[start + 1..]
        .iter()
        .position(|l| l.starts_with("## "))
        .map_or(lines.len(), |rel| start + 1 + rel);

    let head = lines[..start].join("\n").trim_end().to_string();
    let tail = lines[end..].join("\n").trim_start().to_string();
    let section = new_section.trim().to_string();

    let mut parts: Vec<String> = Vec::new();
    if !head.is_empty() {
        parts.push(head);
    }
    parts.push(section);
    if !tail.is_empty() {
        parts.push(tail);
    }
    let mut out = parts.join("\n\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Remove a `## graphify` section from markdown content.
///
/// Equivalent to Python: `re.sub(r"\n*## graphify\n.*?(?=\n## |\Z)", "", content, flags=re.DOTALL)`.
/// The `regex` crate does not support lookahead, so this is implemented with
/// pure string manipulation.
pub(in crate::platform) fn remove_graphify_section(content: &str) -> String {
    const MARKER: &str = "## graphify";
    let Some(marker_byte) = content.find(MARKER) else {
        return content.trim_end().to_string();
    };
    let section_start = content[..marker_byte]
        .rfind(|c: char| c != '\n')
        .map_or(0, |i| i + 1);

    let after_marker = marker_byte + MARKER.len();
    let section_end = content[after_marker..]
        .find("\n## ")
        .map_or(content.len(), |rel| after_marker + rel);

    let head = &content[..section_start];
    let tail = &content[section_end..];
    format!("{head}{tail}").trim_end().to_string()
}

/// Write `content` to `path` atomically by writing to a sibling `.tmp` file
/// then renaming it.
///
/// # Errors
///
/// Returns `HooksError::Io` on write or rename failure. The tmp file is
/// cleaned up on rename failure.
pub(in crate::platform) fn write_atomic(path: &Path, content: &str) -> Result<(), HooksError> {
    let tmp = path.with_extension(path.extension().map_or_else(
        || "tmp".to_string(),
        |e| format!("{}.tmp", e.to_string_lossy()),
    ));
    fs::write(&tmp, content.as_bytes())?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    Ok(())
}

/// Install a skill markdown file to `dst`, creating parent directories.
///
/// Returns the destination path on success.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failure.
pub(in crate::platform) fn install_skill(
    skill_content: &str,
    dst: &Path,
) -> Result<PathBuf, HooksError> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    write_atomic(dst, skill_content)?;
    Ok(dst.to_path_buf())
}

/// Remove a skill file, its sibling `.graphify_version` file, and up to
/// three empty ancestor directories (the immediate parent and two
/// grandparents).
///
/// Silently ignores every step that fails — uninstall is best-effort.
pub(in crate::platform) fn remove_skill(skill_dst: &Path) {
    if skill_dst.exists() {
        let _ = fs::remove_file(skill_dst);
    }
    let version_file = skill_dst
        .parent()
        .map(|p| p.join(".graphify_version"))
        .unwrap_or_default();
    if version_file.exists() {
        let _ = fs::remove_file(&version_file);
    }
    let Some(p0) = skill_dst.parent() else {
        return;
    };
    let Some(p1) = p0.parent() else {
        return;
    };
    let Some(p2) = p1.parent() else {
        return;
    };
    for dir in &[p0, p1, p2] {
        if fs::remove_dir(dir).is_err() {
            break;
        }
    }
}

/// Resolve the graphify executable path.
///
/// Shells out to `which graphify` and returns the resolved path if
/// successful; otherwise returns the bare string `"graphify"` to let the
/// caller's shell perform path resolution.
#[must_use]
pub fn resolve_graphify_exe() -> String {
    let cmd = if cfg!(windows) { "where" } else { "which" };
    if let Ok(output) = std::process::Command::new(cmd).arg("graphify").output()
        && output.status.success()
    {
        // `where` on Windows may return multiple lines; take the first.
        let stdout = String::from_utf8_lossy(&output.stdout);
        let path = stdout.lines().next().unwrap_or("").trim().to_string();
        if !path.is_empty() {
            return path;
        }
    }
    "graphify".to_string()
}

/// Read a JSON file from `path`, returning an empty JSON object on missing
/// or unparseable files.
///
/// Used to load existing platform settings files in a permissive manner so
/// installers can layer their changes onto whatever the user already has.
pub(in crate::platform) fn read_json_or_empty(path: &Path) -> Value {
    if !path.exists() {
        return Value::Object(serde_json::Map::new());
    }
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()))
}

/// Serialise `value` to `path` with 2-space indentation, creating parent
/// directories as needed.
///
/// # Errors
///
/// Returns `HooksError::Json` on serialisation failure or `HooksError::Io`
/// on filesystem failure.
pub(in crate::platform) fn write_json(path: &Path, value: &Value) -> Result<(), HooksError> {
    let json = serde_json::to_string_pretty(value).map_err(|e| HooksError::Json(e.to_string()))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, json.as_bytes())?;
    Ok(())
}

/// Return the user's home directory as a `PathBuf`.
///
/// Falls back to `"."` if neither `HOME` nor `USERPROFILE` is set — this
/// should not happen in normal operation but avoids panics in degenerate
/// test environments.
pub(in crate::platform) fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_or_else(|_| PathBuf::from("."), PathBuf::from)
}

/// The `CLAUDE_CONFIG_DIR` override as a `PathBuf`, or `None` when unset.
///
/// An *empty* value is treated as unset: `std::env::var` returns `Ok("")` for
/// `CLAUDE_CONFIG_DIR=`, and joining an empty `PathBuf` would silently produce a
/// stray *relative* `skills/graphify/SKILL.md` (or `CLAUDE.md`) that the install
/// path writes but the uninstall path can never find. Falling back to the home
/// directory in that case keeps install and uninstall pointed at the same file.
pub(in crate::platform) fn claude_config_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}
