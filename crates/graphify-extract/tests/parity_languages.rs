//! Coverage tests for per-language extractors.
//!
//! Exercises every public `extract_<lang>` entry point on a small fixture so the
//! tree-sitter walk paths in each language module are executed at least once.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use graphify_extract::{
    extract_astro, extract_blade, extract_c, extract_cpp, extract_csharp, extract_dart,
    extract_delphi_form, extract_elixir, extract_fortran, extract_go, extract_groovy, extract_java,
    extract_julia, extract_kotlin, extract_lazarus_form, extract_lazarus_package, extract_lua,
    extract_markdown, extract_objc, extract_pascal, extract_php, extract_powershell, extract_ruby,
    extract_rust, extract_scala, extract_sql, extract_svelte, extract_swift, extract_verilog,
    extract_zig,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn assert_no_dangling_edges(result: &graphify_extract::FileResult) {
    let ids: std::collections::HashSet<&str> = result.nodes.iter().map(|n| n.id.as_str()).collect();
    for edge in &result.edges {
        assert!(
            ids.contains(edge.source.as_str()),
            "dangling source {} in edges",
            edge.source
        );
    }
}

#[test]
fn pascal_extractor_produces_nodes() {
    let result = extract_pascal(&fixtures().join("sample.pas"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no pascal nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn delphi_form_extractor_produces_nodes() {
    let result = extract_delphi_form(&fixtures().join("sample.dfm"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no dfm nodes");
}

#[test]
fn lazarus_form_extractor_produces_nodes() {
    let result = extract_lazarus_form(&fixtures().join("sample.lfm"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no lfm nodes");
}

#[test]
fn lazarus_package_extractor_produces_nodes() {
    let result = extract_lazarus_package(&fixtures().join("sample.lpk"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no lpk nodes");
}

#[test]
fn sql_extractor_produces_nodes() {
    let result = extract_sql(&fixtures().join("sample.sql"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no sql nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn sql_alter_fk_extractor() {
    let result = extract_sql(&fixtures().join("sample_alter_fk.sql"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no nodes for alter fk sql");
}

#[test]
fn sql_schema_qualified_extractor() {
    let result = extract_sql(&fixtures().join("sample_schema_qualified.sql"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(
        !result.nodes.is_empty(),
        "no nodes for schema-qualified sql"
    );
}

#[test]
fn sql_complex_fixture_extracts_many_objects() {
    // Exercises CREATE TABLE+constraints, CREATE VIEW, CREATE MATERIALIZED VIEW,
    // CREATE FUNCTION, CREATE PROCEDURE, CREATE TRIGGER, CREATE INDEX,
    // ALTER TABLE ADD CONSTRAINT, CREATE SEQUENCE, JOIN references, etc.
    let result = extract_sql(&fixtures().join("sample_complex.sql"));
    assert!(result.error.is_none(), "{:?}", result.error);
    // Many objects → many nodes.
    assert!(
        result.nodes.len() > 5,
        "expected lots of nodes, got {}",
        result.nodes.len()
    );
    // FK references should produce `references` edges.
    let has_refs = result.edges.iter().any(|e| e.relation == "references");
    assert!(has_refs, "expected references edges from FK constraints");
}

#[test]
fn julia_extractor_produces_nodes() {
    let result = extract_julia(&fixtures().join("sample.jl"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no julia nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn julia_macros_and_parametric_types() {
    let result = extract_julia(&fixtures().join("sample_macros.jl"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(
        result.nodes.len() > 5,
        "expected lots of nodes from macros fixture"
    );
}

#[test]
fn objc_extractor_produces_nodes() {
    let result = extract_objc(&fixtures().join("sample.m"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no objc nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn objc_protocols_and_categories() {
    let result = extract_objc(&fixtures().join("sample_protocols.m"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(
        result.nodes.len() > 3,
        "expected lots of nodes from objc protocols"
    );
}

#[test]
fn go_extractor_produces_nodes() {
    let result = extract_go(&fixtures().join("sample.go"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no go nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn go_interfaces_and_methods() {
    let result = extract_go(&fixtures().join("sample_interfaces.go"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(result.nodes.len() > 5, "expected many go nodes");
}

#[test]
fn fortran_extractor_produces_nodes() {
    let result = extract_fortran(&fixtures().join("sample.f90"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no fortran nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn elixir_extractor_produces_nodes() {
    let result = extract_elixir(&fixtures().join("sample.ex"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no elixir nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn powershell_extractor_produces_nodes() {
    let result = extract_powershell(&fixtures().join("sample.ps1"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no powershell nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn powershell_advanced_classes_enums_configs() {
    let result = extract_powershell(&fixtures().join("sample_advanced.ps1"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(
        result.nodes.len() > 5,
        "expected many powershell nodes from advanced fixture"
    );
}

#[test]
fn zig_extractor_produces_nodes() {
    let result = extract_zig(&fixtures().join("sample.zig"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no zig nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn verilog_extractor_produces_nodes() {
    let result = extract_verilog(&fixtures().join("sample.v"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no verilog nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn systemverilog_complex_extracts_nodes() {
    let result = extract_verilog(&fixtures().join("sample_complex.sv"));
    assert!(result.error.is_none(), "{:?}", result.error);
    // Even if tree-sitter-verilog stumbles on advanced SV constructs, the
    // file node + a few module/task/function nodes should be present.
    assert!(!result.nodes.is_empty(), "no verilog nodes from complex sv");
}

#[test]
fn rust_extractor_produces_nodes() {
    let result = extract_rust(&fixtures().join("sample.rs"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no rust nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn markdown_extractor_produces_nodes() {
    let result = extract_markdown(&fixtures().join("sample.md"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no markdown nodes");
}

#[test]
fn markdown_rich_doc_extracts_headings_and_code_blocks() {
    let result = extract_markdown(&fixtures().join("sample_rich.md"));
    assert!(result.error.is_none(), "{:?}", result.error);
    // Headings and code blocks both produce nodes.
    assert!(result.nodes.len() > 5, "expected many nodes from rich md");
}

#[test]
fn c_extractor_produces_nodes() {
    let result = extract_c(&fixtures().join("sample.c"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no c nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn cpp_extractor_produces_nodes() {
    let result = extract_cpp(&fixtures().join("sample.cpp"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no cpp nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn csharp_extractor_produces_nodes() {
    let result = extract_csharp(&fixtures().join("sample.cs"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no csharp nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn ruby_extractor_produces_nodes() {
    let result = extract_ruby(&fixtures().join("sample.rb"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no ruby nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn java_extractor_produces_nodes() {
    let result = extract_java(&fixtures().join("sample.java"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no java nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn groovy_extractor_produces_nodes() {
    let result = extract_groovy(&fixtures().join("sample.groovy"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no groovy nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn groovy_spock_fallback_kicks_in() {
    // Spock test files use `def "feature name"()` and require the regex fallback.
    let result = extract_groovy(&fixtures().join("sample_spock.groovy"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no spock nodes");
}

#[test]
fn kotlin_extractor_produces_nodes() {
    let result = extract_kotlin(&fixtures().join("sample.kt"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no kotlin nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn scala_extractor_produces_nodes() {
    let result = extract_scala(&fixtures().join("sample.scala"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no scala nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn php_extractor_produces_nodes() {
    let result = extract_php(&fixtures().join("sample.php"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no php nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn lua_extractor_produces_nodes() {
    let result = extract_lua(&fixtures().join("sample.luau"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no lua nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn swift_extractor_produces_nodes() {
    let result = extract_swift(&fixtures().join("sample.swift"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no swift nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn dart_extractor_produces_nodes() {
    let result = extract_dart(&fixtures().join("sample.dart"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no dart nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn blade_extractor_produces_nodes() {
    let result = extract_blade(&fixtures().join("sample.blade.php"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no blade nodes");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(
        labels.iter().any(|l| l.contains("partials.header")
            || l.contains("user-profile")
            || l.contains("save")),
        "expected blade include/livewire/wire-click label, got {labels:?}"
    );
}

#[test]
fn svelte_extractor_produces_nodes() {
    let result = extract_svelte(&fixtures().join("sample.svelte"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no svelte nodes");
}

#[test]
fn astro_extractor_produces_nodes() {
    let result = extract_astro(&fixtures().join("sample.astro"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no astro nodes");
}

#[test]
fn python_extracts_varied_imports() {
    use graphify_extract::extract_python;
    let result = extract_python(&fixtures().join("imports_python.py"));
    assert!(result.error.is_none(), "{:?}", result.error);
    // The file has many imports → expect both imports and imports_from edges.
    let has_imports = result.edges.iter().any(|e| e.relation == "imports");
    let has_imports_from = result.edges.iter().any(|e| e.relation == "imports_from");
    assert!(has_imports, "expected `imports` edges");
    assert!(has_imports_from, "expected `imports_from` edges");
}

#[test]
fn typescript_extracts_varied_imports() {
    use graphify_extract::extract_js;
    let result = extract_js(&fixtures().join("imports_js.ts"));
    assert!(result.error.is_none(), "{:?}", result.error);
    let has_imports_from = result.edges.iter().any(|e| e.relation == "imports_from");
    assert!(
        has_imports_from,
        "expected `imports_from` edges from TS imports"
    );
}

#[test]
fn extractors_handle_missing_file() {
    // Each extractor should return an error result for a nonexistent file
    // rather than panicking.
    let bogus = fixtures().join("does_not_exist.xyz");
    assert!(extract_blade(&bogus).error.is_some());
    assert!(extract_pascal(&bogus).error.is_some());
    assert!(extract_lazarus_form(&bogus).error.is_some());
    assert!(extract_lazarus_package(&bogus).error.is_some());
    assert!(extract_delphi_form(&bogus).error.is_some());
}
