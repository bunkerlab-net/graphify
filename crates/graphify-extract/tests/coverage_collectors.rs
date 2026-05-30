//! Rust-only branch-coverage tests for the v0.8.25 semantic type-reference
//! collectors and inheritance emitters.
//!
//! Unlike `parity_semantic_types.rs` (1:1 ports of graphify-py's suite over the
//! shared `sample.*` fixtures), these drive small inline snippets crafted to
//! exercise collector branches the single-fixture-per-language tests miss:
//! qualified / scoped names, generic arguments, optional / nullable / union
//! wrappers, collection types, and the inheritance-classification variants.

#![allow(clippy::expect_used)]

use graphify_extract::{
    FileResult, extract_c, extract_cpp, extract_go, extract_kotlin, extract_php, extract_rust,
    extract_scala, extract_swift,
};

mod common;
use common::has_edge;

/// Write `src` to a tempfile named `name` and run `extract`, returning the result.
fn extract_snippet(
    name: &str,
    src: &str,
    extract: fn(&std::path::Path) -> FileResult,
) -> (tempfile::TempDir, FileResult) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join(name);
    std::fs::write(&path, src).expect("write snippet");
    let r = extract(&path);
    assert!(r.error.is_none(), "extract error: {:?}", r.error);
    (tmp, r)
}

// ── Go: qualified types, generics, collection wrappers ─────────────────────────

#[test]
fn go_collects_qualified_generic_and_collection_type_refs() {
    let src = "package p\n\
type Item struct{}\n\
type Store struct {\n\
    items map[string]*Item\n\
    cache []Item\n\
    ch chan Item\n\
}\n\
func process(parts []Item, lookup map[string]Item) (chan Item, error) {\n\
    return nil, nil\n\
}\n";
    let (_t, r) = extract_snippet("g.go", src, extract_go);
    // map/slice/channel wrappers recurse to the element type.
    assert!(has_edge(&r, "references", Some("field"), "Store", "Item"));
    assert!(has_edge(
        &r,
        "references",
        Some("parameter_type"),
        "process",
        "Item"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "process",
        "Item"
    ));
}

#[test]
fn go_generic_type_arguments_are_generic_args() {
    let src = "package p\n\
type Box[T any] struct{ v T }\n\
type Payload struct{}\n\
type Holder struct {\n\
    boxed Box[Payload]\n\
}\n";
    let (_t, r) = extract_snippet("gg.go", src, extract_go);
    assert!(has_edge(&r, "references", Some("field"), "Holder", "Box"));
    assert!(has_edge(
        &r,
        "references",
        Some("generic_arg"),
        "Holder",
        "Payload"
    ));
}

// ── Rust: scoped ids, references/tuples/slices, multiple supertraits ───────────

#[test]
fn rust_scoped_and_wrapped_type_refs() {
    let src = "struct Config {}\n\
struct Engine {\n\
    cfg: std::sync::Arc<Config>,\n\
    pair: (Config, Config),\n\
    slot: [Config; 4],\n\
}\n\
fn run(input: &Config, items: Vec<Config>) -> Option<Config> { None }\n";
    let (_t, r) = extract_snippet("r.rs", src, extract_rust);
    // Arc<Config> → Arc (field) + Config (generic_arg); reference/tuple/array recurse.
    assert!(has_edge(
        &r,
        "references",
        Some("generic_arg"),
        "Engine",
        "Config"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "Engine",
        "Config"
    ));
    // &Config → direct parameter_type; Vec<Config>/Option<Config> → generic_arg.
    assert!(has_edge(
        &r,
        "references",
        Some("parameter_type"),
        "run",
        "Config"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("generic_arg"),
        "run",
        "Config"
    ));
}

#[test]
fn rust_trait_with_multiple_supertraits_and_generic_impl() {
    let src = "trait A {}\n\
trait B {}\n\
trait C: A + B {}\n\
struct Widget {}\n\
struct Canvas {}\n\
trait Render<T> {}\n\
impl Render<Widget> for Widget {}\n\
impl Render<Widget> for Canvas {}\n";
    let (_t, r) = extract_snippet("rt.rs", src, extract_rust);
    // Each supertrait in `C: A + B` is a separate bound, so both are `inherits`
    // (a supertrait is inheritance, not a generic argument).
    assert!(has_edge(&r, "inherits", None, "C", "A"));
    assert!(has_edge(&r, "inherits", None, "C", "B"));
    // impl Render<Widget> for Widget → implements Render; the Widget generic arg
    // equals the impl type, so the self-edge is intentionally skipped.
    assert!(has_edge(&r, "implements", None, "Widget", "Render"));
    // impl Render<Widget> for Canvas → implements Render + generic_arg Widget,
    // exercising the generic-argument branch (impl type ≠ generic arg).
    assert!(has_edge(&r, "implements", None, "Canvas", "Render"));
    assert!(has_edge(
        &r,
        "references",
        Some("generic_arg"),
        "Canvas",
        "Widget"
    ));
}

// ── Swift: optionals, arrays, dictionaries, generic base, struct conformance ───

#[test]
fn swift_collection_and_generic_member_refs() {
    let src = "protocol Drawable {}\n\
class Shape {}\n\
class Box<T> {}\n\
struct Canvas {\n\
    var shapes: [Shape]\n\
    var lookup: [String: Shape]\n\
    var maybe: Shape?\n\
    var boxed: Box<Shape>\n\
}\n\
class View: Shape, Drawable {\n\
    func render(items: [Shape]) -> Box<Shape> { return Box<Shape>() }\n\
}\n";
    let (_t, r) = extract_snippet("s.swift", src, extract_swift);
    // struct (Canvas) conformance-less; field types via collection/optional/generic.
    assert!(has_edge(&r, "references", Some("field"), "Canvas", "Shape"));
    assert!(has_edge(
        &r,
        "references",
        Some("generic_arg"),
        "Canvas",
        "Shape"
    ));
    // class View: first base = inherits (Shape), protocol = implements (Drawable).
    assert!(has_edge(&r, "inherits", None, "View", "Shape"));
    assert!(has_edge(&r, "implements", None, "View", "Drawable"));
    assert!(has_edge(
        &r,
        "references",
        Some("parameter_type"),
        "render",
        "Shape"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "render",
        "Box"
    ));
}

#[test]
fn swift_struct_enum_actor_conform_protocols() {
    let src = "protocol P {}\n\
struct S: P {}\n\
enum E: P { case a }\n\
actor A: P {}\n";
    let (_t, r) = extract_snippet("se.swift", src, extract_swift);
    // struct/enum/actor can only conform to protocols → all implements.
    assert!(has_edge(&r, "implements", None, "S", "P"));
    assert!(has_edge(&r, "implements", None, "E", "P"));
    assert!(has_edge(&r, "implements", None, "A", "P"));
}

// ── PHP: nullable, union, qualified names ──────────────────────────────────────

#[test]
fn php_nullable_union_and_qualified_type_refs() {
    let src = "<?php\n\
class Result {}\n\
class Other {}\n\
class Svc {\n\
    private ?Result $maybe;\n\
    public function run(Result|Other $in): ?Result { return null; }\n\
}\n";
    let (_t, r) = extract_snippet("p.php", src, extract_php);
    assert!(has_edge(&r, "references", Some("field"), "Svc", "Result"));
    assert!(has_edge(
        &r,
        "references",
        Some("parameter_type"),
        "run",
        "Result"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("parameter_type"),
        "run",
        "Other"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "run",
        "Result"
    ));
}

// ── Kotlin: nullable, generic projection, constructor vs interface base ────────

#[test]
fn kotlin_nullable_generic_projection_refs() {
    let src = "open class Base\n\
interface Iface\n\
class Result<T>\n\
class Payload\n\
class Worker : Base(), Iface {\n\
    var slot: Result<Payload>? = null\n\
    fun handle(items: List<Payload>): Result<Payload>? = null\n\
}\n";
    let (_t, r) = extract_snippet("k.kt", src, extract_kotlin);
    assert!(has_edge(&r, "inherits", None, "Worker", "Base"));
    assert!(has_edge(&r, "implements", None, "Worker", "Iface"));
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "Worker",
        "Result"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("generic_arg"),
        "Worker",
        "Payload"
    ));
    // handle(items: List<Payload>) → List is the param type, Payload a generic arg.
    assert!(has_edge(
        &r,
        "references",
        Some("generic_arg"),
        "handle",
        "Payload"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "handle",
        "Result"
    ));
}

// ── Scala: generic base, with-mixins, generic val/param ────────────────────────

#[test]
fn scala_generic_base_and_type_args() {
    let src = "trait Logger\n\
class Base[T]\n\
class Payload\n\
class Service(cfg: Payload) extends Base[Payload] with Logger {\n\
    val store: List[Payload] = Nil\n\
    def build(items: List[Payload]): Payload = cfg\n\
}\n";
    let (_t, r) = extract_snippet("sc.scala", src, extract_scala);
    // extends Base[Payload] → inherits Base; with Logger → mixes_in.
    assert!(has_edge(&r, "inherits", None, "Service", "Base"));
    assert!(has_edge(&r, "mixes_in", None, "Service", "Logger"));
    // constructor param + val + method generic-arg/return.
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "Service",
        "Payload"
    ));
    // build(items: List[Payload]) → List is the param type, Payload a generic arg.
    assert!(has_edge(
        &r,
        "references",
        Some("generic_arg"),
        "build",
        "Payload"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "build",
        "Payload"
    ));
}

// ── C / C++: pointers, arrays, qualified ids, templates ────────────────────────

#[test]
fn c_pointer_and_array_param_return_refs() {
    let src = "typedef struct { int x; } Point;\n\
Point *clone(Point *src, Point pool[]) {\n\
    return src;\n\
}\n";
    let (_t, r) = extract_snippet("c.c", src, extract_c);
    assert!(has_edge(
        &r,
        "references",
        Some("parameter_type"),
        "clone",
        "Point"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("return_type"),
        "clone",
        "Point"
    ));
}

#[test]
fn cpp_qualified_and_template_type_refs() {
    let src = "#include <vector>\n\
#include <memory>\n\
class Widget {};\n\
class Manager {\n\
public:\n\
    std::vector<Widget> all();\n\
    std::shared_ptr<Widget> find(const std::vector<Widget>& pool);\n\
private:\n\
    std::vector<Widget> items_;\n\
};\n";
    let (_t, r) = extract_snippet("cc.cpp", src, extract_cpp);
    // field: vector (template base) + Widget (generic arg).
    assert!(has_edge(
        &r,
        "references",
        Some("field"),
        "Manager",
        "vector"
    ));
    assert!(has_edge(
        &r,
        "references",
        Some("generic_arg"),
        "Manager",
        "Widget"
    ));
}

// ── Forward references: placeholder reconciliation ─────────────────────────────

/// All `(id)`s of nodes carrying `label`.
#[must_use]
fn node_ids_for_label<'a>(r: &'a FileResult, label: &str) -> Vec<&'a str> {
    r.nodes
        .iter()
        .filter(|n| n.label == label)
        .map(|n| n.id.as_str())
        .collect()
}

#[test]
fn rust_forward_reference_binds_to_declaration_not_placeholder() {
    // `Engine` references `Item` before `Item` is declared. The forward
    // reference must resolve to the single file-qualified `Item` declaration,
    // not a duplicate bare-name placeholder.
    let src = "struct Engine {\n    item: Item,\n}\nstruct Item {}\n";
    let (_t, r) = extract_snippet("fwd.rs", src, extract_rust);
    let item_ids = node_ids_for_label(&r, "Item");
    assert_eq!(
        item_ids.len(),
        1,
        "expected one Item node, got {item_ids:?}"
    );
    assert_ne!(
        item_ids[0],
        graphify_extract::make_id1("Item"),
        "Item node must be the file-qualified declaration, not a bare placeholder"
    );
    // The field reference edge must target that single declaration.
    let field_target = r
        .edges
        .iter()
        .find(|e| e.relation == "references" && e.context.as_deref() == Some("field"))
        .map(|e| e.target.as_str());
    assert_eq!(
        field_target,
        Some(item_ids[0]),
        "field edge must point at the Item declaration"
    );
}

#[test]
fn go_forward_reference_binds_to_declaration_not_placeholder() {
    // `Store` references `Item` before `Item` is declared (Go allows any order).
    let src = "package p\ntype Store struct {\n    item *Item\n}\ntype Item struct{}\n";
    let (_t, r) = extract_snippet("fwd.go", src, extract_go);
    let item_ids = node_ids_for_label(&r, "Item");
    assert_eq!(
        item_ids.len(),
        1,
        "expected one Item node, got {item_ids:?}"
    );
    assert_ne!(
        item_ids[0],
        graphify_extract::make_id1("Item"),
        "Item node must be the package-qualified declaration, not a bare placeholder"
    );
    assert!(has_edge(&r, "references", Some("field"), "Store", "Item"));
}
