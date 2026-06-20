//! Parity tests for PowerShell Import-Module / dot-source edges and `.psd1`
//! manifest ingestion (#1331, #1315), ported from
//! `graphify-py/tests/test_languages.py`.

use std::path::{Path, PathBuf};

use graphify_extract::{FileResult, extract, extract_powershell, extract_powershell_manifest};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn import_targets(r: &FileResult) -> Vec<String> {
    r.edges
        .iter()
        .filter(|e| e.relation == "imports_from")
        .map(|e| e.target.clone())
        .collect()
}

fn has_target(r: &FileResult, needle: &str) -> bool {
    import_targets(r).iter().any(|t| t.contains(needle))
}

// ── Import-Module + dot-source (#1331) ───────────────────────────────────────

#[test]
fn powershell_import_module_emits_edge() {
    let r = extract_powershell(&fixtures().join("sample_import.ps1"));
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(has_target(&r, "foo"), "{:?}", import_targets(&r));
}

#[test]
fn powershell_import_module_with_name_param() {
    let r = extract_powershell(&fixtures().join("sample_import.ps1"));
    assert!(has_target(&r, "bar"), "{:?}", import_targets(&r));
}

#[test]
fn powershell_dot_source_forward_slash_emits_edge() {
    let r = extract_powershell(&fixtures().join("sample_import.ps1"));
    assert!(has_target(&r, "shared"), "{:?}", import_targets(&r));
}

#[test]
fn powershell_dot_source_backslash_emits_edge() {
    let r = extract_powershell(&fixtures().join("sample_import.ps1"));
    assert!(has_target(&r, "utils"), "{:?}", import_targets(&r));
}

#[test]
fn powershell_import_module_inside_function_emits_edge() {
    let r = extract_powershell(&fixtures().join("sample_import.ps1"));
    assert!(has_target(&r, "innermod"), "{:?}", import_targets(&r));
}

#[test]
fn powershell_dot_source_inside_function_emits_edge() {
    let r = extract_powershell(&fixtures().join("sample_import.ps1"));
    assert!(has_target(&r, "innershared"), "{:?}", import_targets(&r));
}

#[test]
fn powershell_import_module_not_a_raw_call() {
    let r = extract_powershell(&fixtures().join("sample_import.ps1"));
    let bad: Vec<&str> = r
        .raw_calls
        .iter()
        .filter(|rc| rc.callee.eq_ignore_ascii_case("import-module"))
        .map(|rc| rc.callee.as_str())
        .collect();
    assert!(
        bad.is_empty(),
        "Import-Module leaked into raw_calls: {bad:?}"
    );
}

#[test]
fn powershell_psm1_dispatched_and_extracted() -> Result<(), Box<dyn std::error::Error>> {
    // #1315: .psm1 routes to extract_powershell and is indexed.
    let tmp = tempfile::tempdir()?;
    let mod_path = tmp.path().join("Utils.psm1");
    std::fs::write(
        &mod_path,
        "function Get-Greeting { param([string]$Name) return \"Hi $Name\" }\n",
    )?;
    let res = extract(&[mod_path], Some(tmp.path()));
    assert!(
        res.nodes.iter().any(|n| n
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("Get-Greeting")),
        "psm1 not dispatched/extracted"
    );
    Ok(())
}

// ── PowerShell manifest (.psd1) (#1331) ──────────────────────────────────────

#[test]
fn powershell_psd1_no_error() {
    let r = extract_powershell_manifest(&fixtures().join("sample.psd1"));
    assert!(r.error.is_none(), "{:?}", r.error);
}

#[test]
fn powershell_psd1_has_file_node() {
    let r = extract_powershell_manifest(&fixtures().join("sample.psd1"));
    assert!(
        r.nodes.iter().any(|n| n.label.contains("sample.psd1")),
        "missing file node; nodes={:?}",
        r.nodes.iter().map(|n| &n.label).collect::<Vec<_>>()
    );
}

#[test]
fn powershell_psd1_root_module() {
    let r = extract_powershell_manifest(&fixtures().join("sample.psd1"));
    assert!(has_target(&r, "mymodule"), "{:?}", import_targets(&r));
}

#[test]
fn powershell_psd1_nested_modules() {
    let r = extract_powershell_manifest(&fixtures().join("sample.psd1"));
    assert!(has_target(&r, "helpers"), "{:?}", import_targets(&r));
    assert!(has_target(&r, "logger"), "{:?}", import_targets(&r));
}

#[test]
fn powershell_psd1_required_modules_string() {
    let r = extract_powershell_manifest(&fixtures().join("sample.psd1"));
    assert!(has_target(&r, "psreadline"), "{:?}", import_targets(&r));
}

#[test]
fn powershell_psd1_required_modules_hashtable() {
    let r = extract_powershell_manifest(&fixtures().join("sample.psd1"));
    assert!(has_target(&r, "pester"), "{:?}", import_targets(&r));
}

#[test]
fn powershell_psd1_no_moduleversion_as_edge() {
    let r = extract_powershell_manifest(&fixtures().join("sample.psd1"));
    let targets = import_targets(&r);
    for bad in ["5_0", "1_0_0", "5.0", "1.0.0"] {
        assert!(
            !targets.iter().any(|t| t == bad),
            "ModuleVersion leaked into targets: {targets:?}"
        );
    }
}

#[test]
fn powershell_psd1_no_dangling_edges() {
    let r = extract_powershell_manifest(&fixtures().join("sample.psd1"));
    let node_ids: std::collections::HashSet<&str> = r.nodes.iter().map(|n| n.id.as_str()).collect();
    for e in &r.edges {
        assert!(
            node_ids.contains(e.source.as_str()),
            "dangling source: {e:?}"
        );
    }
}
