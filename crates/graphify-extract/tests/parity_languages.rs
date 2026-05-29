//! Coverage tests for per-language extractors.
//!
//! Exercises every public `extract_<lang>` entry point on a small fixture so the
//! tree-sitter walk paths in each language module are executed at least once.

#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};

use graphify_extract::{
    FileResult, extract_astro, extract_blade, extract_c, extract_cpp, extract_csharp,
    extract_csproj, extract_dart, extract_delphi_form, extract_dm, extract_dmf, extract_dmi,
    extract_dmm, extract_elixir, extract_fortran, extract_go, extract_groovy, extract_java,
    extract_julia, extract_kotlin, extract_lazarus_form, extract_lazarus_package, extract_lua,
    extract_markdown, extract_objc, extract_pascal, extract_php, extract_powershell, extract_razor,
    extract_ruby, extract_rust, extract_scala, extract_sln, extract_sql, extract_svelte,
    extract_swift, extract_verilog, extract_zig, file_stem, make_id,
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

/// Helper used by the cross-language reference-context parity tests below.
/// Maps a node label as it appears on graph edges back to its display label,
/// stripping `()` suffixes and leading `.` characters so the assertions stay
/// readable.
#[must_use]
fn normalise_label(label: &str) -> String {
    label
        .trim_end_matches("()")
        .trim_start_matches('.')
        .to_string()
}

/// Return the set of `(source_label, target_label)` pairs for edges matching
/// `relation` (and optionally `context`). Used by reference-context tests so
/// the assertion reads like Python's `_edge_labels(result, relation, context)`.
#[must_use]
fn edge_label_pairs(
    result: &graphify_extract::types::FileResult,
    relation: &str,
    context: Option<&str>,
) -> std::collections::HashSet<(String, String)> {
    let id_to_label: std::collections::HashMap<&str, String> = result
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), normalise_label(&n.label)))
        .collect();
    result
        .edges
        .iter()
        .filter(|e| e.relation == relation)
        .filter(|e| match context {
            Some(c) => e.context.as_deref() == Some(c),
            None => true,
        })
        .map(|e| {
            (
                id_to_label
                    .get(e.source.as_str())
                    .cloned()
                    .unwrap_or_else(|| e.source.clone()),
                id_to_label
                    .get(e.target.as_str())
                    .cloned()
                    .unwrap_or_else(|| e.target.clone()),
            )
        })
        .collect()
}

/// C# `base_list` entries are split into `inherits` (base class) and
/// `implements` (interface implementation). The pre-scan recognises `IProcessor`
/// as an interface via both the in-file `interface IProcessor` declaration AND
/// the `I<UpperLetter>…` naming convention.
///
/// Ports `tests/test_languages.py::test_csharp_splits_inherits_and_implements_edges`.
#[test]
fn csharp_splits_inherits_and_implements_edges() {
    let result = extract_csharp(&fixtures().join("sample.cs"));
    assert!(result.error.is_none(), "{:?}", result.error);
    let inherits = edge_label_pairs(&result, "inherits", None);
    let implements = edge_label_pairs(&result, "implements", None);
    assert!(
        inherits.contains(&("DataProcessor".to_string(), "Processor".to_string())),
        "expected DataProcessor inherits Processor, got inherits={inherits:?}"
    );
    assert!(
        implements.contains(&("DataProcessor".to_string(), "IProcessor".to_string())),
        "expected DataProcessor implements IProcessor, got implements={implements:?}"
    );
}

/// Java's source-level `extends` keyword (class extending a base class) is
/// normalised to the `inherits` relation. `implements` (class implementing
/// an interface) keeps its name.
///
/// Ports `tests/test_languages.py::test_java_normalizes_inherits_and_implements`.
#[test]
fn java_normalises_inherits_and_implements() {
    let result = extract_java(&fixtures().join("sample.java"));
    assert!(result.error.is_none(), "{:?}", result.error);
    let inherits = edge_label_pairs(&result, "inherits", None);
    let implements = edge_label_pairs(&result, "implements", None);
    assert!(
        inherits.contains(&("DataProcessor".to_string(), "BaseProcessor".to_string())),
        "expected DataProcessor inherits BaseProcessor, got inherits={inherits:?}"
    );
    assert!(
        implements.contains(&("DataProcessor".to_string(), "Processor".to_string())),
        "expected DataProcessor implements Processor, got implements={implements:?}"
    );
}

/// C# methods emit `references` edges tagged with `parameter_type`,
/// `return_type`, `generic_arg` based on the method signature shape.
///
/// Ports `tests/test_languages.py::test_csharp_parameter_return_and_generic_contexts`.
#[test]
fn csharp_parameter_return_and_generic_contexts() {
    let result = extract_csharp(&fixtures().join("sample.cs"));
    assert!(result.error.is_none(), "{:?}", result.error);
    let params = edge_label_pairs(&result, "references", Some("parameter_type"));
    let returns = edge_label_pairs(&result, "references", Some("return_type"));
    let generics = edge_label_pairs(&result, "references", Some("generic_arg"));
    assert!(
        params.contains(&("Build".to_string(), "HttpClient".to_string())),
        "expected Build(HttpClient) param edge, got {params:?}"
    );
    assert!(
        returns.contains(&("Build".to_string(), "Result".to_string())),
        "expected Build → Result return edge, got {returns:?}"
    );
    assert!(
        generics.contains(&("Build".to_string(), "DataProcessor".to_string())),
        "expected Build → DataProcessor generic_arg, got {generics:?}"
    );
}

/// Java methods emit `references` edges with `parameter_type`, `return_type`,
/// `generic_arg`, plus `attribute` for `@Override`-style annotations.
///
/// Ports `tests/test_languages.py::test_java_parameter_return_generic_and_attribute_contexts`.
#[test]
fn java_parameter_return_generic_and_attribute_contexts() {
    let result = extract_java(&fixtures().join("sample.java"));
    assert!(result.error.is_none(), "{:?}", result.error);
    let params = edge_label_pairs(&result, "references", Some("parameter_type"));
    let returns = edge_label_pairs(&result, "references", Some("return_type"));
    let generics = edge_label_pairs(&result, "references", Some("generic_arg"));
    let attrs = edge_label_pairs(&result, "references", Some("attribute"));
    assert!(
        params.contains(&("build".to_string(), "HttpClient".to_string())),
        "expected build(HttpClient) param edge, got {params:?}"
    );
    assert!(
        returns.contains(&("build".to_string(), "Result".to_string())),
        "expected build → Result return edge, got {returns:?}"
    );
    assert!(
        generics.contains(&("build".to_string(), "DataProcessor".to_string())),
        "expected build → DataProcessor generic_arg, got {generics:?}"
    );
    assert!(
        attrs.contains(&("build".to_string(), "Override".to_string())),
        "expected build → Override attribute, got {attrs:?}"
    );
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
fn dart_child_node_ids_are_stem_based() {
    // Child node IDs must be built from file_stem, not the absolute path, so
    // graph.json stays machine-independent (graphify-py #999).
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("mydir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let src_file = dir.join("sample.dart");
    std::fs::write(&src_file, b"class MyClass {}\nvoid myFunc() {}\n").expect("write");

    let result = extract_dart(&src_file);
    let stem = file_stem(&src_file); // -> "mydir.sample"
    let expected_class_nid = make_id(&[&stem, "MyClass"]); // -> "mydir_sample_myclass"
    let expected_func_nid = make_id(&[&stem, "myFunc"]); // -> "mydir_sample_myfunc"

    let node_ids: std::collections::HashSet<&str> =
        result.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(
        node_ids.contains(expected_class_nid.as_str()),
        "class nid {expected_class_nid} not in {node_ids:?}"
    );
    assert!(
        node_ids.contains(expected_func_nid.as_str()),
        "func nid {expected_func_nid} not in {node_ids:?}"
    );

    // No child node ID should leak a path separator fragment.
    let stem_prefix = stem.replace('.', "_");
    let file_label = src_file
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    for node in &result.nodes {
        if node.label == file_label {
            continue;
        }
        assert!(
            node.id.starts_with(&stem_prefix),
            "child id {} lacks stem prefix {stem_prefix}",
            node.id
        );
    }
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

// ── .NET project files (.sln, .csproj, .razor) ───────────────────────────────
//
// Ports the parity assertions from graphify-py `tests/test_dotnet.py` and the
// `# -- .NET project files (.sln, .csproj, .razor) --` section of
// `tests/test_languages.py`. The fixtures (`sample.sln`, `sample.csproj`,
// `sample.razor`) are copied verbatim from graphify-py so node labels and
// edge counts stay in lockstep.

#[must_use]
fn labels(r: &graphify_extract::FileResult) -> Vec<&str> {
    r.nodes.iter().map(|n| n.label.as_str()).collect()
}

#[must_use]
fn relations(r: &graphify_extract::FileResult) -> std::collections::HashSet<&str> {
    r.edges.iter().map(|e| e.relation.as_str()).collect()
}

#[test]
fn sln_extracts_projects() {
    let r = extract_sln(&fixtures().join("sample.sln"));
    assert!(r.error.is_none(), "{:?}", r.error);
    let ls: std::collections::HashSet<&str> = labels(&r).into_iter().collect();
    assert!(ls.contains("WebApi"));
    assert!(ls.contains("Domain"));
    assert!(ls.contains("Tests"));
}

#[test]
fn sln_contains_edges() {
    let r = extract_sln(&fixtures().join("sample.sln"));
    assert!(r.error.is_none(), "{:?}", r.error);
    let contains: Vec<_> = r
        .edges
        .iter()
        .filter(|e| e.relation == "contains")
        .collect();
    assert_eq!(contains.len(), 3);
}

#[test]
fn sln_project_dependency() {
    let r = extract_sln(&fixtures().join("sample.sln"));
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(relations(&r).contains("imports"));
}

#[test]
fn csproj_packages() {
    let r = extract_csproj(&fixtures().join("sample.csproj"));
    assert!(r.error.is_none(), "{:?}", r.error);
    let ls = labels(&r);
    assert!(ls.iter().any(|l| l.contains("MediatR")));
    assert!(ls.iter().any(|l| l.contains("FluentValidation")));
    assert!(ls.iter().any(|l| l.contains("Swashbuckle")));
}

#[test]
fn csproj_project_references() {
    let r = extract_csproj(&fixtures().join("sample.csproj"));
    assert!(r.error.is_none(), "{:?}", r.error);
    let imports: Vec<_> = r.edges.iter().filter(|e| e.relation == "imports").collect();
    assert_eq!(imports.len(), 6); // 4 packages + 2 project refs
}

#[test]
fn csproj_target_framework() {
    let r = extract_csproj(&fixtures().join("sample.csproj"));
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(labels(&r).contains(&"net8.0"));
}

#[test]
fn csproj_sdk() {
    let r = extract_csproj(&fixtures().join("sample.csproj"));
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(labels(&r).contains(&"Microsoft.NET.Sdk.Web"));
}

#[test]
fn csproj_invalid_xml() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bad = tmp.path().join("bad.csproj");
    std::fs::write(&bad, "<Project><Invalid></Project>").expect("write fixture");
    let r = extract_csproj(&bad);
    assert!(r.error.is_some());
}

#[test]
fn razor_using_and_inject() {
    let r = extract_razor(&fixtures().join("sample.razor"));
    assert!(r.error.is_none(), "{:?}", r.error);
    let targets: std::collections::HashSet<&str> = r
        .edges
        .iter()
        .filter(|e| e.relation == "imports")
        .map(|e| e.target.as_str())
        .collect();
    assert!(targets.iter().any(|t| t.contains("microsoft")));
    assert!(
        targets
            .iter()
            .any(|t| t.to_lowercase().contains("counterservice"))
    );
}

#[test]
fn razor_components() {
    let r = extract_razor(&fixtures().join("sample.razor"));
    assert!(r.error.is_none(), "{:?}", r.error);
    let targets: std::collections::HashSet<&str> = r
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .map(|e| e.target.as_str())
        .collect();
    assert!(targets.iter().any(|t| t.contains("weatherdisplay")));
    assert!(targets.iter().any(|t| t.contains("datagrid")));
}

#[test]
fn razor_page_route() {
    let r = extract_razor(&fixtures().join("sample.razor"));
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(labels(&r).iter().any(|l| l.contains("/counter")));
}

#[test]
fn razor_inherits() {
    let r = extract_razor(&fixtures().join("sample.razor"));
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(relations(&r).contains("inherits"));
}

#[test]
fn razor_code_methods() {
    let r = extract_razor(&fixtures().join("sample.razor"));
    assert!(r.error.is_none(), "{:?}", r.error);
    let ls = labels(&r);
    assert!(ls.contains(&"IncrementCount"));
    assert!(ls.contains(&"LoadData"));
}

#[test]
fn fsproj_extractor_produces_nodes() {
    // `.fsproj` (F#) routes through `extract_csproj` — same MSBuild
    // schema, different file extension. Smoke-tests the dispatch path.
    let tmp = tempfile::tempdir().expect("tempdir");
    let fixture = tmp.path().join("Lib.fsproj");
    std::fs::write(
        &fixture,
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  \
         <PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup>\n  \
         <ItemGroup><PackageReference Include=\"FSharp.Core\" Version=\"8.0.0\" /></ItemGroup>\n\
         </Project>\n",
    )
    .expect("write fsproj fixture");
    let r = extract_csproj(&fixture);
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(!r.nodes.is_empty());
    assert!(labels(&r).contains(&"net8.0"));
}

#[test]
fn vbproj_extractor_produces_nodes() {
    // `.vbproj` (VB.NET) — same path as `extract_csproj`.
    let tmp = tempfile::tempdir().expect("tempdir");
    let fixture = tmp.path().join("Lib.vbproj");
    std::fs::write(
        &fixture,
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  \
         <PropertyGroup><TargetFramework>net6.0</TargetFramework></PropertyGroup>\n  \
         <ItemGroup><PackageReference Include=\"Newtonsoft.Json\" Version=\"13.0.3\" /></ItemGroup>\n\
         </Project>\n",
    )
    .expect("write vbproj fixture");
    let r = extract_csproj(&fixture);
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(!r.nodes.is_empty());
    assert!(labels(&r).iter().any(|l| l.contains("Newtonsoft.Json")));
}

#[test]
fn cshtml_extractor_produces_nodes() {
    // `.cshtml` (Razor Pages / MVC views) routes to `extract_razor`.
    let tmp = tempfile::tempdir().expect("tempdir");
    let fixture = tmp.path().join("Index.cshtml");
    std::fs::write(
        &fixture,
        "@page\n@model IndexModel\n@using MyApp.Services\n\
         <h1>Hello</h1>\n",
    )
    .expect("write cshtml fixture");
    let r = extract_razor(&fixture);
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(!r.nodes.is_empty());
    assert!(relations(&r).contains("imports"));
}

#[test]
fn razor_code_block_brace_counter_ignores_strings_and_comments() {
    // Regression for the brace-counter divergence from graphify-py: a
    // method body with `"}{"` or a `}` inside a `// ...` comment used
    // to truncate `block_end` early, silently dropping every method
    // declared further down the @code block. The state-machine scan
    // in `find_csharp_block_end` should keep both methods discoverable.
    let tmp = tempfile::tempdir().expect("tempdir");
    let fixture = tmp.path().join("Bug.razor");
    std::fs::write(
        &fixture,
        "@page \"/bug\"\n\n@code {\n    \
         private string s = \"}{\";\n    \
         // method body terminator: }\n    \
         private void First() { Console.WriteLine(\"}\"); }\n    \
         public async Task Second() { return; }\n}\n",
    )
    .expect("write razor fixture");
    let r = extract_razor(&fixture);
    assert!(r.error.is_none(), "{:?}", r.error);
    let ls = labels(&r);
    assert!(
        ls.contains(&"First"),
        "First missing — brace counter truncated early: {ls:?}"
    );
    assert!(
        ls.contains(&"Second"),
        "Second missing — brace counter truncated early: {ls:?}"
    );
}

#[test]
fn razor_missing_file() {
    // Build a guaranteed-nonexistent path inside a tempdir so the assertion
    // holds on Windows (where `/nonexistent/...` would otherwise resolve
    // against the current drive). The tempdir is dropped at scope end; the
    // child path inside it never gets created.
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing = tmp.path().join("does_not_exist.razor");
    let r = extract_razor(&missing);
    assert!(r.error.is_some());
}

#[test]
fn razor_no_dangling_edges() {
    let r = extract_razor(&fixtures().join("sample.razor"));
    assert_no_dangling_edges(&r);
}

// ── BYOND DreamMaker (.dm / .dme) ───────────────────────────────────────────
//
// Ports the DM/DMI/DMM/DMF cases from graphify-py `tests/test_languages.py`.
// (Reuses the existing `labels` helper above for node labels.)

/// `(source_label, target_label)` pairs for `calls` edges (Python `_calls`).
fn calls(r: &FileResult) -> Vec<(String, String)> {
    let by_id: std::collections::HashMap<&str, &str> = r
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    r.edges
        .iter()
        .filter(|e| e.relation == "calls")
        .map(|e| {
            (
                by_id
                    .get(e.source.as_str())
                    .map_or(e.source.clone(), |s| (*s).to_string()),
                by_id
                    .get(e.target.as_str())
                    .map_or(e.target.clone(), |s| (*s).to_string()),
            )
        })
        .collect()
}

/// Edges whose relation is in `relations` (Python `_edges_with_relation`).
fn edges_with_relation<'a>(
    r: &'a FileResult,
    relations: &[&str],
) -> Vec<&'a graphify_extract::Edge> {
    r.edges
        .iter()
        .filter(|e| relations.contains(&e.relation.as_str()))
        .collect()
}

#[test]
fn dm_no_error() {
    let r = extract_dm(&fixtures().join("sample.dm"));
    assert!(r.error.is_none());
}

#[test]
fn dm_finds_global_proc() {
    let r = extract_dm(&fixtures().join("sample.dm"));
    let ls = labels(&r);
    assert!(ls.contains(&"log_event()"));
    assert!(ls.contains(&"RunTest()"));
}

#[test]
fn dm_finds_type_definition() {
    let r = extract_dm(&fixtures().join("sample.dm"));
    let ls = labels(&r);
    assert!(ls.contains(&"/datum/weapon"));
    assert!(ls.contains(&"/datum/weapon/sword"));
}

#[test]
fn dm_qualifies_proc_with_type_path() {
    let r = extract_dm(&fixtures().join("sample.dm"));
    let ls = labels(&r);
    assert!(ls.contains(&"/datum/weapon/attack()"));
    assert!(ls.contains(&"/datum/weapon/sword/attack()"));
}

#[test]
fn dm_finds_path_form_proc_definition() {
    let r = extract_dm(&fixtures().join("sample.dm"));
    assert!(labels(&r).contains(&"/datum/weapon/sword/sharpen()"));
}

#[test]
fn dm_emits_include_edge() {
    let r = extract_dm(&fixtures().join("sample.dm"));
    let import_edges = edges_with_relation(&r, &["imports", "imports_from"]);
    assert!(!import_edges.is_empty());
    assert!(
        import_edges
            .iter()
            .all(|e| e.context.as_deref() == Some("import"))
    );
}

#[test]
fn dm_unresolved_include_flagged_external() {
    let r = extract_dm(&fixtures().join("sample.dm"));
    let import_edges = edges_with_relation(&r, &["imports", "imports_from"]);
    let helpers: Vec<_> = import_edges
        .iter()
        .filter(|e| e.target.contains("helpers"))
        .collect();
    assert!(!helpers.is_empty());
    assert!(helpers.iter().all(|e| e.external));
}

#[test]
fn dm_resolves_in_file_calls() {
    let r = extract_dm(&fixtures().join("sample.dm"));
    let cs = calls(&r);
    assert!(cs.iter().any(|(_, callee)| callee == "log_event()"));
    assert!(cs.contains(&(
        "/datum/weapon/sword/attack()".to_string(),
        "/datum/weapon/sword/sharpen()".to_string(),
    )));
}

#[test]
fn dm_ambiguous_member_call_left_unresolved() {
    let r = extract_dm(&fixtures().join("sample.dm"));
    let cs = calls(&r);
    assert!(
        !cs.iter()
            .any(|(s, c)| s == "RunTest()" && c.contains("attack"))
    );
    assert!(r.raw_calls.iter().any(|rc| rc.callee == "attack"));
}

#[test]
fn dm_emits_new_as_instantiates() {
    let r = extract_dm(&fixtures().join("sample.dm"));
    let by_id: std::collections::HashMap<&str, &str> = r
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    let inst: Vec<(Option<&&str>, Option<&&str>)> = r
        .edges
        .iter()
        .filter(|e| e.relation == "instantiates")
        .map(|e| (by_id.get(e.source.as_str()), by_id.get(e.target.as_str())))
        .collect();
    assert!(inst.contains(&(Some(&"RunTest()"), Some(&"/datum/weapon/sword"))));
}

#[test]
fn dm_call_edges_have_call_context() {
    let r = extract_dm(&fixtures().join("sample.dm"));
    let call_edges = edges_with_relation(&r, &["calls", "instantiates"]);
    assert!(!call_edges.is_empty());
    assert!(
        call_edges
            .iter()
            .all(|e| e.context.as_deref() == Some("call"))
    );
}

#[test]
fn dm_no_dangling_edges() {
    let r = extract_dm(&fixtures().join("sample.dm"));
    let ids: std::collections::HashSet<&str> = r.nodes.iter().map(|n| n.id.as_str()).collect();
    for e in &r.edges {
        assert!(
            ids.contains(e.source.as_str()),
            "dangling source {}",
            e.source
        );
    }
}

#[test]
fn dm_super_call_not_emitted() {
    let r = extract_dm(&fixtures().join("sample.dm"));
    let cs = calls(&r);
    assert!(
        !cs.iter()
            .any(|(_, c)| c.trim_matches(|ch| ch == '(' || ch == ')') == "..")
    );
    assert!(!r.raw_calls.iter().any(|rc| rc.callee == ".."));
}

// ── DMI (BYOND icon sheets) ─────────────────────────────────────────────────

#[test]
fn dmi_no_error() {
    let r = extract_dmi(&fixtures().join("sample.dmi"));
    assert!(r.error.is_none());
}

#[test]
fn dmi_emits_state_nodes() {
    let r = extract_dmi(&fixtures().join("sample.dmi"));
    assert!(labels(&r).contains(&"\"mob\""));
}

#[test]
fn dmi_state_contained_by_file() {
    let r = extract_dmi(&fixtures().join("sample.dmi"));
    let by_id: std::collections::HashMap<&str, &str> = r
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    let contains: Vec<(Option<&&str>, Option<&&str>)> = r
        .edges
        .iter()
        .filter(|e| e.relation == "contains")
        .map(|e| (by_id.get(e.source.as_str()), by_id.get(e.target.as_str())))
        .collect();
    assert!(contains.contains(&(Some(&"sample.dmi"), Some(&"\"mob\""))));
}

// ── DMM (BYOND map files) ───────────────────────────────────────────────────

#[test]
fn dmm_no_error() {
    let r = extract_dmm(&fixtures().join("sample.dmm"));
    assert!(r.error.is_none());
}

#[test]
fn dmm_extracts_type_paths_as_uses_edges() {
    let r = extract_dmm(&fixtures().join("sample.dmm"));
    let targets: std::collections::HashSet<&str> = r
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .map(|e| e.target.as_str())
        .collect();
    assert!(targets.contains("turf_closed_wall"));
    assert!(targets.contains("obj_structure_table"));
    assert!(targets.contains("obj_item_weapon_sword"));
}

#[test]
fn dmm_strips_var_overrides() {
    let r = extract_dmm(&fixtures().join("sample.dmm"));
    let targets: std::collections::HashSet<&str> = r
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .map(|e| e.target.as_str())
        .collect();
    assert!(!targets.iter().any(|t| t.contains('{')));
    assert!(targets.contains("obj_item_weapon_sword"));
}

#[test]
fn dmm_handles_multiline_tile_definition() {
    let r = extract_dmm(&fixtures().join("sample.dmm"));
    let targets: std::collections::HashSet<&str> = r
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .map(|e| e.target.as_str())
        .collect();
    assert!(targets.contains("area_station_maintenance"));
}

#[test]
fn dmm_skips_grid_section() {
    let r = extract_dmm(&fixtures().join("sample.dmm"));
    let targets: std::collections::HashSet<&str> = r
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .map(|e| e.target.as_str())
        .collect();
    assert_eq!(targets.len(), 5);
}

// ── DMF (BYOND interface forms) ─────────────────────────────────────────────

#[test]
fn dmf_no_error() {
    let r = extract_dmf(&fixtures().join("sample.dmf"));
    assert!(r.error.is_none());
}

#[test]
fn dmf_extracts_windows() {
    let r = extract_dmf(&fixtures().join("sample.dmf"));
    let ls = labels(&r);
    assert!(ls.contains(&"window \"mapwindow\""));
    assert!(ls.contains(&"window \"infowindow\""));
}

#[test]
fn dmf_elem_labels_carry_control_type() {
    let r = extract_dmf(&fixtures().join("sample.dmf"));
    assert!(labels(&r).contains(&"elem \"map\" [MAP]"));
}

#[test]
fn dmf_elem_under_window() {
    let r = extract_dmf(&fixtures().join("sample.dmf"));
    let by_id: std::collections::HashMap<&str, &str> = r
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    let contains: Vec<(Option<&&str>, Option<&&str>)> = r
        .edges
        .iter()
        .filter(|e| e.relation == "contains")
        .map(|e| (by_id.get(e.source.as_str()), by_id.get(e.target.as_str())))
        .collect();
    assert!(contains.contains(&(Some(&"window \"mapwindow\""), Some(&"elem \"map\" [MAP]"))));
}

#[test]
fn dmf_no_dangling_edges() {
    let r = extract_dmf(&fixtures().join("sample.dmf"));
    let ids: std::collections::HashSet<&str> = r.nodes.iter().map(|n| n.id.as_str()).collect();
    for e in &r.edges {
        assert!(
            ids.contains(e.source.as_str()),
            "dangling source {}",
            e.source
        );
        assert!(
            ids.contains(e.target.as_str()),
            "dangling target {}",
            e.target
        );
    }
}

// ── BYOND coverage tests (beyond the graphify-py parity suite) ──────────────
//
// These exercise paths the shipped fixtures don't reach: the zTXt (compressed)
// `.dmi` branch — the v0.8.22 capped-decompression security fix — plus resolved
// includes and error returns.

/// Build a minimal `.dmi` PNG carrying a zlib-compressed `Description` chunk.
/// CRCs are left zeroed; `read_dmi_description` does not validate them.
fn png_with_ztxt_description(text: &str) -> Vec<u8> {
    use std::io::Write;
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(text.as_bytes()).expect("zlib write");
    let compressed = enc.finish().expect("zlib finish");

    let mut payload = Vec::new();
    payload.extend_from_slice(b"Description\x00"); // keyword + null separator
    payload.push(0); // zTXt compression method = 0 (zlib)
    payload.extend_from_slice(&compressed);

    let mut png = Vec::from(*b"\x89PNG\r\n\x1a\n");
    let mut chunk = |typ: &[u8], data: &[u8]| {
        let len = u32::try_from(data.len()).expect("chunk len fits u32");
        png.extend_from_slice(&len.to_be_bytes());
        png.extend_from_slice(typ);
        png.extend_from_slice(data);
        png.extend_from_slice(&[0, 0, 0, 0]); // placeholder CRC (unvalidated)
    };
    chunk(b"IHDR", &[0u8; 13]);
    chunk(b"zTXt", &payload);
    chunk(b"IEND", &[]);
    png
}

#[test]
fn dmi_reads_compressed_ztxt_description() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("icons.dmi");
    let desc = "# BEGIN DMI\nversion = 4.0\nstate = \"ztxt_mob\"\n# END DMI\n";
    std::fs::write(&path, png_with_ztxt_description(desc)).expect("write dmi");

    let r = extract_dmi(&path);
    assert!(r.error.is_none());
    assert!(labels(&r).contains(&"\"ztxt_mob\""));
}

#[test]
fn dm_resolvable_include_emits_imports_from() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("helpers.dm"), "/proc/helper()\n").expect("write helper");
    let main = tmp.path().join("main.dm");
    std::fs::write(&main, "#include \"helpers.dm\"\n/proc/run()\n").expect("write main");

    let r = extract_dm(&main);
    let import_edges = edges_with_relation(&r, &["imports", "imports_from"]);
    assert!(
        import_edges
            .iter()
            .any(|e| e.relation == "imports_from" && !e.external),
        "a resolvable include should produce a non-external imports_from edge"
    );
}

#[test]
fn byond_extractors_error_on_missing_file() {
    let missing = Path::new("/nonexistent/graphify/byond/sample");
    assert!(extract_dm(&missing.with_extension("dm")).error.is_some());
    assert!(extract_dmi(&missing.with_extension("dmi")).error.is_some());
    assert!(extract_dmm(&missing.with_extension("dmm")).error.is_some());
    assert!(extract_dmf(&missing.with_extension("dmf")).error.is_some());
}
