//! Install/uninstall graphify for Kilo Code.
//!
//! Kilo gets the full native integration (#512): a global skill + `/graphify`
//! command, the always-on `AGENTS.md` rules, and a project-local
//! `.kilo/plugins/graphify.js` plugin (mirroring the `OpenCode`
//! `tool.execute.before` hook). JSONC config is handled non-destructively — an existing
//! `.kilo/kilo.jsonc` is read but never rewritten; automated plugin
//! registration goes to `.kilo/kilo.json`, preserving user comments.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};
use url::Url;

use super::common::{COMMAND_KILO_MD, KILO_PLUGIN_JS, dirs_home, install_skill, remove_skill};
use crate::HooksError;

const KILO_PLUGIN_PATH: &str = ".kilo/plugins/graphify.js";
const KILO_CONFIG_JSON: &str = "kilo.json";
const KILO_CONFIG_JSONC: &str = "kilo.jsonc";

/// Global Kilo skill destination (`~/.config/kilo/skills/graphify/SKILL.md`).
fn kilo_skill_dst() -> PathBuf {
    dirs_home()
        .join(".config")
        .join("kilo")
        .join("skills")
        .join("graphify")
        .join("SKILL.md")
}

/// Global Kilo `/graphify` command destination
/// (`~/.config/kilo/command/graphify.md`).
fn kilo_command_dst() -> PathBuf {
    dirs_home()
        .join(".config")
        .join("kilo")
        .join("command")
        .join("graphify.md")
}

/// Trailing-comma cleanup applied after JSONC comment stripping.
#[allow(clippy::expect_used)] // literal pattern; build cannot panic
static TRAILING_COMMA_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r",(\s*[}\]])").expect("static trailing-comma regex"));

/// Remove JSONC-style comments while leaving string content intact, then drop
/// trailing commas so the result parses as strict JSON. Mirrors Python's
/// `_strip_json_comments`.
#[must_use]
fn strip_json_comments(raw: &str) -> String {
    let mut result = String::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let mut in_string = false;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;
    let mut i = 0;
    let n = bytes.len();
    while i < n {
        let ch = bytes[i];
        let nxt = if i + 1 < n { bytes[i + 1] } else { 0 };

        if line_comment {
            if ch == b'\n' {
                line_comment = false;
                result.push('\n');
            }
            i += 1;
            continue;
        }
        if block_comment {
            if ch == b'*' && nxt == b'/' {
                block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if in_string {
            result.push(ch as char);
            if escaped {
                escaped = false;
            } else if ch == b'\\' {
                escaped = true;
            } else if ch == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if ch == b'/' && nxt == b'/' {
            line_comment = true;
            i += 2;
            continue;
        }
        if ch == b'/' && nxt == b'*' {
            block_comment = true;
            i += 2;
            continue;
        }
        result.push(ch as char);
        if ch == b'"' {
            in_string = true;
        }
        i += 1;
    }
    TRAILING_COMMA_RE.replace_all(&result, "$1").into_owned()
}

/// Load a Kilo config file as a JSON object, stripping JSONC comments for a
/// `.jsonc` file. Returns an empty object on any read/parse error or a
/// non-object top-level value. Mirrors Python's `_load_json_like`.
#[must_use]
fn load_json_like(config_file: &Path) -> Value {
    if !config_file.exists() {
        return Value::Object(Map::new());
    }
    let Ok(raw) = fs::read_to_string(config_file) else {
        return Value::Object(Map::new());
    };
    let parsed_input = if config_file.extension().and_then(|e| e.to_str()) == Some("jsonc") {
        strip_json_comments(&raw)
    } else {
        raw
    };
    match serde_json::from_str::<Value>(&parsed_input) {
        Ok(v @ Value::Object(_)) => v,
        _ => Value::Object(Map::new()),
    }
}

/// Resolve the Kilo config file to *read* from: prefer `.kilo/kilo.json`, then
/// `.kilo/kilo.jsonc`, else default to `.kilo/kilo.json`.
fn kilo_config_path(project_dir: &Path) -> PathBuf {
    let kilo_dir = project_dir.join(".kilo");
    let json_path = kilo_dir.join(KILO_CONFIG_JSON);
    if json_path.exists() {
        return json_path;
    }
    let jsonc_path = kilo_dir.join(KILO_CONFIG_JSONC);
    if jsonc_path.exists() {
        return jsonc_path;
    }
    json_path
}

/// Config file to *write* automated edits to — always `.kilo/kilo.json`, so an
/// existing `.kilo/kilo.jsonc` (with the user's comments) is never rewritten.
fn kilo_config_write_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".kilo").join(KILO_CONFIG_JSON)
}

/// `file://` URI of the plugin path, resolving the (existing) parent directory
/// for symlinks. Mirrors Python's `plugin_file.resolve().as_uri()`, which works
/// even after the plugin file itself is deleted (uninstall) because the parent
/// directory still exists.
fn plugin_uri(plugin_file: &Path) -> Option<String> {
    let parent = plugin_file.parent()?;
    let name = plugin_file.file_name()?;
    let resolved = parent.canonicalize().ok()?.join(name);
    Url::from_file_path(&resolved).ok().map(|u| u.to_string())
}

/// Write the `graphify.js` plugin and register it in `.kilo/kilo.json` without
/// rewriting an existing `.kilo/kilo.jsonc`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn install_kilo_plugin(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let plugin_file = project_dir.join(KILO_PLUGIN_PATH);
    if let Some(parent) = plugin_file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&plugin_file, KILO_PLUGIN_JS.as_bytes())?;
    msgs.push(format!(
        "  {KILO_PLUGIN_PATH}  ->  tool.execute.before hook written"
    ));

    let config_file = kilo_config_path(project_dir);
    let write_config_file = kilo_config_write_path(project_dir);
    if let Some(parent) = write_config_file.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut config = load_json_like(&config_file);
    let entry = plugin_uri(&plugin_file)
        .ok_or_else(|| HooksError::Io(std::io::Error::other("cannot resolve plugin path")))?;

    let plugins = config
        .as_object_mut()
        .map(|o| {
            o.entry("plugin")
                .or_insert_with(|| Value::Array(Vec::new()))
        })
        .ok_or_else(|| HooksError::Json("config is not an object".to_string()))?;
    if !matches!(plugins, Value::Array(_)) {
        *plugins = Value::Array(Vec::new());
    }

    let already = plugins
        .as_array()
        .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some(entry.as_str())));
    if already {
        msgs.push(format!(
            "  {}  ->  plugin already registered (no change)",
            display_rel(&config_file, project_dir)
        ));
    } else {
        if let Value::Array(arr) = plugins {
            arr.push(Value::String(entry));
        }
        let serialized =
            serde_json::to_string_pretty(&config).map_err(|e| HooksError::Json(e.to_string()))?;
        fs::write(&write_config_file, serialized.as_bytes())?;
        msgs.push(format!(
            "  {}  ->  plugin registered",
            display_rel(&write_config_file, project_dir)
        ));
    }

    Ok(msgs.join("\n"))
}

/// Remove the `graphify.js` plugin and deregister it without rewriting an
/// existing `.kilo/kilo.jsonc`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn uninstall_kilo_plugin(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let plugin_file = project_dir.join(KILO_PLUGIN_PATH);
    if plugin_file.exists() {
        fs::remove_file(&plugin_file)?;
        msgs.push(format!("  {KILO_PLUGIN_PATH}  ->  removed"));
    }

    let config_file = kilo_config_path(project_dir);
    if !config_file.exists() {
        return Ok(msgs.join("\n"));
    }
    let write_config_file = kilo_config_write_path(project_dir);
    let mut config = load_json_like(&config_file);
    let Some(entry) = plugin_uri(&plugin_file) else {
        return Ok(msgs.join("\n"));
    };

    let plugins = config.pointer_mut("/plugin").and_then(Value::as_array_mut);
    if let Some(arr) = plugins {
        let before = arr.len();
        arr.retain(|v| v.as_str() != Some(entry.as_str()));
        if arr.len() != before {
            if arr.is_empty()
                && let Some(obj) = config.as_object_mut()
            {
                obj.remove("plugin");
            }
            if let Some(parent) = write_config_file.parent() {
                fs::create_dir_all(parent)?;
            }
            let serialized = serde_json::to_string_pretty(&config)
                .map_err(|e| HooksError::Json(e.to_string()))?;
            fs::write(&write_config_file, serialized.as_bytes())?;
            msgs.push(format!(
                "  {}  ->  plugin deregistered",
                display_rel(&write_config_file, project_dir)
            ));
        }
    }

    Ok(msgs.join("\n"))
}

/// Display `path` relative to `project_dir` when possible (matches Python's
/// `relative_to(project_dir)` in the install messages), else the full path.
fn display_rel(path: &Path, project_dir: &Path) -> String {
    path.strip_prefix(project_dir)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Install the native Kilo skill (`~/.config/kilo/skills/graphify/SKILL.md`) and
/// `/graphify` command file (`~/.config/kilo/command/graphify.md`).
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn install_kilo_skill_and_command() -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();
    let skill_dst = kilo_skill_dst();
    install_skill(super::common::SKILL_MD, &skill_dst)?;
    msgs.push(format!("  skill installed  ->  {}", skill_dst.display()));

    let command_dst = kilo_command_dst();
    if let Some(parent) = command_dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&command_dst, COMMAND_KILO_MD.as_bytes())?;
    msgs.push(format!("  command installed ->  {}", command_dst.display()));
    Ok(msgs.join("\n"))
}

/// Remove the global Kilo command + skill files (and prune empty dirs).
/// Mirrors Python's `_kilo_uninstall_global`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
fn kilo_uninstall_global() -> Result<Vec<String>, HooksError> {
    let mut removed: Vec<String> = Vec::new();

    let command_dst = kilo_command_dst();
    if command_dst.exists() {
        fs::remove_file(&command_dst)?;
        removed.push(format!("command removed: {}", command_dst.display()));
    }
    if let Some(parent) = command_dst.parent() {
        let _ = fs::remove_dir(parent); // best-effort: only if now empty
    }

    let skill_dst = kilo_skill_dst();
    if skill_dst.exists() {
        removed.push(format!("skill removed: {}", skill_dst.display()));
    }
    // remove_skill prunes SKILL.md + version stamp + up to three empty ancestors
    // (graphify/, skills/, kilo/), matching Python's rmdir walk.
    remove_skill(&skill_dst);

    Ok(removed)
}

/// Install native Kilo skill + command globally and always-on project wiring
/// (`AGENTS.md` + `.kilo` plugin) locally. Mirrors Python's `_kilo_install`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn kilo_install(project_dir: &Path) -> Result<String, HooksError> {
    let skill_and_command = install_kilo_skill_and_command()?;
    let agents = super::agents::agents_install(project_dir, "kilo")?;
    Ok([skill_and_command, agents].join("\n"))
}

/// Remove Kilo always-on project wiring and global skill/command files.
/// Mirrors Python's `_kilo_uninstall`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn kilo_uninstall(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = vec![super::agents::agents_uninstall(project_dir, "kilo")?];
    msgs.extend(kilo_uninstall_global()?);
    Ok(msgs.join("\n"))
}
