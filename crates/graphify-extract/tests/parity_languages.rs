//! Coverage tests for per-language extractors.
//!
//! Exercises every public `extract_<lang>` entry point on a small fixture so the
//! tree-sitter walk paths in each language module are executed at least once.

#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};

use graphify_extract::{
    FileResult, extract_apex, extract_astro, extract_blade, extract_c, extract_cpp, extract_csharp,
    extract_csproj, extract_dart, extract_delphi_form, extract_dm, extract_dmf, extract_dmi,
    extract_dmm, extract_elixir, extract_fortran, extract_go, extract_groovy, extract_java,
    extract_julia, extract_kotlin, extract_lazarus_form, extract_lazarus_package, extract_lua,
    extract_markdown, extract_objc, extract_pascal, extract_php, extract_powershell, extract_razor,
    extract_ruby, extract_rust, extract_scala, extract_sln, extract_slnx, extract_sql,
    extract_svelte, extract_swift, extract_verilog, extract_zig, file_stem, make_id,
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

/// Node id of the first node whose exact `label` matches, panicking with the
/// label when absent so endpoint assertions fail loudly on a missing fixture node.
fn php_node_id(result: &FileResult, label: &str) -> String {
    result
        .nodes
        .iter()
        .find(|n| n.label == label)
        .unwrap_or_else(|| panic!("fixture has no node labeled {label:?}"))
        .id
        .clone()
}

/// True when a `source -[relation]-> target` edge exists (compared by node id).
fn php_has_edge(result: &FileResult, source: &str, target: &str, relation: &str) -> bool {
    result
        .edges
        .iter()
        .any(|e| e.source == source && e.target == target && e.relation == relation)
}

#[test]
fn pascal_extractor_produces_nodes() {
    let result = extract_pascal(&fixtures().join("sample.pas"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no pascal nodes");
    assert_no_dangling_edges(&result);
}

#[test]
fn pascal_no_duplicate_edges() {
    // A class method declared in the interface section and defined in the
    // implementation section each used to emit a `method` edge to the same node,
    // doubling method/contains/inherits edges — skewing degree/centrality and
    // tripping the cross-file resolver's single-owner god-node guard (d2d1f68).
    // Edges are now deduped on (source, target, relation).
    let result = extract_pascal(&fixtures().join("sample.pas"));
    let mut seen = std::collections::HashSet::new();
    let dups: Vec<(&str, &str, &str)> = result
        .edges
        .iter()
        .filter_map(|e| {
            let key = (e.source.as_str(), e.target.as_str(), e.relation.as_str());
            if seen.insert(key) { None } else { Some(key) }
        })
        .collect();
    assert!(dups.is_empty(), "duplicate pascal edges: {dups:?}");
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

/// Ports `test_objc_protocol_adopts_protocol` (cd3a376): `@protocol Derived <Base>`
/// emits `implements` Derived->Base — protocol-on-protocol adoption nests under a
/// `protocol_reference_list` node that was previously ignored. Protocol nodes are
/// labeled with angle brackets (`<Base>`).
#[test]
fn objc_protocol_adopts_protocol() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("protocols.m");
    std::fs::write(
        &source,
        "@protocol Base\n- (void)baseMethod;\n@end\n\n\
         @protocol Derived <Base>\n- (void)derivedMethod;\n@end\n",
    )?;
    let result = extract_objc(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let implements = edge_label_pairs(&result, "implements", None);
    assert!(
        implements
            .iter()
            .any(|(s, t)| s == "<Derived>" && t == "<Base>"),
        "<Derived> implements <Base> missing: {implements:?}"
    );
    Ok(())
}

/// Ports the #1475/#1543 dot-syntax + @selector features (0792b41): `self.name`
/// dot-syntax emits an `accesses` edge to a sibling method resolved by EXACT id.
#[test]
fn objc_dot_syntax_property_access() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("Dog.m");
    std::fs::write(
        &source,
        "@implementation Dog\n- (NSString *)name { return @\"Rex\"; }\n\
         - (void)greet { NSLog(@\"%@\", self.name); }\n@end\n",
    )?;
    let result = extract_objc(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let nid2label: std::collections::HashMap<&str, &str> = result
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    let accesses: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "accesses")
        .collect();
    assert_eq!(
        accesses.len(),
        1,
        "expected exactly one access: {accesses:?}"
    );
    assert_eq!(
        nid2label.get(accesses[0].source.as_str()),
        Some(&"-greet"),
        "{accesses:?}"
    );
    assert_eq!(
        nid2label.get(accesses[0].target.as_str()),
        Some(&"-name"),
        "{accesses:?}"
    );
    Ok(())
}

/// Two classes each declaring `-name`: `self.name` in one must resolve only to
/// its OWN class's `-name`, not fan out to the other's (scoped to sibling set).
#[test]
fn objc_dot_syntax_no_fanout_across_classes() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("AB.m");
    std::fs::write(
        &source,
        "@implementation A\n- (NSString *)name { return @\"A\"; }\n\
         - (void)show { NSLog(@\"%@\", self.name); }\n@end\n\
         @implementation B\n- (NSString *)name { return @\"B\"; }\n\
         - (void)show { NSLog(@\"%@\", self.name); }\n@end\n",
    )?;
    let result = extract_objc(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let nid2label: std::collections::HashMap<&str, &str> = result
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    // method nid -> its containing class nid (from `class -> method` edges).
    let method_class: std::collections::HashMap<&str, &str> = result
        .edges
        .iter()
        .filter(|e| e.relation == "method")
        .map(|e| (e.target.as_str(), e.source.as_str()))
        .collect();
    let accesses: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "accesses")
        .collect();
    assert_eq!(
        accesses.len(),
        2,
        "expected 2 scoped accesses: {accesses:?}"
    );
    for e in &accesses {
        assert_eq!(nid2label.get(e.source.as_str()), Some(&"-show"), "{e:?}");
        assert_eq!(nid2label.get(e.target.as_str()), Some(&"-name"), "{e:?}");
        // Scoping: source and target belong to the SAME class (no cross fan-out).
        assert_eq!(
            method_class.get(e.source.as_str()),
            method_class.get(e.target.as_str()),
            "access crossed a class boundary: {e:?}"
        );
    }
    Ok(())
}

/// A property not defined in the current class produces zero `accesses` edges.
#[test]
fn objc_dot_syntax_unresolvable_zero_edges() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("X.m");
    std::fs::write(
        &source,
        "@implementation X\n- (void)run { NSLog(@\"%@\", self.missing); }\n@end\n",
    )?;
    let result = extract_objc(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(
        edge_label_pairs(&result, "accesses", None).is_empty(),
        "unresolvable property must emit no accesses"
    );
    Ok(())
}

/// A substring-colliding sibling must neither be falsely matched nor suppress the
/// real one: `self.name` with both `-name` and `-surname` resolves to `-name` only
/// (EXACT id, not `ends_with`).
#[test]
fn objc_dot_syntax_substring_sibling_exact_match() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("Person.m");
    std::fs::write(
        &source,
        "@implementation Person\n- (NSString *)name { return @\"n\"; }\n\
         - (NSString *)surname { return @\"s\"; }\n\
         - (void)show { NSLog(@\"%@\", self.name); }\n@end\n",
    )?;
    let result = extract_objc(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let accesses = edge_label_pairs(&result, "accesses", None);
    assert!(
        accesses.contains(&("-show".to_string(), "-name".to_string())),
        "self.name must resolve to -name: {accesses:?}"
    );
    assert!(
        !accesses.iter().any(|(_, t)| t == "-surname"),
        "substring sibling -surname must not be matched: {accesses:?}"
    );
    Ok(())
}

/// DIVERGENCE from graphify-py (correctness fix): a non-`self` receiver whose
/// trailing field name collides with a sibling method must NOT emit an access.
/// `other.name` inside a class with `-name` is not a self access.
#[test]
fn objc_dot_syntax_non_self_receiver_no_access() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("Owner.m");
    std::fs::write(
        &source,
        "@implementation Owner\n- (NSString *)name { return @\"o\"; }\n\
         - (void)show:(Owner *)other { NSLog(@\"%@\", other.name); }\n@end\n",
    )?;
    let result = extract_objc(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let accesses = edge_label_pairs(&result, "accesses", None);
    assert!(
        !accesses.iter().any(|(_, t)| t == "-name"),
        "non-self receiver must not fabricate a self access: {accesses:?}"
    );
    Ok(())
}

/// `@selector(fetch)` with exactly one matching method emits a `calls` edge.
#[test]
fn objc_selector_expression_calls_edge() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("Sched.m");
    std::fs::write(
        &source,
        "@implementation Sched\n- (void)fetch { }\n\
         - (void)schedule { [self performSelector:@selector(fetch)]; }\n@end\n",
    )?;
    let result = extract_objc(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let calls = edge_label_pairs(&result, "calls", Some("call"));
    assert!(
        calls.iter().any(|(s, t)| s == "-schedule" && t == "-fetch"),
        "-schedule -> -fetch selector call missing: {calls:?}"
    );
    Ok(())
}

/// `@selector(doThing)` with two `doThing` methods is ambiguous and emits zero
/// `calls` edges (no fan-out).
#[test]
fn objc_selector_ambiguous_no_fanout() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("Dual.m");
    std::fs::write(
        &source,
        "@implementation A\n- (void)doThing { }\n\
         - (void)run { [self performSelector:@selector(doThing)]; }\n@end\n\
         @implementation B\n- (void)doThing { }\n@end\n",
    )?;
    let result = extract_objc(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let calls = edge_label_pairs(&result, "calls", None);
    assert!(
        !calls.iter().any(|(_, t)| t == "-doThing"),
        "ambiguous selector must emit no calls edge: {calls:?}"
    );
    Ok(())
}

/// Regression for the separate calls/accesses dedup sets (DIVERGENCE from
/// graphify-py's relation-blind dedup): a body with BOTH `[self name]` (message)
/// and `self.name` (dot-syntax) to the same sibling must emit a `calls` AND an
/// `accesses` edge — a shared dedup set would drop whichever is visited second.
#[test]
fn objc_calls_and_accesses_coexist_same_target() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("Both.m");
    std::fs::write(
        &source,
        "@implementation Both\n- (NSString *)name { return @\"n\"; }\n\
         - (void)run { [self name]; NSLog(@\"%@\", self.name); }\n@end\n",
    )?;
    let result = extract_objc(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let calls = edge_label_pairs(&result, "calls", None);
    let accesses = edge_label_pairs(&result, "accesses", None);
    assert!(
        calls.iter().any(|(s, t)| s == "-run" && t == "-name"),
        "[self name] must emit a calls edge: {calls:?}"
    );
    assert!(
        accesses.iter().any(|(s, t)| s == "-run" && t == "-name"),
        "self.name must emit a separate accesses edge: {accesses:?}"
    );
    Ok(())
}

/// Ports `test_languages.py::test_objc_resolves_self_method_calls` (#1475): the
/// method-body call pass reads the selector from `method`-field identifiers.
#[test]
fn objc_resolves_self_method_calls() {
    let result = extract_objc(&fixtures().join("sample.m"));
    let nid2label: std::collections::HashMap<&str, &str> = result
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    let calls: Vec<&str> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .filter_map(|e| nid2label.get(e.target.as_str()).copied())
        .collect();
    assert!(calls.iter().any(|t| t.contains("speak")), "{calls:?}");
}

/// Ports `test_languages.py::test_objc_class_method_labeled_with_plus` (#1475).
#[test]
fn objc_class_method_labeled_with_plus() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let p = tmp.path().join("S.m");
    std::fs::write(
        &p,
        "@implementation S\n+ (instancetype)shared { return nil; }\n- (void)go { }\n@end\n",
    )?;
    let r = extract_objc(&p);
    let labels: std::collections::HashSet<String> =
        r.nodes.iter().map(|n| n.label.clone()).collect();
    assert!(labels.contains("+shared"), "{labels:?}");
    assert!(labels.contains("-go"), "{labels:?}");
    Ok(())
}

/// Ports `test_languages.py::test_objc_compound_selector_call_resolves` (#1475).
#[test]
fn objc_compound_selector_call_resolves() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let p = tmp.path().join("V.m");
    std::fs::write(
        &p,
        "@implementation V\n\
         - (void)tableView:(id)tv numberOfRowsInSection:(int)s { }\n\
         - (void)go { [self tableView:nil numberOfRowsInSection:0]; }\n\
         @end\n",
    )?;
    let r = extract_objc(&p);
    let nid2label: std::collections::HashMap<&str, &str> = r
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    let calls: Vec<&str> = r
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .filter_map(|e| nid2label.get(e.target.as_str()).copied())
        .collect();
    assert!(
        calls
            .iter()
            .any(|t| t.contains("tableViewnumberOfRowsInSection")),
        "{calls:?}"
    );
    Ok(())
}

/// Ports `test_languages.py::test_objc_generic_property_type_extracted` (#1475).
#[test]
fn objc_generic_property_type_extracted() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let p = tmp.path().join("M.h");
    std::fs::write(
        &p,
        "@interface M : NSObject\n@property (strong) NSArray<Product *> *items;\n@end\n",
    )?;
    let refs = edge_label_pairs(&extract_objc(&p), "references", Some("field"));
    assert!(refs.contains(&("M".into(), "Product".into())), "{refs:?}");
    assert!(refs.contains(&("M".into(), "NSArray".into())), "{refs:?}");
    Ok(())
}

/// Ports `test_languages.py::test_objc_module_import_edge` (#1475).
#[test]
fn objc_module_import_edge() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let p = tmp.path().join("X.m");
    std::fs::write(
        &p,
        "@import Foundation;\n@import UIKit.UIView;\n@implementation X\n@end\n",
    )?;
    let r = extract_objc(&p);
    let targets: std::collections::HashSet<&str> = r
        .edges
        .iter()
        .filter(|e| e.relation == "imports")
        .map(|e| e.target.as_str())
        .collect();
    assert!(
        targets.contains(make_id(&["Foundation"]).as_str()),
        "{targets:?}"
    );
    assert!(
        targets.contains(make_id(&["UIKit"]).as_str()),
        "{targets:?}"
    );
    Ok(())
}

/// Ports `test_languages.py::test_objc_header_dispatch_routes_objc_not_c` (#1475):
/// an Objective-C `.h` (has `@interface`) routes to the Objective-C extractor; a
/// plain C `.h` stays on the C extractor, so C/C++ headers are never hijacked by
/// the sniff. `get_extractor` is private, so this asserts the routing observably
/// via `extract`: only the Objective-C extractor emits the `@interface` class
/// node, and only the C extractor emits the C function node.
#[test]
fn objc_header_dispatch_routes_objc_not_c() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let objc_h = tmp.path().join("AppDelegate.h");
    std::fs::write(
        &objc_h,
        "@interface AppDelegate : NSObject <UIApplicationDelegate>\n@end\n",
    )?;
    let c_h = tmp.path().join("util.h");
    std::fs::write(&c_h, "int add(int a, int b) { return a + b; }\n")?;

    let objc_out = graphify_extract::extract(&[objc_h], Some(tmp.path()));
    assert!(
        objc_out
            .nodes
            .iter()
            .any(|n| n.get("label").and_then(|v| v.as_str()) == Some("AppDelegate")),
        "ObjC .h must route to the ObjC extractor (no AppDelegate interface node)"
    );
    let c_out = graphify_extract::extract(&[c_h], Some(tmp.path()));
    assert!(
        c_out.nodes.iter().any(|n| n
            .get("label")
            .and_then(|v| v.as_str())
            .is_some_and(|l| l.contains("add"))),
        "C .h must stay on the C extractor (no `add` function node)"
    );
    Ok(())
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

/// Ports `test_fortran_finds_function_call` (b8f41c7): `y = f(x)` function
/// invocations parse as `call_expression` (not `subroutine_call`) and must emit
/// a `calls` edge, resolved against defined procedures so `arr(i)` array
/// indexing can't fabricate a spurious edge.
#[test]
fn fortran_finds_function_call() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("geometry.f90");
    std::fs::write(
        &source,
        "module geometry\ncontains\n  \
         subroutine helper()\n    print *, \"hi\"\n  end subroutine helper\n\n  \
         function double_val(x) result(y)\n    real, intent(in) :: x\n    real :: y\n    \
         y = x * 2.0\n  end function double_val\n\n  \
         subroutine report(radius)\n    real, intent(in) :: radius\n    real :: scaled\n    \
         real :: values(3)\n    call helper()\n    scaled = double_val(radius)\n    \
         scaled = values(1)\n    print *, scaled\n  \
         end subroutine report\nend module geometry\n",
    )?;
    let result = extract_fortran(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let calls = edge_label_pairs(&result, "calls", None);
    // `scaled = double_val(radius)` — function invocation (`call_expression`).
    assert!(
        calls
            .iter()
            .any(|(s, t)| s.contains("report") && t.contains("double_val")),
        "report() -> double_val() calls edge missing: {calls:?}"
    );
    // `call helper()` — subroutine_call still emits after the traversal rewrite.
    assert!(
        calls
            .iter()
            .any(|(s, t)| s.contains("report") && t.contains("helper")),
        "report() -> helper() subroutine call missing: {calls:?}"
    );
    // Attribution: the enclosing module must NOT also receive the subroutine's
    // calls (nested-scope calls belong only to their own scope).
    assert!(
        !calls
            .iter()
            .any(|(s, t)| s.contains("geometry")
                && (t.contains("double_val") || t.contains("helper"))),
        "module must not receive nested subroutine calls: {calls:?}"
    );
    // Array indexing (`values(1)`) shares `name(...)` syntax with a call but the
    // array variable resolves to no procedure node, so the `seen_ids` guard must
    // suppress a spurious `calls` edge to it.
    assert!(
        !calls.iter().any(|(_, t)| t.contains("values")),
        "array access must not fabricate a calls edge: {calls:?}"
    );
    Ok(())
}

#[test]
fn elixir_extractor_produces_nodes() {
    let result = extract_elixir(&fixtures().join("sample.ex"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no elixir nodes");
    assert_no_dangling_edges(&result);
}

/// Ports `test_elixir_multi_alias_expands` (f2ea6a6): `alias Foo.{Bar, Baz}` (a
/// `dot` node + trailing `tuple`) emits one imports edge per expanded module,
/// including a multi-segment prefix; the single form stays unchanged.
#[test]
fn elixir_multi_alias_expands() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("user.ex");
    std::fs::write(
        &source,
        "defmodule MyApp.Accounts.User do\n  \
         alias MyApp.Repo\n  \
         alias MyApp.Schemas.{Account, Token}\nend\n",
    )?;
    let result = extract_elixir(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let targets: Vec<&str> = result
        .edges
        .iter()
        .filter(|e| e.relation == "imports")
        .map(|e| e.target.as_str())
        .collect();
    // Multi-segment brace prefix expands to one target per member.
    assert!(
        targets.contains(&make_id(&["MyApp.Schemas.Account"]).as_str()),
        "MyApp.Schemas.Account import missing: {targets:?}"
    );
    assert!(
        targets.contains(&make_id(&["MyApp.Schemas.Token"]).as_str()),
        "MyApp.Schemas.Token import missing: {targets:?}"
    );
    // Single-alias form is unchanged.
    assert!(
        targets.contains(&make_id(&["MyApp.Repo"]).as_str()),
        "single alias MyApp.Repo missing: {targets:?}"
    );
    Ok(())
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

/// Ports `test_powershell_class_base_type_emits_inherits_edge` (a129ff2):
/// `class Circle : Shape` emits `inherits`; with multiple bases the first is
/// `inherits` and the rest `implements` (no syntactic base/interface split).
#[test]
fn powershell_class_base_types_emit_inheritance() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("shapes.ps1");
    std::fs::write(
        &source,
        "class Shape {\n}\nclass IDrawable {\n}\nclass Circle : Shape, IDrawable {\n}\n",
    )?;
    let result = extract_powershell(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let inherits = edge_label_pairs(&result, "inherits", None);
    let implements = edge_label_pairs(&result, "implements", None);
    // First base -> inherits.
    assert!(
        inherits.iter().any(|(s, t)| s == "Circle" && t == "Shape"),
        "Circle inherits Shape missing: {inherits:?}"
    );
    // Subsequent bases -> implements.
    assert!(
        implements
            .iter()
            .any(|(s, t)| s == "Circle" && t == "IDrawable"),
        "Circle implements IDrawable missing: {implements:?}"
    );
    Ok(())
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

/// Ports `test_systemverilog_qualified_field_references` (297075c): class
/// properties with leading qualifiers (rand/local/protected/...) must still emit
/// `references[field]` edges — `rand Config x;` (three tokens) previously failed
/// the two-token `<type> <name>;` shape and dropped its type reference.
#[test]
fn systemverilog_qualified_field_references() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("proc.sv");
    std::fs::write(
        &source,
        "class Config;\nendclass\n\nclass BaseProcessor;\nendclass\n\n\
         class DataProcessor;\n  rand Config m_cfg;\n  \
         protected BaseProcessor m_parent;\nendclass\n",
    )?;
    let result = extract_verilog(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let field_refs = edge_label_pairs(&result, "references", Some("field"));
    assert!(
        field_refs
            .iter()
            .any(|(s, t)| s == "DataProcessor" && t == "Config"),
        "rand-qualified field dropped: {field_refs:?}"
    );
    assert!(
        field_refs
            .iter()
            .any(|(s, t)| s == "DataProcessor" && t == "BaseProcessor"),
        "protected-qualified field dropped: {field_refs:?}"
    );
    Ok(())
}

#[test]
fn rust_extractor_produces_nodes() {
    let result = extract_rust(&fixtures().join("sample.rs"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no rust nodes");
    assert_no_dangling_edges(&result);
}

/// Ports `test_rust_tuple_struct_field_references` (7eb847b): tuple-struct
/// positional field types nest under `ordered_field_declaration_list` with no
/// `field_declaration` wrapper, and must still emit `field`/`generic_arg` refs.
#[test]
fn rust_tuple_struct_field_references() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("lib.rs");
    std::fs::write(
        &source,
        "struct Graph {}\nstruct DataProcessor {}\nstruct Wrapper<T> { value: T }\n\
         struct GraphPair(Graph, Wrapper<DataProcessor>);\n",
    )?;
    let result = extract_rust(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let fields = edge_label_pairs(&result, "references", Some("field"));
    let generics = edge_label_pairs(&result, "references", Some("generic_arg"));
    assert!(
        fields.iter().any(|(s, t)| s == "GraphPair" && t == "Graph"),
        "tuple-struct field reference missing: {fields:?}"
    );
    assert!(
        fields
            .iter()
            .any(|(s, t)| s == "GraphPair" && t == "Wrapper"),
        "tuple-struct generic container reference missing: {fields:?}"
    );
    assert!(
        generics
            .iter()
            .any(|(s, t)| s == "GraphPair" && t == "DataProcessor"),
        "tuple-struct generic_arg reference missing: {generics:?}"
    );
    Ok(())
}

/// Divergence from graphify-py (7eb847b): the Python allowlist for tuple
/// positional field types omits `pointer_type`/`slice_type`, silently dropping
/// them. Recursing the collector (which handles both) fixes that gap — a
/// raw-pointer positional field still emits a `field` reference.
#[test]
fn rust_tuple_struct_pointer_field_reference() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("ptr.rs");
    std::fs::write(&source, "struct Config {}\nstruct Holder(*const Config);\n")?;
    let result = extract_rust(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let fields = edge_label_pairs(&result, "references", Some("field"));
    assert!(
        fields.iter().any(|(s, t)| s == "Holder" && t == "Config"),
        "raw-pointer positional field must emit a field ref: {fields:?}"
    );
    Ok(())
}

/// Ports `test_rust_enum_variant_references` (674184d), covering all three
/// variant shapes: unit (no refs), tuple payload (`field` + nested `generic_arg`),
/// and struct payload (`field`). All were dropped before the enum path existed.
#[test]
fn rust_enum_variant_field_references() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("events.rs");
    std::fs::write(
        &source,
        "struct Foo {}\nstruct Bar {}\nstruct Baz {}\nstruct Wrapper<T> { value: T }\n\
         enum E {\n    Unit,\n    Tuple(Foo, Wrapper<Bar>),\n    Named { value: Baz },\n}\n",
    )?;
    let result = extract_rust(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let fields = edge_label_pairs(&result, "references", Some("field"));
    let generics = edge_label_pairs(&result, "references", Some("generic_arg"));
    // Tuple variant: direct payload -> field, nested generic arg -> generic_arg.
    assert!(
        fields.iter().any(|(s, t)| s == "E" && t == "Foo"),
        "tuple-variant field reference missing: {fields:?}"
    );
    assert!(
        fields.iter().any(|(s, t)| s == "E" && t == "Wrapper"),
        "tuple-variant generic container reference missing: {fields:?}"
    );
    assert!(
        generics.iter().any(|(s, t)| s == "E" && t == "Bar"),
        "tuple-variant generic_arg reference missing: {generics:?}"
    );
    // Struct variant: named field payload -> field.
    assert!(
        fields.iter().any(|(s, t)| s == "E" && t == "Baz"),
        "struct-variant field reference missing: {fields:?}"
    );
    Ok(())
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
fn markdown_nested_fence_does_not_leak_inner_heading() -> Result<(), Box<dyn std::error::Error>> {
    // A four-backtick fence wraps a three-backtick block whose contents include
    // a line that looks like a heading. Per CommonMark, the inner ``` is too
    // short to close the outer ```` fence, so the `#` line stays inside the
    // code block and must not become a phantom heading node.
    let src = "# Real Heading\n\
````markdown\n\
```python\n\
# not a heading, just a comment\n\
```\n\
````\n\
## Another Real Heading\n";
    let tmp = tempfile::tempdir()?;
    let path = tmp.path().join("nested.md");
    std::fs::write(&path, src)?;
    let result = extract_markdown(&path);
    assert!(result.error.is_none(), "{:?}", result.error);
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"Real Heading"), "labels: {labels:?}");
    assert!(
        labels.contains(&"Another Real Heading"),
        "labels: {labels:?}"
    );
    assert!(
        !labels.iter().any(|l| l.contains("not a heading")),
        "inner code-block line leaked as a heading: {labels:?}"
    );
    Ok(())
}

#[test]
fn markdown_closing_fence_with_info_string_does_not_close() -> Result<(), Box<dyn std::error::Error>>
{
    // A closing fence must carry only optional whitespace after its run; a line
    // with an info string (```text) is not a valid close (CommonMark), so the
    // block stays open and its `#` lines must not become heading nodes.
    let src = "# Title\n\
```\n\
# inside code\n\
```text\n\
# still inside code\n\
```\n\
## End\n";
    let tmp = tempfile::tempdir()?;
    let path = tmp.path().join("infoclose.md");
    std::fs::write(&path, src)?;
    let result = extract_markdown(&path);
    assert!(result.error.is_none(), "{:?}", result.error);
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"Title"), "labels: {labels:?}");
    assert!(labels.contains(&"End"), "labels: {labels:?}");
    assert!(
        !labels.iter().any(|l| l.contains("inside code")),
        "code-block line leaked as a heading: {labels:?}"
    );
    Ok(())
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

/// Ports the C++ base-class template-arg fix (21bcb43): a templated base like
/// `Connection<HttpClient>` emits both the `inherits` edge to the container and
/// a `generic_arg` reference to the type argument, mirroring the Java behaviour.
#[test]
fn cpp_base_class_template_args_emit_generic_arg_refs() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("pool.cpp");
    std::fs::write(
        &source,
        "template <typename T> class Connection {};\nclass HttpClient {};\n\
         class PooledClient : public Connection<HttpClient> {};\n",
    )?;
    let result = extract_cpp(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let inherits = edge_label_pairs(&result, "inherits", None);
    let generics = edge_label_pairs(&result, "references", Some("generic_arg"));
    assert!(
        inherits
            .iter()
            .any(|(s, t)| s == "PooledClient" && t == "Connection"),
        "PooledClient inherits Connection missing: {inherits:?}"
    );
    assert!(
        generics
            .iter()
            .any(|(s, t)| s == "PooledClient" && t == "HttpClient"),
        "PooledClient->HttpClient generic_arg missing: {generics:?}"
    );
    Ok(())
}

// ── CUDA (.cu/.cuh route through the C++ extractor) ──────────────────────────

#[test]
fn cuda_no_error() {
    let r = extract_cpp(&fixtures().join("sample.cu"));
    assert!(r.error.is_none(), "{:?}", r.error);
}

#[test]
fn cuda_finds_kernel_and_device_functions() {
    let r = extract_cpp(&fixtures().join("sample.cu"));
    let labels = labels(&r);
    assert!(labels.iter().any(|l| l.contains("saxpy")), "{labels:?}"); // __global__ kernel
    assert!(labels.iter().any(|l| l.contains("dot")), "{labels:?}"); // __device__ function
}

#[test]
fn cuda_finds_struct() {
    let r = extract_cpp(&fixtures().join("sample.cu"));
    assert!(labels(&r).iter().any(|l| l.contains("Vec3")));
}

#[test]
fn cuda_finds_includes() {
    let r = extract_cpp(&fixtures().join("sample.cu"));
    assert!(relations(&r).contains("imports"));
}

#[test]
fn cuda_host_call_edges() {
    let r = extract_cpp(&fixtures().join("sample.cu"));
    let calls = calls(&r);
    assert!(
        calls.contains(&("host_norm()".to_string(), "dot()".to_string())),
        "{calls:?}"
    );
    assert!(
        calls.contains(&("main()".to_string(), "host_norm()".to_string())),
        "{calls:?}"
    );
}

/// Ports `test_languages.py::test_metal_no_error` (#1480): Metal Shading Language
/// is C++14, so `.metal` routes through the C++ extractor (like CUDA `.cu`).
#[test]
fn metal_no_error() {
    let r = extract_cpp(&fixtures().join("sample.metal"));
    assert!(r.error.is_none(), "{:?}", r.error);
}

/// Ports `test_languages.py::test_metal_finds_kernel_function_and_struct` (#1480).
#[test]
fn metal_finds_kernel_function_and_struct() {
    let r = extract_cpp(&fixtures().join("sample.metal"));
    let labels = labels(&r);
    assert!(labels.iter().any(|l| l.contains("Vec3")), "{labels:?}");
    assert!(labels.iter().any(|l| l.contains("dot3")), "{labels:?}");
    assert!(labels.iter().any(|l| l.contains("saxpy")), "{labels:?}");
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

/// Ports `test_csharp_property_type_references_have_field_context` (bb5e519):
/// C# auto-properties emit `references[field]` for the property type, plus
/// `generic_arg` for type arguments — `List<Processor>` yields both.
#[test]
fn csharp_property_type_references_have_field_context() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("DataProcessor.cs");
    std::fs::write(
        &source,
        "using System.Collections.Generic;\nclass Processor {}\nclass DataProcessor {\n    \
         public Processor Owner { get; set; }\n    \
         public List<Processor> Workers { get; set; }\n}\n",
    )?;
    let result = extract_csharp(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let field_refs = edge_label_pairs(&result, "references", Some("field"));
    let generic_refs = edge_label_pairs(&result, "references", Some("generic_arg"));
    // `public Processor Owner { get; set; }` — property type -> field ref.
    assert!(
        field_refs
            .iter()
            .any(|(s, t)| s == "DataProcessor" && t == "Processor"),
        "DataProcessor->Processor field missing: {field_refs:?}"
    );
    // `public List<Processor> Workers { get; set; }` — the List container -> field.
    assert!(
        field_refs
            .iter()
            .any(|(s, t)| s == "DataProcessor" && t == "List"),
        "DataProcessor->List field missing: {field_refs:?}"
    );
    // ...and the generic argument -> generic_arg.
    assert!(
        generic_refs
            .iter()
            .any(|(s, t)| s == "DataProcessor" && t == "Processor"),
        "DataProcessor->Processor generic_arg missing: {generic_refs:?}"
    );
    Ok(())
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

/// Ports `test_languages.py::test_java_generic_parents_include_type_argument_references`
/// (#1510): a generic parent emits the inherits/implements edge to the base AND a
/// `generic_arg` reference for each type argument.
#[test]
fn java_generic_parents_include_type_argument_references() -> Result<(), Box<dyn std::error::Error>>
{
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("GenericParents.java");
    std::fs::write(
        &source,
        "class Dependency {}\n\
         interface Event {}\n\
         class Base<T> {}\n\
         interface Handler<T> {}\n\
         interface DerivedHandler extends Handler<Event> {}\n\
         class Service extends Base<Dependency> implements Handler<Event> {}\n",
    )?;
    let result = extract_java(&source);
    let inherits = edge_label_pairs(&result, "inherits", None);
    let implements = edge_label_pairs(&result, "implements", None);
    let refs = edge_label_pairs(&result, "references", Some("generic_arg"));
    assert!(
        inherits.contains(&("Service".into(), "Base".into())),
        "{inherits:?}"
    );
    assert!(
        implements.contains(&("Service".into(), "Handler".into())),
        "{implements:?}"
    );
    assert!(
        refs.contains(&("Service".into(), "Dependency".into())),
        "{refs:?}"
    );
    assert!(
        refs.contains(&("Service".into(), "Event".into())),
        "{refs:?}"
    );
    assert!(
        inherits.contains(&("DerivedHandler".into(), "Handler".into())),
        "{inherits:?}"
    );
    assert!(
        refs.contains(&("DerivedHandler".into(), "Event".into())),
        "{refs:?}"
    );
    Ok(())
}

/// Ports `test_languages.py::test_java_field_type_references_have_field_context` (#1485).
#[test]
fn java_field_type_references_have_field_context() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("Fields.java");
    std::fs::write(
        &source,
        "class PaymentGateway {}\n\
         class Handler {}\n\
         class CheckoutService {\n\
         \x20   PaymentGateway gateway;\n\
         \x20   List<Handler> handlers;\n\
         }\n",
    )?;
    let result = extract_java(&source);
    let fields = edge_label_pairs(&result, "references", Some("field"));
    let generics = edge_label_pairs(&result, "references", Some("generic_arg"));
    assert!(
        fields.contains(&("CheckoutService".into(), "PaymentGateway".into())),
        "{fields:?}"
    );
    assert!(
        generics.contains(&("CheckoutService".into(), "Handler".into())),
        "{generics:?}"
    );
    Ok(())
}

/// Ports `test_languages.py::test_java_type_parameters_do_not_emit_references` (#1518):
/// `<T>` / `<U>` / `<V>` are type variables, not real types — no `references` edge
/// and no sourceless stub node, while real types (Base, Payload) survive.
#[test]
fn java_type_parameters_do_not_emit_references() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("TypeParameters.java");
    std::fs::write(
        &source,
        "class Payload {}\n\
         class Base<X> {}\n\
         class Box<T> extends Base<T> {\n\
         \x20   T value;\n\
         \x20   List<T> values;\n\
         \x20   <U> U convert(T input, List<U> mapped, List<Payload> retained) {\n\
         \x20       return null;\n\
         \x20   }\n\
         \x20   <V> Box(V value) {}\n\
         }\n",
    )?;
    let result = extract_java(&source);
    let references = edge_label_pairs(&result, "references", None);
    assert!(
        !references
            .iter()
            .any(|(_, t)| matches!(t.as_str(), "T" | "U" | "V")),
        "type-parameter references leaked: {references:?}"
    );
    assert!(
        !result
            .nodes
            .iter()
            .any(|n| matches!(n.label.as_str(), "T" | "U" | "V") && n.source_file.is_empty()),
        "sourceless type-parameter stub node leaked"
    );
    let inherits = edge_label_pairs(&result, "inherits", None);
    assert!(
        inherits.contains(&("Box".into(), "Base".into())),
        "{inherits:?}"
    );
    let generics = edge_label_pairs(&result, "references", Some("generic_arg"));
    assert!(
        generics.contains(&("convert".into(), "Payload".into())),
        "{generics:?}"
    );
    Ok(())
}

/// Ports `test_languages.py::test_java_record_component_type_references` (#1519):
/// a record's header components emit `field` / `generic_arg` references like fields.
#[test]
fn java_record_component_type_references() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("RecordComponents.java");
    std::fs::write(
        &source,
        "class Payload {}\n\
         class Item {}\n\
         class Attachment {}\n\
         record Order(Payload payload, List<Item> items, int count, Attachment... attachments) {}\n",
    )?;
    let result = extract_java(&source);
    let fields = edge_label_pairs(&result, "references", Some("field"));
    let generics = edge_label_pairs(&result, "references", Some("generic_arg"));
    assert!(
        fields.contains(&("Order".into(), "Payload".into())),
        "{fields:?}"
    );
    // `List` is a java.util library type: skipped as noise (92edf78), so only its
    // user-type generic argument (`Item`) survives, not the container itself.
    let all_refs = edge_label_pairs(&result, "references", None);
    assert!(
        !all_refs.contains(&("Order".into(), "List".into())),
        "List (library type) must not be a references target: {all_refs:?}"
    );
    assert!(
        generics.contains(&("Order".into(), "Item".into())),
        "{generics:?}"
    );
    assert!(
        fields.contains(&("Order".into(), "Attachment".into())),
        "{fields:?}"
    );
    Ok(())
}

/// Ports `test_languages.py::test_java_record_components_skip_type_parameters` (#1519):
/// a generic record's type parameters are skipped in its component references.
#[test]
fn java_record_components_skip_type_parameters() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("GenericRecord.java");
    std::fs::write(
        &source,
        "class Payload {}\n\
         class Box<X> {}\n\
         record Batch<T>(T value, Box<T> boxed, Box<Payload> retained) {}\n",
    )?;
    let result = extract_java(&source);
    let references = edge_label_pairs(&result, "references", None);
    assert!(
        !references.contains(&("Batch".into(), "T".into())),
        "{references:?}"
    );
    assert!(
        !result
            .nodes
            .iter()
            .any(|n| n.label == "T" && n.source_file.is_empty()),
        "sourceless T stub node leaked"
    );
    let fields = edge_label_pairs(&result, "references", Some("field"));
    let generics = edge_label_pairs(&result, "references", Some("generic_arg"));
    assert!(
        fields.contains(&("Batch".into(), "Box".into())),
        "{fields:?}"
    );
    assert!(
        generics.contains(&("Batch".into(), "Payload".into())),
        "{generics:?}"
    );
    Ok(())
}

/// Ports `test_java_builtin_library_types_not_emitted_as_references` (92edf78):
/// ubiquitous java.lang/util/... types (String, List, Map, ...) never resolve to
/// a project node, so they must not be emitted as `references` targets.
#[test]
fn java_builtin_library_types_not_emitted_as_references() -> Result<(), Box<dyn std::error::Error>>
{
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("Svc.java");
    std::fs::write(
        &source,
        "package com.app;\nimport java.util.List;\nimport java.util.Map;\n\
         public class Svc {\n    private String name;\n    private List<Integer> ids;\n    \
         public Map<String, Object> lookup(Long id) { return null; }\n    \
         public java.util.Optional<Boolean> flag() { return null; }\n}\n",
    )?;
    let result = extract_java(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let refs = edge_label_pairs(&result, "references", None);
    for builtin in [
        "String", "Integer", "Map", "Object", "Long", "List", "Optional", "Boolean",
    ] {
        assert!(
            !refs.iter().any(|(_, t)| t == builtin),
            "builtin/library type {builtin} must not be a references target: {refs:?}"
        );
    }
    Ok(())
}

/// Ports `test_java_user_types_still_emit_references` (92edf78): guard against
/// over-skipping — a user type sharing the field/return shape still resolves.
#[test]
fn java_user_types_still_emit_references() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("OrderSvc.java");
    std::fs::write(
        &source,
        "package com.app;\npublic class OrderSvc {\n    \
         private java.util.List<OrderDto> orders;\n    \
         public OrderDto first() { return null; }\n}\n",
    )?;
    let result = extract_java(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let refs = edge_label_pairs(&result, "references", None);
    assert!(
        refs.iter().any(|(_, t)| t == "OrderDto"),
        "user type OrderDto must still emit references: {refs:?}"
    );
    Ok(())
}

/// Ports `test_languages.py::test_java_type_annotations_have_attribute_context` (#1487).
#[test]
fn java_type_annotations_have_attribute_context() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("TypeAnnotations.java");
    std::fs::write(
        &source,
        "@Service\n@Entity(name = \"checkout\")\nclass CheckoutService {}\n",
    )?;
    let result = extract_java(&source);
    let refs = edge_label_pairs(&result, "references", Some("attribute"));
    assert!(
        refs.contains(&("CheckoutService".into(), "Service".into())),
        "{refs:?}"
    );
    assert!(
        refs.contains(&("CheckoutService".into(), "Entity".into())),
        "{refs:?}"
    );
    Ok(())
}

/// Ports `test_languages.py::test_java_enum_and_annotation_declarations_are_type_nodes`
/// (#1512): enum and `@interface` declarations become real type nodes.
#[test]
fn java_enum_and_annotation_declarations_are_type_nodes() -> Result<(), Box<dyn std::error::Error>>
{
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("TypeDeclarations.java");
    std::fs::write(
        &source,
        "enum PaymentStatus { PENDING, PAID }\n\
         @interface Audited {}\n\
         class Order { PaymentStatus status; }\n\
         @Audited class CheckoutService {}\n",
    )?;
    let result = extract_java(&source);
    let contains = edge_label_pairs(&result, "contains", None);
    assert!(
        contains.contains(&("TypeDeclarations.java".into(), "PaymentStatus".into())),
        "{contains:?}"
    );
    assert!(
        contains.contains(&("TypeDeclarations.java".into(), "Audited".into())),
        "{contains:?}"
    );
    assert!(
        edge_label_pairs(&result, "references", Some("field"))
            .contains(&("Order".into(), "PaymentStatus".into()))
    );
    assert!(
        edge_label_pairs(&result, "references", Some("attribute"))
            .contains(&("CheckoutService".into(), "Audited".into()))
    );
    let sf = source.to_string_lossy();
    for label in ["PaymentStatus", "Audited"] {
        let def = result
            .nodes
            .iter()
            .find(|n| n.label == label)
            .unwrap_or_else(|| panic!("no def node for {label}"));
        assert_eq!(def.source_file, sf, "{label} must be a source-backed def");
    }
    Ok(())
}

/// Ports `test_languages.py::test_java_enum_constants_have_case_of_edge` (cf36d10):
/// Java enum constants become nodes with a `case_of` edge to the enum.
#[test]
fn java_enum_constants_have_case_of_edge() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("ErrorCode.java");
    std::fs::write(
        &source,
        "enum ErrorCode {\n    OK(0),\n    GAME_DONE(1001);\n    private final int code;\n    \
         ErrorCode(int code) { this.code = code; }\n}\n",
    )?;
    let result = extract_java(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let ls = labels(&result);
    assert!(ls.contains(&"OK"), "OK constant node missing: {ls:?}");
    assert!(
        ls.contains(&"GAME_DONE"),
        "GAME_DONE constant node missing: {ls:?}"
    );
    let case_of = edge_label_pairs(&result, "case_of", None);
    assert!(
        case_of.contains(&("ErrorCode".into(), "OK".into())),
        "ErrorCode->OK case_of missing: {case_of:?}"
    );
    assert!(
        case_of.contains(&("ErrorCode".into(), "GAME_DONE".into())),
        "ErrorCode->GAME_DONE case_of missing: {case_of:?}"
    );
    Ok(())
}

#[test]
fn groovy_extractor_produces_nodes() {
    let result = extract_groovy(&fixtures().join("sample.groovy"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no groovy nodes");
    assert_no_dangling_edges(&result);
}

/// Ports `test_groovy_extends_edge` + `test_groovy_implements_edge` (64a6093):
/// tree-sitter-groovy exposes `superclass`/`interfaces` like Java, so Groovy
/// `extends`/`implements` must emit `inherits`/`implements` edges (previously
/// gated Java-only and silently dropped).
#[test]
fn groovy_extends_and_implements_edges() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("ExtendedService.groovy");
    std::fs::write(
        &source,
        "class SampleService {}\ninterface Resettable {\n    void reset()\n}\n\
         class ExtendedService extends SampleService implements Resettable {\n    \
         void reset() {}\n}\n",
    )?;
    let result = extract_groovy(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let inherits = edge_label_pairs(&result, "inherits", None);
    let implements = edge_label_pairs(&result, "implements", None);
    assert!(
        inherits
            .iter()
            .any(|(s, t)| s == "ExtendedService" && t == "SampleService"),
        "ExtendedService inherits SampleService missing: {inherits:?}"
    );
    assert!(
        implements
            .iter()
            .any(|(s, t)| s == "ExtendedService" && t == "Resettable"),
        "ExtendedService implements Resettable missing: {implements:?}"
    );
    Ok(())
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

/// Ports `test_languages.py::test_kotlin_enum_entries_have_case_of_edge`
/// (#1700 Kotlin half, #1738): Kotlin enum entries become nodes with a
/// `case_of` edge to the enum (needs the `enum_class_body` body fallback).
#[test]
fn kotlin_enum_entries_have_case_of_edge() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("Direction.kt");
    std::fs::write(
        &source,
        "enum class Direction {\n    NORTH,\n    SOUTH\n}\n",
    )?;
    let result = extract_kotlin(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let ls = labels(&result);
    assert!(ls.contains(&"NORTH"), "NORTH entry node missing: {ls:?}");
    assert!(ls.contains(&"SOUTH"), "SOUTH entry node missing: {ls:?}");
    let case_of = edge_label_pairs(&result, "case_of", None);
    assert!(
        case_of.contains(&("Direction".into(), "NORTH".into())),
        "Direction->NORTH case_of missing: {case_of:?}"
    );
    assert!(
        case_of.contains(&("Direction".into(), "SOUTH".into())),
        "Direction->SOUTH case_of missing: {case_of:?}"
    );
    Ok(())
}

/// Ports the Kotlin interface-delegation fix (9b04022): `class Foo : Bar by r`
/// wraps the delegated interface in an `explicit_delegation` node, which must
/// still emit an `implements` edge to the interface.
#[test]
fn kotlin_interface_delegation_emits_implements_edge() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("Delegate.kt");
    std::fs::write(&source, "interface Repo\nclass Foo(r: Repo) : Repo by r\n")?;
    let result = extract_kotlin(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let implements = edge_label_pairs(&result, "implements", None);
    assert!(
        implements.iter().any(|(s, t)| s == "Foo" && t == "Repo"),
        "Foo implements Repo (by delegation) missing: {implements:?}"
    );
    Ok(())
}

#[test]
fn scala_extractor_produces_nodes() {
    let result = extract_scala(&fixtures().join("sample.scala"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no scala nodes");
    assert_no_dangling_edges(&result);
}

/// Ports the Scala var-field regression (67b4525): a `var b: Repo` field's type
/// reference is emitted (previously only `val` fields were handled).
#[test]
fn scala_var_field_emits_type_reference() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("Service.scala");
    std::fs::write(
        &source,
        "class Repo\nclass Service {\n  var repo: Repo = null\n}\n",
    )?;
    let result = extract_scala(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let refs = edge_label_pairs(&result, "references", None);
    assert!(
        refs.iter().any(|(s, t)| s == "Service" && t == "Repo"),
        "Service->Repo var-field type reference missing: {refs:?}"
    );
    Ok(())
}

/// Ports the Ruby-superclass regression (a19b9e9): `class Dog < Animal` emits an
/// `inherits` edge (previously every Ruby inherits edge was silently dropped).
#[test]
fn ruby_superclass_emits_inherits_edge() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("dog.rb");
    std::fs::write(&source, "class Animal\nend\nclass Dog < Animal\nend\n")?;
    let result = extract_ruby(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let inherits = edge_label_pairs(&result, "inherits", None);
    assert!(
        inherits.iter().any(|(s, t)| s == "Dog" && t == "Animal"),
        "Dog->Animal inherits edge missing: {inherits:?}"
    );
    Ok(())
}

/// Ports `test_languages.py::test_julia_qualified_and_relative_imports` (984a6a8):
/// a qualified `using Base.Threads` emits an `imports` edge (previously only bare
/// identifiers were handled, so scoped/relative forms were silently dropped).
#[test]
fn julia_qualified_import_emits_edge() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("mod.jl");
    std::fs::write(&source, "using Base.Threads\n")?;
    let result = extract_julia(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let import_targets: Vec<String> = result
        .edges
        .iter()
        .filter(|e| e.relation == "imports")
        .map(|e| e.target.clone())
        .collect();
    assert!(
        import_targets.iter().any(|t| t.contains("base_threads")),
        "qualified import Base.Threads missing: {import_targets:?}"
    );
    Ok(())
}

/// The imported-symbol node must be deduplicated: importing the same module in
/// two statements emits one node, not a duplicate with the same id (CodeRabbit
/// follow-up - the `seen_ids` insert previously did not guard the node push).
#[test]
fn julia_repeated_import_emits_one_node() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("mod.jl");
    std::fs::write(&source, "using Base.Threads\nusing Base.Threads\n")?;
    let result = extract_julia(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let threads_nodes = result
        .nodes
        .iter()
        .filter(|n| n.id.contains("base_threads"))
        .count();
    assert_eq!(
        threads_nodes,
        1,
        "repeated import must emit exactly one node: {:?}",
        result.nodes.iter().map(|n| &n.id).collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn php_extractor_produces_nodes() {
    let result = extract_php(&fixtures().join("sample.php"));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.nodes.is_empty(), "no php nodes");
    assert_no_dangling_edges(&result);
}

/// Ports `test_php.py::test_php_constructor_property_promotion_contexts` (51f805e):
/// a PHP 8 promoted ctor param (`__construct(private Repo $r)`) emits both a
/// `parameter_type` (on the ctor) and a `field` (on the class) reference; a
/// non-promoted param leaks no `field` edge.
#[test]
fn php_constructor_property_promotion_contexts() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("Service.php");
    std::fs::write(
        &source,
        "<?php\nclass Repo {}\nclass Logger {}\nclass Service {\n    \
         public function __construct(private Repo $repo, Logger $logger) {}\n}\n",
    )?;
    let result = extract_php(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let field_refs = edge_label_pairs(&result, "references", Some("field"));
    let param_refs = edge_label_pairs(&result, "references", Some("parameter_type"));
    // Promoted `Repo` is both a ctor parameter type and a class field.
    assert!(
        param_refs.iter().any(|(_, t)| t == "Repo"),
        "Repo parameter_type missing: {param_refs:?}"
    );
    assert!(
        field_refs
            .iter()
            .any(|(s, t)| s == "Service" && t == "Repo"),
        "Service->Repo field missing: {field_refs:?}"
    );
    // Non-promoted `Logger` is only a parameter type, never a class field.
    assert!(
        param_refs.iter().any(|(_, t)| t == "Logger"),
        "Logger parameter_type missing: {param_refs:?}"
    );
    assert!(
        !field_refs.iter().any(|(_, t)| t == "Logger"),
        "non-promoted Logger must not leak a field edge: {field_refs:?}"
    );
    Ok(())
}

/// PHP static property access (`DefaultPalette::$primary`) → `uses_static_prop`.
/// Mirrors `test_php_finds_static_property_access`.
#[test]
fn php_finds_static_property_access() {
    let r = extract_php(&fixtures().join("sample_php_static_prop.php"));
    let primary = php_node_id(&r, ".primary()");
    let palette = php_node_id(&r, "DefaultPalette");
    assert!(
        php_has_edge(&r, &primary, &palette, "uses_static_prop"),
        "ColorResolver::primary() should resolve DefaultPalette::$primary to a \
         uses_static_prop edge into the owning class"
    );
}

/// PHP `config('throttle.api.per_second')` → `uses_config` edge to `Throttle`.
/// Mirrors `test_php_finds_config_helper_call`.
#[test]
fn php_finds_config_helper_call() {
    let r = extract_php(&fixtures().join("sample_php_config.php"));
    let per_second = php_node_id(&r, ".perSecond()");
    let throttle = php_node_id(&r, "Throttle");
    assert!(
        php_has_edge(&r, &per_second, &throttle, "uses_config"),
        "RateLimiter::perSecond() should resolve config('throttle.api.per_second') \
         to a uses_config edge into the Throttle config class"
    );
}

/// PHP `$this->app->bind(Foo::class, Bar::class)` → `bound_to` edge.
/// Mirrors `test_php_finds_container_bind`.
#[test]
fn php_finds_container_bind() {
    let r = extract_php(&fixtures().join("sample_php_container.php"));
    let payment = php_node_id(&r, "PaymentGateway");
    let stripe = php_node_id(&r, "StripeGateway");
    let register = php_node_id(&r, ".register()");
    assert!(
        php_has_edge(&r, &payment, &stripe, "bound_to"),
        "bind(PaymentGateway::class, StripeGateway::class) should bind the abstract \
         to the concrete implementation"
    );
    // Each `::class` argument is a class-constant access → a references_constant
    // edge from the enclosing method to the referenced class.
    assert!(
        php_has_edge(&r, &register, &payment, "references_constant"),
        "register() should reference PaymentGateway via its ::class constant"
    );
    // The singleton() binding takes the same abstract→concrete shape.
    let cashier = php_node_id(&r, "CashierGateway");
    assert!(
        php_has_edge(&r, &cashier, &stripe, "bound_to"),
        "singleton(CashierGateway::class, StripeGateway::class) should also bind \
         the abstract to the concrete implementation"
    );
}

/// PHP `$listen = [Event::class => [Listener::class]]` → `listened_by` edges.
/// Mirrors `test_php_finds_event_listeners`.
#[test]
fn php_finds_event_listeners() {
    let r = extract_php(&fixtures().join("sample_php_listen.php"));
    let user_registered = php_node_id(&r, "UserRegistered");
    let welcome = php_node_id(&r, "SendWelcomeEmail");
    assert!(
        php_has_edge(&r, &user_registered, &welcome, "listened_by"),
        "UserRegistered should map to a listened_by edge into SendWelcomeEmail"
    );
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

/// Ports `test_swift_enum_associated_value_type_reference` (ad70152):
/// a Swift enum case with an associated value (`case failed(Config)`) emits a
/// `references[type]` edge from the enum to the associated type.
#[test]
fn swift_enum_associated_value_type_references() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("NetworkError.swift");
    std::fs::write(
        &source,
        "class Config {}\nenum NetworkError {\n    case timeout\n    case failed(Config)\n}\n",
    )?;
    let result = extract_swift(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let type_refs = edge_label_pairs(&result, "references", Some("type"));
    assert!(
        type_refs
            .iter()
            .any(|(s, t)| s == "NetworkError" && t == "Config"),
        "NetworkError->Config type reference missing: {type_refs:?}"
    );
    Ok(())
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
    // Child node IDs share the file_stem prefix so file->symbol `contains` edges
    // connect (graphify-py #999). The single-file extractor encodes the absolute
    // path in the stem here; the multi-file post-pass canonicalises it to the
    // repo-relative form, keeping graph.json machine-independent.
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("mydir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let src_file = dir.join("sample.dart");
    std::fs::write(&src_file, b"class MyClass {}\nvoid myFunc() {}\n").expect("write");

    let result = extract_dart(&src_file);
    let stem = file_stem(&src_file);
    let expected_class_nid = make_id(&[&stem, "MyClass"]);
    let expected_func_nid = make_id(&[&stem, "myFunc"]);

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

    // Every non-file child node ID must keep the file_stem prefix.
    let stem_prefix = make_id(&[&stem]);
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

// ── .slnx (test_dotnet.py) ──────────────────────────────────────────────────

/// `test_slnx_extracts_projects`
#[test]
fn slnx_extracts_projects() {
    let r = extract_slnx(&fixtures().join("sample.slnx"));
    assert!(r.error.is_none(), "{:?}", r.error);
    let ls: std::collections::HashSet<&str> = labels(&r).into_iter().collect();
    assert!(ls.contains("WebApi"), "{ls:?}");
    assert!(ls.contains("Domain"), "{ls:?}");
    assert!(ls.contains("Tests"), "{ls:?}");
}

/// `test_slnx_contains_edges`
#[test]
fn slnx_contains_edges() {
    let r = extract_slnx(&fixtures().join("sample.slnx"));
    assert!(r.error.is_none(), "{:?}", r.error);
    let contains: Vec<_> = r
        .edges
        .iter()
        .filter(|e| e.relation == "contains")
        .collect();
    assert_eq!(contains.len(), 3);
}

/// `test_slnx_project_dependency`
#[test]
fn slnx_project_dependency() {
    let r = extract_slnx(&fixtures().join("sample.slnx"));
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(relations(&r).contains("imports"));
}

/// `test_slnx_invalid_xml`
#[test]
fn slnx_invalid_xml() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let f = tmp.path().join("bad.slnx");
    std::fs::write(&f, "<Solution><Project></Solution>")?;
    let r = extract_slnx(&f);
    assert!(r.error.is_some(), "expected XML parse error");
    Ok(())
}

/// `test_slnx_missing_file`
#[test]
fn slnx_missing_file() {
    let r = extract_slnx(Path::new("/nonexistent/file.slnx"));
    assert!(r.error.is_some());
}

// ── Salesforce Apex (.cls / .trigger) — test_languages.py ────────────────────

/// `test_apex_class_extraction`
#[test]
fn apex_class_extraction() {
    let r = extract_apex(&fixtures().join("sample.cls"));
    assert!(labels(&r).contains(&"AccountService"));
}

/// `test_apex_enum_extraction`
#[test]
fn apex_enum_extraction() {
    let r = extract_apex(&fixtures().join("sample.cls"));
    assert!(labels(&r).contains(&"AccountStatus"));
}

/// `test_apex_interface_extraction`
#[test]
fn apex_interface_extraction() {
    let r = extract_apex(&fixtures().join("sample.cls"));
    assert!(labels(&r).contains(&"Notifiable"));
}

/// Ports `test_apex_interface_extends` (53c769d): `interface X extends A, B`
/// emits one `extends` edge per parent — group 2 was captured but never read,
/// so interface multiple inheritance was silently dropped.
#[test]
fn apex_interface_extends() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let source = tmp.path().join("PaymentProcessor.cls");
    std::fs::write(
        &source,
        "public interface PaymentProcessor extends Processor, Auditable { void process(); }\n",
    )?;
    let result = extract_apex(&source);
    assert!(result.error.is_none(), "{:?}", result.error);
    let extends = edge_label_pairs(&result, "extends", None);
    assert!(
        extends
            .iter()
            .any(|(s, t)| s == "PaymentProcessor" && t == "Processor"),
        "PaymentProcessor extends Processor missing: {extends:?}"
    );
    assert!(
        extends
            .iter()
            .any(|(s, t)| s == "PaymentProcessor" && t == "Auditable"),
        "PaymentProcessor extends Auditable missing: {extends:?}"
    );
    Ok(())
}

/// `test_apex_method_extraction`
#[test]
fn apex_method_extraction() {
    let r = extract_apex(&fixtures().join("sample.cls"));
    let ls = labels(&r);
    for m in [
        "getAccounts",
        "updateAccountsAsync",
        "createAccounts",
        "deleteOldAccounts",
    ] {
        assert!(
            ls.iter().any(|l| l.contains(m)),
            "missing method {m}: {ls:?}"
        );
    }
}

/// `test_apex_contains_and_method_relations`
#[test]
fn apex_contains_and_method_relations() {
    let r = extract_apex(&fixtures().join("sample.cls"));
    let rels = relations(&r);
    assert!(rels.contains("contains"));
    assert!(rels.contains("method"));
}

/// `test_apex_soql_uses_edge`
#[test]
fn apex_soql_uses_edge() {
    let r = extract_apex(&fixtures().join("sample.cls"));
    assert!(relations(&r).contains("uses"));
    assert!(labels(&r).contains(&"Account"));
}

/// `test_apex_dml_uses_edge`
#[test]
fn apex_dml_uses_edge() {
    let r = extract_apex(&fixtures().join("sample.cls"));
    let dml: Vec<&str> = r
        .nodes
        .iter()
        .map(|n| n.label.as_str())
        .filter(|l| matches!(*l, "insert" | "update" | "delete" | "upsert"))
        .collect();
    assert!(!dml.is_empty(), "expected DML nodes: {dml:?}");
}

/// `test_apex_file_node_present`
#[test]
fn apex_file_node_present() {
    let r = extract_apex(&fixtures().join("sample.cls"));
    assert!(labels(&r).contains(&"sample.cls"));
}

/// `test_apex_trigger_extraction`
#[test]
fn apex_trigger_extraction() {
    let r = extract_apex(&fixtures().join("sample.trigger"));
    let ls = labels(&r);
    assert!(ls.contains(&"sample.trigger"), "{ls:?}");
    assert!(ls.contains(&"AccountTrigger"), "{ls:?}");
}

/// `test_apex_trigger_uses_sobject`
#[test]
fn apex_trigger_uses_sobject() {
    let r = extract_apex(&fixtures().join("sample.trigger"));
    assert!(relations(&r).contains("uses"));
    assert!(labels(&r).contains(&"Account"));
}

/// Inline annotations (annotation + declaration on the same line) must not
/// drop the declaration. This is a DIVERGENCE from graphify-py's `extract_apex`,
/// which `continue`s on every `@`-line and so loses inline-annotated classes and
/// methods despite the declaration regexes carrying an annotation prefix.
#[test]
fn apex_inline_annotation_keeps_declaration() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("Inline.cls");
    std::fs::write(
        &src,
        "@IsTest public class Inline {\n    @AuraEnabled public static String foo() { return null; }\n}\n",
    )
    .expect("write cls");
    let r = extract_apex(&src);
    let ls = labels(&r);
    assert!(
        ls.contains(&"Inline"),
        "inline-annotated class was dropped: {ls:?}"
    );
    assert!(
        ls.iter().any(|l| l.contains("foo")),
        "inline-annotated method was dropped: {ls:?}"
    );
}

/// Own-line annotations must keep working after the inline-annotation fix: the
/// pending annotation has to carry to the declaration on the next line so the
/// `@AuraEnabled`/`@InvocableMethod` `contains` edge is still emitted.
#[test]
fn apex_own_line_annotation_still_carries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("OwnLine.cls");
    std::fs::write(
        &src,
        "public class OwnLine {\n    @AuraEnabled\n    public static String bar() { return null; }\n}\n",
    )
    .expect("write cls");
    let r = extract_apex(&src);
    assert!(
        labels(&r).iter().any(|l| l.contains("bar")),
        "own-line-annotated method was dropped: {:?}",
        labels(&r)
    );
    assert!(
        relations(&r).contains("contains"),
        "carried @AuraEnabled did not produce a contains edge"
    );
}

/// `test_apex_missing_file_returns_empty`
#[test]
fn apex_missing_file_returns_empty() {
    let r = extract_apex(Path::new("nonexistent.cls"));
    assert!(r.nodes.is_empty());
    assert!(r.edges.is_empty());
}

/// `test_apex_no_dangling_edges`
#[test]
fn apex_no_dangling_edges() {
    for fixture in ["sample.cls", "sample.trigger"] {
        let r = extract_apex(&fixtures().join(fixture));
        let ids: std::collections::HashSet<&str> = r.nodes.iter().map(|n| n.id.as_str()).collect();
        for e in &r.edges {
            assert!(
                ids.contains(e.source.as_str()),
                "dangling source in {fixture}: {e:?}"
            );
            assert!(
                ids.contains(e.target.as_str()),
                "dangling target in {fixture}: {e:?}"
            );
        }
    }
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
fn dm_call_does_not_resolve_to_type_node() {
    // A bare call whose name matches a *type's* last segment must not resolve to
    // that (non-callable) type node — there is no proc by that name, so it
    // belongs in raw_calls. graphify-py indexes every label and would resolve
    // `widget()` to the `/datum/widget` type (a latent bug we fix in Rust).
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = "/datum/widget\n\tvar/size = 1\n\n/proc/run()\n\twidget()\n";
    let path = tmp.path().join("types.dm");
    std::fs::write(&path, src).expect("write dm");

    let r = extract_dm(&path);
    let Some(widget_type) = r.nodes.iter().find(|n| n.label == "/datum/widget") else {
        panic!("expected a `/datum/widget` type node, got {:?}", labels(&r));
    };
    assert!(
        !r.edges
            .iter()
            .any(|e| e.relation == "calls" && e.target == widget_type.id),
        "a bare call must not resolve to a non-callable type node"
    );
    assert!(
        r.raw_calls.iter().any(|rc| rc.callee == "widget"),
        "the unresolved call should be recorded in raw_calls: {:?}",
        r.raw_calls
    );
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
    // A path under a fresh tempdir that is never created: isolated and
    // platform-independent (no reliance on a hard-coded absolute path).
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing = tmp.path().join("graphify").join("byond").join("sample");
    assert!(extract_dm(&missing.with_extension("dm")).error.is_some());
    assert!(extract_dmi(&missing.with_extension("dmi")).error.is_some());
    assert!(extract_dmm(&missing.with_extension("dmm")).error.is_some());
    assert!(extract_dmf(&missing.with_extension("dmf")).error.is_some());
}

#[test]
fn dm_parent_relative_include_resolves_to_parent_dir() {
    // graphify-py's `lstrip("./")` collapses `../helpers.dm` to `helpers.dm`,
    // mis-resolving parent-relative includes. The Rust normaliser preserves
    // `../`, so this include resolves to the real file one directory up and
    // emits a non-external `imports_from` edge.
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("helpers.dm"), "/proc/helper()\n").expect("write helper");
    let sub = tmp.path().join("sub");
    std::fs::create_dir_all(&sub).expect("create sub");
    let main = sub.join("main.dm");
    std::fs::write(&main, "#include \"../helpers.dm\"\n/proc/run()\n").expect("write main");

    let r = extract_dm(&main);
    let import_edges = edges_with_relation(&r, &["imports", "imports_from"]);
    assert!(
        import_edges
            .iter()
            .any(|e| e.relation == "imports_from" && !e.external),
        "a `../` include of an existing file must resolve (non-external imports_from): {import_edges:?}"
    );
}

// ── Python package-form submodule imports (#1146) ────────────────────────────

/// #1146: `from pkg import submod` where `pkg` is a package (`__init__.py`) and
/// `submod` is a submodule file on disk should emit a file-level
/// `imports_from`/`submodule_import` edge to the submodule, so package-form
/// imports do not leave the submodule as a disconnected island. Covers both the
/// relative (`from .pkg import …`) and absolute (`from pkg import …`) forms.
#[test]
fn python_package_form_import_resolves_submodule() -> Result<(), Box<dyn std::error::Error>> {
    use graphify_extract::{extract_python, make_id1};

    let tmp = tempfile::tempdir()?;
    let root = tmp.path();
    // Lay out a package with two submodules and a sibling importer.
    let pkg = root.join("pkg");
    std::fs::create_dir_all(&pkg)?;
    std::fs::write(pkg.join("__init__.py"), "")?;
    std::fs::write(pkg.join("models.py"), "class Widget:\n    pass\n")?;
    std::fs::create_dir_all(pkg.join("services"))?;
    std::fs::write(pkg.join("services").join("__init__.py"), "")?;

    // Importer lives inside the package and uses both absolute and relative forms.
    let importer = pkg.join("app.py");
    std::fs::write(
        &importer,
        "from pkg import models\nfrom . import services\nfrom pkg import missing\n",
    )?;

    let result = extract_python(&importer);
    let importer_nid = make_id1(&importer.to_string_lossy());

    let submodule_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| {
            e.relation == "imports_from" && e.context.as_deref() == Some("submodule_import")
        })
        .collect();

    // Absolute `from pkg import models` resolves to pkg/models.py.
    let models_nid = make_id1(&pkg.join("models.py").to_string_lossy());
    assert!(
        submodule_edges
            .iter()
            .any(|e| e.source == importer_nid && e.target == models_nid),
        "expected submodule_import edge to pkg/models.py: {submodule_edges:?}"
    );

    // Relative `from . import services` resolves to pkg/services/__init__.py.
    let services_nid = make_id1(&pkg.join("services").join("__init__.py").to_string_lossy());
    assert!(
        submodule_edges
            .iter()
            .any(|e| e.source == importer_nid && e.target == services_nid),
        "expected submodule_import edge to pkg/services/__init__.py: {submodule_edges:?}"
    );

    // `from pkg import missing` has no file on disk → no submodule edge for it,
    // only the ordinary module-level imports_from edge.
    assert_eq!(
        submodule_edges.len(),
        2,
        "only resolvable submodules should produce edges: {submodule_edges:?}"
    );
    Ok(())
}
