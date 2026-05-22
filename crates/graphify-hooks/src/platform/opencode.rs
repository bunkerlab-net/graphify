//! Install/uninstall graphify for `OpenCode`.
//!
//! `OpenCode` uses a JS plugin (`tool.execute.before`) rather than a JSON hook,
//! and registers itself in `opencode.json`. The plugin path and config structure
//! are unique enough to warrant a dedicated file.

use std::fs;
use std::path::Path;

use serde_json::Value;

use super::common::{OPENCODE_PLUGIN_JS, read_json_or_empty, write_json};
use crate::HooksError;

const OPENCODE_PLUGIN_PATH: &str = ".opencode/plugins/graphify.js";
const OPENCODE_CONFIG_PATH: &str = ".opencode/opencode.json";

/// Write `graphify.js` plugin and register it in `opencode.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn install_opencode_plugin(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let plugin_file = project_dir.join(OPENCODE_PLUGIN_PATH);
    if let Some(parent) = plugin_file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&plugin_file, OPENCODE_PLUGIN_JS.as_bytes())?;
    msgs.push(format!(
        "  {OPENCODE_PLUGIN_PATH}  ->  tool.execute.before hook written"
    ));

    let config_file = project_dir.join(OPENCODE_CONFIG_PATH);
    let mut config = read_json_or_empty(&config_file);

    let plugins = config
        .as_object_mut()
        .map(|o| {
            o.entry("plugin")
                .or_insert_with(|| Value::Array(Vec::new()))
        })
        .ok_or_else(|| HooksError::Json("config is not an object".to_string()))?;

    let entry = ".opencode/plugins/graphify.js";
    let already = if let Value::Array(arr) = &plugins {
        arr.iter().any(|v| v.as_str() == Some(entry))
    } else {
        false
    };

    if already {
        msgs.push(format!(
            "  {OPENCODE_CONFIG_PATH}  ->  plugin already registered (no change)"
        ));
    } else {
        if let Value::Array(arr) = plugins {
            arr.push(Value::String(entry.to_string()));
        }
        write_json(&config_file, &config)?;
        msgs.push(format!("  {OPENCODE_CONFIG_PATH}  ->  plugin registered"));
    }

    Ok(msgs.join("\n"))
}

/// Remove `graphify.js` plugin and deregister from `opencode.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn uninstall_opencode_plugin(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let plugin_file = project_dir.join(OPENCODE_PLUGIN_PATH);
    if plugin_file.exists() {
        fs::remove_file(&plugin_file)?;
        msgs.push(format!("  {OPENCODE_PLUGIN_PATH}  ->  removed"));
    }

    let config_file = project_dir.join(OPENCODE_CONFIG_PATH);
    if !config_file.exists() {
        return Ok(msgs.join("\n"));
    }
    let mut config = read_json_or_empty(&config_file);
    let entry = ".opencode/plugins/graphify.js";
    let plugins = config.pointer_mut("/plugin").and_then(Value::as_array_mut);
    if let Some(arr) = plugins {
        let before = arr.len();
        arr.retain(|v| v.as_str() != Some(entry));
        if arr.len() != before {
            if arr.is_empty()
                && let Some(obj) = config.as_object_mut()
            {
                obj.remove("plugin");
            }
            write_json(&config_file, &config)?;
            msgs.push(format!("  {OPENCODE_CONFIG_PATH}  ->  plugin deregistered"));
        }
    }

    Ok(msgs.join("\n"))
}
