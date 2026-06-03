//! Parity port of the Kilo Code tests in `graphify-py/tests/test_install.py`.
//!
//! Kilo gets the full native integration (#512): a global skill + `/graphify`
//! command, the always-on `AGENTS.md` rules, and a project-local
//! `.kilo/plugins/graphify.js` plugin registered (as a `file://` URI) in
//! `.kilo/kilo.json` — without rewriting an existing `.kilo/kilo.jsonc`.

#![allow(clippy::expect_used, unsafe_code)]

use std::path::Path;

use graphify_hooks::platform::{agents_install, agents_uninstall, kilo_install, kilo_uninstall};
use serde_json::Value;
use serial_test::serial;
use url::Url;

/// Compute the `file://` URI the installer registers for the plugin: resolve
/// the (existing) parent dir, join the filename, then `from_file_path`.
fn plugin_uri(project_dir: &Path) -> String {
    let plugin = project_dir.join(".kilo/plugins/graphify.js");
    let resolved = plugin
        .parent()
        .expect("parent")
        .canonicalize()
        .expect("canonicalize parent")
        .join("graphify.js");
    Url::from_file_path(&resolved)
        .expect("file uri")
        .to_string()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("read")).expect("parse")
}

fn plugins_of(config: &Value) -> Vec<String> {
    config
        .get("plugin")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn kilo_agents_install_writes_agents_md() {
    let tmp = tempfile::tempdir().expect("tempdir");
    agents_install(tmp.path(), "kilo").expect("install");
    assert!(tmp.path().join("AGENTS.md").exists());
}

#[test]
fn kilo_agents_install_writes_plugin() {
    let tmp = tempfile::tempdir().expect("tempdir");
    agents_install(tmp.path(), "kilo").expect("install");
    let plugin = tmp.path().join(".kilo/plugins/graphify.js");
    assert!(plugin.exists());
    assert!(
        std::fs::read_to_string(&plugin)
            .expect("read")
            .contains("tool.execute.before")
    );
}

#[test]
fn kilo_agents_install_registers_plugin_in_config() {
    let tmp = tempfile::tempdir().expect("tempdir");
    agents_install(tmp.path(), "kilo").expect("install");
    let config_file = tmp.path().join(".kilo/kilo.json");
    assert!(config_file.exists());
    let plugins = plugins_of(&read_json(&config_file));
    assert!(plugins.contains(&plugin_uri(tmp.path())));
}

#[test]
fn kilo_agents_install_merges_existing_config() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_file = tmp.path().join(".kilo/kilo.json");
    std::fs::create_dir_all(config_file.parent().expect("parent")).expect("mkdir");
    std::fs::write(
        &config_file,
        r#"{"model": "anthropic/claude-sonnet", "plugin": []}"#,
    )
    .expect("write");
    agents_install(tmp.path(), "kilo").expect("install");
    let config = read_json(&config_file);
    assert_eq!(
        config["model"],
        Value::String("anthropic/claude-sonnet".into())
    );
    assert!(plugins_of(&config).contains(&plugin_uri(tmp.path())));
}

#[test]
fn kilo_agents_install_preserves_existing_jsonc_config() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let jsonc = tmp.path().join(".kilo/kilo.jsonc");
    std::fs::create_dir_all(jsonc.parent().expect("parent")).expect("mkdir");
    let original = "// user comment\n{\n  // preferred model\n  \"model\": \"anthropic/claude-haiku\",\n  \"plugin\": []\n}\n";
    std::fs::write(&jsonc, original).expect("write");

    agents_install(tmp.path(), "kilo").expect("install");

    // Automated edit goes to kilo.json; the JSONC stays byte-identical.
    let json = read_json(&tmp.path().join(".kilo/kilo.json"));
    assert_eq!(
        json["model"],
        Value::String("anthropic/claude-haiku".into())
    );
    assert!(plugins_of(&json).contains(&plugin_uri(tmp.path())));
    assert_eq!(std::fs::read_to_string(&jsonc).expect("read"), original);
}

#[test]
fn kilo_agents_uninstall_preserves_existing_jsonc_config() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let jsonc = tmp.path().join(".kilo/kilo.jsonc");
    std::fs::create_dir_all(jsonc.parent().expect("parent")).expect("mkdir");
    let original =
        "// user comment\n{\n  \"model\": \"anthropic/claude-haiku\",\n  \"plugin\": []\n}\n";
    std::fs::write(&jsonc, original).expect("write");

    agents_install(tmp.path(), "kilo").expect("install");
    let uri = plugin_uri(tmp.path());
    agents_uninstall(tmp.path(), "kilo").expect("uninstall");

    let json = read_json(&tmp.path().join(".kilo/kilo.json"));
    assert_eq!(std::fs::read_to_string(&jsonc).expect("read"), original);
    assert!(!plugins_of(&json).contains(&uri));
}

#[test]
fn kilo_agents_install_idempotent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    agents_install(tmp.path(), "kilo").expect("install 1");
    agents_install(tmp.path(), "kilo").expect("install 2");
    let content = std::fs::read_to_string(tmp.path().join("AGENTS.md")).expect("read");
    let config = read_json(&tmp.path().join(".kilo/kilo.json"));
    let uri = plugin_uri(tmp.path());
    assert_eq!(content.matches("## graphify").count(), 1);
    assert_eq!(plugins_of(&config).iter().filter(|p| **p == uri).count(), 1);
}

#[test]
#[serial(home_env)]
fn kilo_install_writes_global_and_project_artifacts() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let project = tmp.path().join("project");
    std::fs::create_dir_all(&home).expect("mkdir home");
    std::fs::create_dir_all(&project).expect("mkdir project");
    // SAFETY: test-only HOME override; `#[serial(home_env)]` serialises access.
    unsafe {
        std::env::set_var("HOME", &home);
    }
    let result = kilo_install(&project);
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    result.expect("kilo install");
    assert!(home.join(".config/kilo/skills/graphify/SKILL.md").exists());
    assert!(home.join(".config/kilo/command/graphify.md").exists());
    assert!(project.join("AGENTS.md").exists());
    assert!(project.join(".kilo/plugins/graphify.js").exists());
}

#[test]
#[serial(home_env)]
fn kilo_uninstall_removes_plugin_registration_and_command() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let project = tmp.path().join("project");
    std::fs::create_dir_all(&home).expect("mkdir home");
    std::fs::create_dir_all(&project).expect("mkdir project");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", &home);
    }
    let r = (|| {
        kilo_install(&project)?;
        kilo_uninstall(&project)
    })();
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    r.expect("kilo install+uninstall");
    assert!(!home.join(".config/kilo/command/graphify.md").exists());
    assert!(!home.join(".config/kilo/skills/graphify/SKILL.md").exists());
    assert!(!project.join(".kilo/plugins/graphify.js").exists());
    let config_file = project.join(".kilo/kilo.json");
    if config_file.exists() {
        assert!(
            !plugins_of(&read_json(&config_file))
                .iter()
                .any(|p| p.ends_with("graphify.js"))
        );
    }
}
