//! Parity tests for the MCP config extractor.
//!
//! 1:1 port of `graphify-py/tests/test_mcp_ingest.py`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use graphify_extract::{
    FileResult, MCP_CONFIG_FILENAMES, Node, extract, extract_mcp_config, is_mcp_config_path,
};
use serde_json::{Value, json};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn mcp_kind(n: &Node) -> Option<&str> {
    n.metadata.as_ref()?.get("mcp_kind")?.as_str()
}

fn label_by_kind<'a>(r: &'a FileResult, kind: &str) -> Vec<&'a str> {
    r.nodes
        .iter()
        .filter(|n| mcp_kind(n) == Some(kind))
        .map(|n| n.label.as_str())
        .collect()
}

fn set_by_kind(r: &FileResult, kind: &str) -> HashSet<String> {
    label_by_kind(r, kind)
        .into_iter()
        .map(String::from)
        .collect()
}

fn relations(r: &FileResult) -> HashSet<String> {
    r.edges.iter().map(|e| e.relation.clone()).collect()
}

fn write_json(dir: &Path, name: &str, value: &Value) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, serde_json::to_string(value).expect("serialize")).expect("write");
    p
}

// ── Detection ────────────────────────────────────────────────────────────────

#[test]
fn is_mcp_config_path_recognises_known_filenames() {
    for name in [
        ".mcp.json",
        "claude_desktop_config.json",
        "mcp.json",
        "mcp_servers.json",
    ] {
        assert!(
            is_mcp_config_path(&PathBuf::from(format!("/some/dir/{name}"))),
            "{name}"
        );
    }
}

#[test]
fn is_mcp_config_path_rejects_generic_json() {
    assert!(!is_mcp_config_path(Path::new("package.json")));
    assert!(!is_mcp_config_path(Path::new("config.json")));
    assert!(!is_mcp_config_path(Path::new("tsconfig.json")));
}

#[test]
fn recognised_filenames_set_is_stable() {
    assert!(MCP_CONFIG_FILENAMES.contains(".mcp.json"));
    assert_eq!(MCP_CONFIG_FILENAMES.len(), 4);
}

// ── Happy path with the bundled fixture ──────────────────────────────────────

#[test]
fn fixture_parses_without_error() {
    let r = extract_mcp_config(&fixtures().join("sample.mcp.json"));
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(!r.nodes.is_empty());
    assert!(!r.edges.is_empty());
}

#[test]
fn fixture_emits_every_server() {
    let r = extract_mcp_config(&fixtures().join("sample.mcp.json"));
    let servers = set_by_kind(&r, "mcp_server");
    let expected: HashSet<String> = ["filesystem", "fetch", "github", "time"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(servers, expected);
}

#[test]
fn fixture_emits_commands_as_global_nodes() {
    let r = extract_mcp_config(&fixtures().join("sample.mcp.json"));
    let commands = set_by_kind(&r, "mcp_command");
    let expected: HashSet<String> = ["npx", "uvx"].into_iter().map(String::from).collect();
    assert_eq!(commands, expected);
}

#[test]
fn fixture_emits_npm_packages() {
    let r = extract_mcp_config(&fixtures().join("sample.mcp.json"));
    let packages = set_by_kind(&r, "mcp_package");
    assert!(packages.contains("@modelcontextprotocol/server-filesystem"));
    assert!(packages.contains("@modelcontextprotocol/server-github"));
}

#[test]
fn fixture_emits_python_packages() {
    let r = extract_mcp_config(&fixtures().join("sample.mcp.json"));
    let packages = set_by_kind(&r, "mcp_package");
    assert!(packages.contains("mcp-server-fetch"));
    assert!(packages.contains("mcp-server-time"));
}

#[test]
fn fixture_strips_version_from_npm_package() {
    let r = extract_mcp_config(&fixtures().join("sample.mcp.json"));
    let packages = set_by_kind(&r, "mcp_package");
    assert!(packages.contains("@modelcontextprotocol/server-github"));
    assert!(!packages.contains("@modelcontextprotocol/server-github@0.6.2"));
}

#[test]
fn fixture_emits_env_var_names() {
    let r = extract_mcp_config(&fixtures().join("sample.mcp.json"));
    let env_vars = set_by_kind(&r, "env_var");
    assert!(env_vars.contains("FILESYSTEM_ROOT"));
    assert!(env_vars.contains("GITHUB_PERSONAL_ACCESS_TOKEN"));
}

#[test]
fn env_var_values_never_appear_anywhere() {
    let secret = "ghp_PLACEHOLDER_NOT_A_REAL_TOKEN";
    let r = extract_mcp_config(&fixtures().join("sample.mcp.json"));
    for n in &r.nodes {
        assert!(!n.label.contains(secret));
        if let Some(md) = n.metadata.as_ref() {
            for v in md.values() {
                assert!(!v.to_string().contains(secret));
            }
        }
    }
    for e in &r.edges {
        let serialized = serde_json::to_string(e).expect("serialize edge");
        assert!(!serialized.contains(secret));
    }
}

#[test]
fn filesystem_path_not_persisted_as_node() {
    let r = extract_mcp_config(&fixtures().join("sample.mcp.json"));
    for n in &r.nodes {
        assert!(!n.label.contains("/tmp/workspace"));
    }
}

#[test]
fn fixture_relations_include_contains_references_requires_env() {
    let r = extract_mcp_config(&fixtures().join("sample.mcp.json"));
    let rels = relations(&r);
    assert!(rels.contains("contains"));
    assert!(rels.contains("references"));
    assert!(rels.contains("requires_env"));
}

#[test]
fn no_dangling_edges() {
    let r = extract_mcp_config(&fixtures().join("sample.mcp.json"));
    let node_ids: HashSet<&str> = r.nodes.iter().map(|n| n.id.as_str()).collect();
    for e in &r.edges {
        assert!(node_ids.contains(e.source.as_str()));
        assert!(node_ids.contains(e.target.as_str()));
    }
}

#[test]
fn every_edge_has_confidence_score() {
    let r = extract_mcp_config(&fixtures().join("sample.mcp.json"));
    for e in &r.edges {
        assert_eq!(e.confidence, "EXTRACTED");
        assert_eq!(e.confidence_score, Some(1.0));
        assert!((e.weight - 1.0).abs() < f64::EPSILON);
    }
}

// ── Cross-config emergent edges (global node IDs) ────────────────────────────

#[test]
fn same_command_collapses_to_one_node_across_configs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_a = write_json(
        tmp.path(),
        ".mcp.json",
        &json!({"mcpServers": {"a": {"command": "npx", "args": ["@scope/server-a"]}}}),
    );
    let subdir = tmp.path().join("subdir");
    std::fs::create_dir_all(&subdir).expect("mkdir");
    let config_b = write_json(
        &subdir,
        "claude_desktop_config.json",
        &json!({"mcpServers": {"b": {"command": "npx", "args": ["@scope/server-b"]}}}),
    );
    let r_a = extract_mcp_config(&config_a);
    let r_b = extract_mcp_config(&config_b);
    let cmd_a = r_a
        .nodes
        .iter()
        .find(|n| mcp_kind(n) == Some("mcp_command"))
        .unwrap();
    let cmd_b = r_b
        .nodes
        .iter()
        .find(|n| mcp_kind(n) == Some("mcp_command"))
        .unwrap();
    assert_eq!(cmd_a.id, cmd_b.id);
}

#[test]
fn same_env_var_collapses_to_one_node_across_configs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let a = write_json(
        tmp.path(),
        ".mcp.json",
        &json!({"mcpServers": {"x": {"command": "npx", "args": ["@scope/x"], "env": {"OPENAI_API_KEY": "v1"}}}}),
    );
    let sub = tmp.path().join("sub");
    std::fs::create_dir_all(&sub).expect("mkdir");
    let b = write_json(
        &sub,
        "claude_desktop_config.json",
        &json!({"mcpServers": {"y": {"command": "uvx", "args": ["mcp-server-y"], "env": {"OPENAI_API_KEY": "v2"}}}}),
    );
    let r_a = extract_mcp_config(&a);
    let r_b = extract_mcp_config(&b);
    let env_a = r_a
        .nodes
        .iter()
        .find(|n| mcp_kind(n) == Some("env_var"))
        .unwrap();
    let env_b = r_b
        .nodes
        .iter()
        .find(|n| mcp_kind(n) == Some("env_var"))
        .unwrap();
    assert_eq!(env_a.id, env_b.id);
}

#[test]
fn same_server_name_in_different_dirs_does_not_collide() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let proj_a = tmp.path().join("proj_a");
    let proj_b = tmp.path().join("proj_b");
    std::fs::create_dir_all(&proj_a).expect("mkdir");
    std::fs::create_dir_all(&proj_b).expect("mkdir");
    let a = write_json(
        &proj_a,
        ".mcp.json",
        &json!({"mcpServers": {"filesystem": {"command": "npx", "args": ["@scope/a"]}}}),
    );
    let b = write_json(
        &proj_b,
        ".mcp.json",
        &json!({"mcpServers": {"filesystem": {"command": "npx", "args": ["@scope/b"]}}}),
    );
    let r_a = extract_mcp_config(&a);
    let r_b = extract_mcp_config(&b);
    let srv_a = r_a
        .nodes
        .iter()
        .find(|n| mcp_kind(n) == Some("mcp_server"))
        .unwrap();
    let srv_b = r_b
        .nodes
        .iter()
        .find(|n| mcp_kind(n) == Some("mcp_server"))
        .unwrap();
    assert_ne!(srv_a.id, srv_b.id);
}

// ── Error handling ───────────────────────────────────────────────────────────

#[test]
fn missing_mcp_servers_key() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = write_json(tmp.path(), ".mcp.json", &json!({"unrelated": "shape"}));
    let r = extract_mcp_config(&p);
    assert!(r.nodes.is_empty());
    assert!(r.edges.is_empty());
    assert!(r.error.unwrap().contains("no mcpServers map"));
}

#[test]
fn nested_mcp_servers_shape() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = write_json(
        tmp.path(),
        ".mcp.json",
        &json!({"mcp": {"servers": {"x": {"command": "node", "args": ["dist/index.js"]}}}}),
    );
    let r = extract_mcp_config(&p);
    assert!(r.error.is_none());
    assert!(label_by_kind(&r, "mcp_server").contains(&"x"));
    assert!(label_by_kind(&r, "mcp_command").contains(&"node"));
}

#[test]
fn malformed_json_returns_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = tmp.path().join(".mcp.json");
    std::fs::write(&p, "{not valid json").expect("write");
    let r = extract_mcp_config(&p);
    assert!(r.nodes.is_empty());
    assert!(r.edges.is_empty());
    assert!(r.error.unwrap().contains("json error"));
}

#[test]
fn oversize_file_skipped() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = tmp.path().join(".mcp.json");
    let payload = format!(
        "{{\"mcpServers\":{{\"x\":{{\"command\":\"npx\",\"args\":[\"{}\"]}}}}}}",
        "a".repeat(2_000_000)
    );
    std::fs::write(&p, payload).expect("write");
    let r = extract_mcp_config(&p);
    assert!(r.error.unwrap().contains("too large"));
}

#[test]
fn root_not_an_object() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = tmp.path().join(".mcp.json");
    std::fs::write(&p, "[1, 2, 3]").expect("write");
    let r = extract_mcp_config(&p);
    assert!(r.error.unwrap().contains("root is not an object"));
}

#[test]
fn non_dict_server_entry_skipped() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = write_json(
        tmp.path(),
        ".mcp.json",
        &json!({"mcpServers": {
            "valid": {"command": "npx", "args": ["@scope/pkg"]},
            "broken": ["this", "is", "not", "an", "object"],
        }}),
    );
    let r = extract_mcp_config(&p);
    let servers = label_by_kind(&r, "mcp_server");
    assert!(servers.contains(&"valid"));
    assert!(!servers.contains(&"broken"));
}

// ── Edge case: package detection ─────────────────────────────────────────────

#[test]
fn package_detection_skips_flags() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = write_json(
        tmp.path(),
        ".mcp.json",
        &json!({"mcpServers": {"x": {"command": "npx", "args": ["-y", "@scope/server-x"]}}}),
    );
    let r = extract_mcp_config(&p);
    assert!(label_by_kind(&r, "mcp_package").contains(&"@scope/server-x"));
}

#[test]
fn no_package_detected_for_unknown_arg_shape() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = write_json(
        tmp.path(),
        ".mcp.json",
        &json!({"mcpServers": {"x": {"command": "node", "args": ["./local-script.js", "--verbose"]}}}),
    );
    let r = extract_mcp_config(&p);
    assert_eq!(label_by_kind(&r, "mcp_package"), Vec::<&str>::new());
}

#[test]
fn server_without_command_still_emits_server_node() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = write_json(
        tmp.path(),
        ".mcp.json",
        &json!({"mcpServers": {"x": {"args": ["@scope/server-x"]}}}),
    );
    let r = extract_mcp_config(&p);
    assert!(label_by_kind(&r, "mcp_server").contains(&"x"));
    assert_eq!(label_by_kind(&r, "mcp_command"), Vec::<&str>::new());
}

// ── Integration: dispatch routes filename-matched files to mcp_ingest ────────

#[test]
fn dispatch_routes_mcp_filename_to_mcp_extractor() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = write_json(
        tmp.path(),
        ".mcp.json",
        &json!({"mcpServers": {"x": {"command": "npx", "args": ["@scope/server-x"]}}}),
    );
    let out = extract(&[p], None);
    let has_server = out.nodes.iter().any(|n| {
        n.get("metadata")
            .and_then(|m| m.get("mcp_kind"))
            .and_then(Value::as_str)
            == Some("mcp_server")
    });
    assert!(has_server, "MCP file must route to MCP extractor");
}

#[test]
fn dispatch_does_not_reroute_generic_json() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = write_json(
        tmp.path(),
        "package.json",
        &json!({"name": "x", "version": "1.0.0"}),
    );
    let out = extract(&[p], None);
    let has_mcp = out
        .nodes
        .iter()
        .any(|n| n.get("metadata").and_then(|m| m.get("mcp_kind")).is_some());
    assert!(!has_mcp, "generic JSON must not route to MCP extractor");
}
