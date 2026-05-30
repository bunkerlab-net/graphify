//! 1:1 ports of the semantic type-reference / inheritance test cases added to
//! `graphify-py/tests/test_languages.py` and `test_multilang.py` in v0.8.25.
//!
//! Covers the `references` edges (`field` / `parameter_type` / `return_type` /
//! `generic_arg` contexts), the `embeds` and `mixes_in` relations, and the
//! inherits-vs-implements split across Go, Rust, C, C++, Kotlin, Scala, PHP,
//! Swift, Objective-C, Julia, Fortran, and PowerShell — plus the JS/TS arrow
//! scope guard (#1077) and markdown fenced-block skipping (#1077).

#![allow(clippy::expect_used)]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use graphify_extract::{
    FileResult, extract_c, extract_cpp, extract_fortran, extract_go, extract_js, extract_julia,
    extract_kotlin, extract_markdown, extract_objc, extract_php, extract_powershell, extract_rust,
    extract_scala, extract_swift,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Mirror Python `_normalize_symbol_label`: strip wrapping `()` and a leading `.`.
fn normalize_symbol_label(label: &str) -> String {
    label
        .trim_matches(|c| c == '(' || c == ')')
        .trim_start_matches('.')
        .to_string()
}

/// Mirror Python `_edge_labels`: the set of `(source_label, target_label)` pairs
/// for `relation` (optionally filtered by `context`), using normalized labels.
fn edge_labels(
    result: &FileResult,
    relation: &str,
    context: Option<&str>,
) -> HashSet<(String, String)> {
    let labels: HashMap<&str, String> = result
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), normalize_symbol_label(&n.label)))
        .collect();
    let mut pairs: HashSet<(String, String)> = HashSet::new();
    for e in &result.edges {
        if e.relation != relation {
            continue;
        }
        if let Some(ctx) = context
            && e.context.as_deref() != Some(ctx)
        {
            continue;
        }
        let s = labels
            .get(e.source.as_str())
            .cloned()
            .unwrap_or_else(|| e.source.clone());
        let t = labels
            .get(e.target.as_str())
            .cloned()
            .unwrap_or_else(|| e.target.clone());
        pairs.insert((s, t));
    }
    pairs
}

/// `true` if `(src, tgt)` appears among `relation`/`context` edges.
fn has_edge(
    result: &FileResult,
    relation: &str,
    context: Option<&str>,
    src: &str,
    tgt: &str,
) -> bool {
    edge_labels(result, relation, context).contains(&(src.to_string(), tgt.to_string()))
}

fn labels(result: &FileResult) -> Vec<String> {
    result.nodes.iter().map(|n| n.label.clone()).collect()
}

fn relations(result: &FileResult) -> HashSet<String> {
    result.edges.iter().map(|e| e.relation.clone()).collect()
}

// ── Go (test_multilang.py) ────────────────────────────────────────────────────

#[test]
fn go_embeds_struct_field() {
    let r = extract_go(&fixtures().join("sample.go"));
    assert!(has_edge(
        &r,
        "embeds",
        None,
        "DataProcessor",
        "BaseProcessor"
    ));
}

#[test]
fn go_interface_embedding_emits_embeds() {
    let r = extract_go(&fixtures().join("sample.go"));
    assert!(has_edge(&r, "embeds", None, "ReaderLogger", "Logger"));
}

#[test]
fn go_struct_named_field_emits_field_context() {
    let r = extract_go(&fixtures().join("sample.go"));
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "DataProcessor",
        "Result"
    ));
}

#[test]
fn go_method_parameter_return_contexts() {
    let r = extract_go(&fixtures().join("sample.go"));
    assert!(has_edge(
        &r,
        "references",
        Some("parameter_type"),
        "Build",
        "DataProcessor"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "Build",
        "Result"
    ));
}

// ── Rust (test_multilang.py) ───────────────────────────────────────────────────

#[test]
fn rust_trait_impl_emits_implements() {
    let r = extract_rust(&fixtures().join("sample.rs"));
    assert!(has_edge(
        &r,
        "implements",
        None,
        "DataProcessor",
        "Processor"
    ));
}

#[test]
fn rust_supertrait_emits_inherits() {
    let r = extract_rust(&fixtures().join("sample.rs"));
    assert!(has_edge(&r, "inherits", None, "Logger", "Processor"));
}

#[test]
fn rust_struct_field_emits_field_context() {
    let r = extract_rust(&fixtures().join("sample.rs"));
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "DataProcessor",
        "Result"
    ));
    assert!(!has_edge(
        &r,
        "references",
        Some("field"),
        "DataProcessor",
        "DataProcessor"
    ));
}

#[test]
fn rust_method_parameter_return_and_generic_contexts() {
    let r = extract_rust(&fixtures().join("sample.rs"));
    assert!(has_edge(
        &r,
        "references",
        Some("parameter_type"),
        "build",
        "DataProcessor"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "build",
        "Result"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("generic_arg"),
        "build",
        "DataProcessor"
    ));
}

// ── C (test_languages.py) ──────────────────────────────────────────────────────

#[test]
fn c_parameter_and_return_type_contexts() {
    let r = extract_c(&fixtures().join("sample.c"));
    assert!(has_edge(
        &r,
        "references",
        Some("parameter_type"),
        "make_rect",
        "Rectangle"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "make_rect",
        "Rectangle"
    ));
}

// ── C++ (test_languages.py) ────────────────────────────────────────────────────

#[test]
fn cpp_method_parameter_and_return_type_contexts() {
    let r = extract_cpp(&fixtures().join("sample.cpp"));
    assert!(has_edge(
        &r,
        "references",
        Some("parameter_type"),
        "get",
        "string"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "get",
        "string"
    ));
}

#[test]
fn cpp_field_and_template_argument_contexts() {
    let r = extract_cpp(&fixtures().join("sample.cpp"));
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "HttpClient",
        "string"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "HttpClient",
        "vector"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("generic_arg"),
        "HttpClient",
        "string"
    ));
}

// ── Kotlin (test_languages.py) ─────────────────────────────────────────────────

#[test]
fn kotlin_splits_inherits_and_implements() {
    let r = extract_kotlin(&fixtures().join("sample.kt"));
    assert!(has_edge(
        &r,
        "inherits",
        None,
        "DataProcessor",
        "BaseProcessor"
    ));
    assert!(has_edge(
        &r,
        "implements",
        None,
        "DataProcessor",
        "Loggable"
    ));
}

#[test]
fn kotlin_parameter_return_generic_and_field_contexts() {
    let r = extract_kotlin(&fixtures().join("sample.kt"));
    assert!(has_edge(
        &r,
        "references",
        Some("parameter_type"),
        "run",
        "DataProcessor"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "run",
        "Result"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("generic_arg"),
        "run",
        "DataProcessor"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "DataProcessor",
        "Result"
    ));
}

// ── Scala (test_languages.py) ──────────────────────────────────────────────────

#[test]
fn scala_splits_inherits_and_mixes_in() {
    let r = extract_scala(&fixtures().join("sample.scala"));
    assert!(has_edge(&r, "inherits", None, "HttpClient", "BaseClient"));
    assert!(has_edge(&r, "mixes_in", None, "HttpClient", "Loggable"));
}

#[test]
fn scala_constructor_parameter_field_context() {
    let r = extract_scala(&fixtures().join("sample.scala"));
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "HttpClient",
        "Config"
    ));
}

#[test]
fn scala_val_definition_field_context() {
    let r = extract_scala(&fixtures().join("sample.scala"));
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "HttpClient",
        "Config"
    ));
}

#[test]
fn scala_method_return_type_context() {
    let r = extract_scala(&fixtures().join("sample.scala"));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "create",
        "HttpClient"
    ));
}

// ── PHP (test_languages.py) ────────────────────────────────────────────────────

#[test]
fn php_splits_inherits_implements_mixes_in() {
    let r = extract_php(&fixtures().join("sample.php"));
    assert!(has_edge(
        &r,
        "inherits",
        None,
        "DataProcessor",
        "BaseProcessor"
    ));
    assert!(has_edge(
        &r,
        "implements",
        None,
        "DataProcessor",
        "Loggable"
    ));
    assert!(has_edge(&r, "mixes_in", None, "DataProcessor", "HasName"));
}

#[test]
fn php_property_parameter_and_return_contexts() {
    let r = extract_php(&fixtures().join("sample.php"));
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "DataProcessor",
        "Result"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("parameter_type"),
        "run",
        "DataProcessor"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "run",
        "Result"
    ));
}

// ── Swift (test_languages.py) ──────────────────────────────────────────────────

#[test]
fn swift_protocol_conformance_emits_implements() {
    let r = extract_swift(&fixtures().join("sample.swift"));
    assert!(has_edge(
        &r,
        "implements",
        None,
        "DataProcessor",
        "Processor"
    ));
}

#[test]
fn swift_extension_conformance_emits_implements() {
    let r = extract_swift(&fixtures().join("sample.swift"));
    assert!(has_edge(
        &r,
        "implements",
        None,
        "DataProcessor",
        "Loggable"
    ));
}

#[test]
fn swift_splits_inherits_and_implements() {
    let r = extract_swift(&fixtures().join("sample.swift"));
    assert!(has_edge(
        &r,
        "inherits",
        None,
        "DataProcessor",
        "BaseProcessor"
    ));
    assert!(has_edge(
        &r,
        "implements",
        None,
        "DataProcessor",
        "Processor"
    ));
}

#[test]
fn swift_parameter_return_generic_and_field_contexts() {
    let r = extract_swift(&fixtures().join("sample.swift"));
    assert!(has_edge(
        &r,
        "references",
        Some("parameter_type"),
        "run",
        "DataProcessor"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "run",
        "Result"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("generic_arg"),
        "run",
        "DataProcessor"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "DataProcessor",
        "Result"
    ));
}

// ── Objective-C (test_languages.py) ────────────────────────────────────────────

#[test]
fn objc_splits_inherits_and_implements() {
    let r = extract_objc(&fixtures().join("sample.m"));
    assert!(has_edge(&r, "inherits", None, "Animal", "NSObject"));
    assert!(has_edge(&r, "inherits", None, "Dog", "Animal"));
    assert!(has_edge(&r, "implements", None, "Animal", "SampleDelegate"));
}

#[test]
fn objc_property_type_context() {
    let r = extract_objc(&fixtures().join("sample.m"));
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "Animal",
        "NSString"
    ));
}

// ── Julia (test_languages.py) ──────────────────────────────────────────────────

#[test]
fn julia_abstract_concrete_hierarchy_inherits() {
    let r = extract_julia(&fixtures().join("sample.jl"));
    assert!(has_edge(&r, "inherits", None, "Point", "Shape"));
    assert!(has_edge(&r, "inherits", None, "Circle", "Shape"));
}

#[test]
fn julia_struct_field_type_context() {
    let r = extract_julia(&fixtures().join("sample.jl"));
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "Point",
        "Float64"
    ));
    assert!(has_edge(&r, "references", Some("field"), "Circle", "Point"));
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "Circle",
        "Float64"
    ));
}

// ── Fortran (test_languages.py) ────────────────────────────────────────────────

#[test]
fn fortran_finds_derived_type() {
    let r = extract_fortran(&fixtures().join("sample.f90"));
    assert!(labels(&r).iter().any(|l| l == "point"));
}

#[test]
fn fortran_parameter_and_return_type_contexts() {
    let r = extract_fortran(&fixtures().join("sample.f90"));
    assert!(has_edge(
        &r,
        "references",
        Some("parameter_type"),
        "translate",
        "point"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "origin",
        "point"
    ));
}

// ── PowerShell (test_languages.py) ─────────────────────────────────────────────

#[test]
fn powershell_no_error() {
    let r = extract_powershell(&fixtures().join("sample.ps1"));
    assert!(r.error.is_none(), "{:?}", r.error);
}

#[test]
fn powershell_finds_class_and_method() {
    let r = extract_powershell(&fixtures().join("sample.ps1"));
    let labs = labels(&r);
    assert!(labs.iter().any(|l| l == "DataProcessor"));
    assert!(labs.iter().any(|l| l.contains("Transform")));
}

#[test]
fn powershell_property_field_type_context() {
    let r = extract_powershell(&fixtures().join("sample.ps1"));
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "DataProcessor",
        "string"
    ));
}

#[test]
fn powershell_method_parameter_and_return_type_contexts() {
    let r = extract_powershell(&fixtures().join("sample.ps1"));
    assert!(has_edge(
        &r,
        "references",
        Some("parameter_type"),
        "Transform",
        "string"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "Transform",
        "string"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "Save",
        "void"
    ));
}

// ── JS/TS arrow scope guard #1077 (test_languages.py) ──────────────────────────

#[test]
fn js_local_const_does_not_emit_phantom_node() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let src = "describe('suite', () => {\n  const inner = new Set([1, 2, 3]);\n  let other = [1, 2];\n});\n\nconst moduleConst = new Set([4, 5]);\nexport const exportedConst = { a: 1 };\n";
    let f = tmp.path().join("scope_guard.js");
    std::fs::write(&f, src)?;
    let r = extract_js(&f);
    let labs = labels(&r);
    assert!(
        !labs.iter().any(|l| l == "inner"),
        "phantom arrow-body local 'inner': {labs:?}"
    );
    assert!(
        !labs.iter().any(|l| l == "other"),
        "phantom arrow-body local 'other': {labs:?}"
    );
    assert!(
        labs.iter().any(|l| l == "moduleConst"),
        "module-level const missing: {labs:?}"
    );
    assert!(
        labs.iter().any(|l| l == "exportedConst"),
        "exported const missing: {labs:?}"
    );
    Ok(())
}

#[test]
fn js_module_level_arrow_produces_node_and_call_edges() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let src = "function helper() { return 1; }\nconst handler = () => {\n  helper();\n};\n";
    let f = tmp.path().join("arrows.js");
    std::fs::write(&f, src)?;
    let r = extract_js(&f);
    assert!(
        labels(&r).iter().any(|l| l.contains("handler")),
        "module-level arrow missing"
    );
    assert!(
        has_edge(&r, "calls", None, "handler", "helper"),
        "expected calls edge handler->helper, edges: {:?}",
        r.edges
    );
    Ok(())
}

#[test]
fn ts_local_const_does_not_emit_phantom_node() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let src = "describe('suite', () => {\n  const inner: Set<number> = new Set([1, 2]);\n});\n\nexport const topLevel = { a: 1 };\n";
    let f = tmp.path().join("scope_guard.ts");
    std::fs::write(&f, src)?;
    let r = extract_js(&f);
    let labs = labels(&r);
    assert!(
        !labs.iter().any(|l| l == "inner"),
        "phantom TS arrow-body local 'inner': {labs:?}"
    );
    assert!(
        labs.iter().any(|l| l == "topLevel"),
        "module-level TS const missing: {labs:?}"
    );
    Ok(())
}

// ── Markdown fenced code blocks #1077 (test_languages.py) ──────────────────────

#[test]
fn markdown_skips_fenced_code_blocks() {
    let r = extract_markdown(&fixtures().join("deploy_guide.md"));
    let code_labels: Vec<String> = labels(&r)
        .into_iter()
        .filter(|l| l.starts_with("code:"))
        .collect();
    assert!(
        code_labels.is_empty(),
        "expected no code:* nodes, got: {code_labels:?}"
    );
}

#[test]
fn markdown_contains_edges() {
    let r = extract_markdown(&fixtures().join("deploy_guide.md"));
    assert!(relations(&r).contains("contains"));
    let contains_edges = r.edges.iter().filter(|e| e.relation == "contains").count();
    assert!(
        contains_edges >= 5,
        "expected >= 5 contains edges, got {contains_edges}"
    );
}

#[test]
fn markdown_fenced_heading_not_parsed() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let src =
        "# Real Heading\n\n```bash\n## Not A Heading\necho hello\n```\n\n## Another Real Heading\n";
    let f = tmp.path().join("fenced.md");
    std::fs::write(&f, src)?;
    let r = extract_markdown(&f);
    let labs = labels(&r);
    assert!(
        labs.iter().any(|l| l.contains("Real Heading")),
        "'Real Heading' missing: {labs:?}"
    );
    assert!(
        labs.iter().any(|l| l.contains("Another Real Heading")),
        "'Another Real Heading' missing: {labs:?}"
    );
    assert!(
        !labs.iter().any(|l| l.contains("Not A Heading")),
        "fenced heading wrongly parsed: {labs:?}"
    );
    Ok(())
}

/// Rust divergence from graphify-py: `~~~` fences are honoured too, so a
/// heading-shaped line inside a tilde-fenced block is not parsed as a heading.
#[test]
fn markdown_tilde_fenced_heading_not_parsed() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let src =
        "# Real Heading\n\n~~~bash\n## Not A Heading\necho hi\n~~~\n\n## Another Real Heading\n";
    let f = tmp.path().join("tilde.md");
    std::fs::write(&f, src)?;
    let r = extract_markdown(&f);
    let labs = labels(&r);
    assert!(
        labs.iter().any(|l| l.contains("Real Heading")),
        "'Real Heading' missing: {labs:?}"
    );
    assert!(
        labs.iter().any(|l| l.contains("Another Real Heading")),
        "'Another Real Heading' missing: {labs:?}"
    );
    assert!(
        !labs.iter().any(|l| l.contains("Not A Heading")),
        "tilde-fenced heading wrongly parsed: {labs:?}"
    );
    Ok(())
}
