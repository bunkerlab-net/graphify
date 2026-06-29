//! Cross-file extraction tests — exercise the multi-file import resolution
//! paths in `extractors/multi.rs`.

#![allow(clippy::expect_used)]

use std::fs;

use graphify_extract::extract;

/// Read a string value from an extract-output edge or node by key.
#[must_use]
fn lookup_str(m: &indexmap::IndexMap<String, serde_json::Value>, key: &str) -> Option<String> {
    m.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

#[test]
fn java_cross_file_imports_resolve() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let pkg = tmp.path().join("com").join("example");
    fs::create_dir_all(&pkg).expect("create_dir_all");
    fs::write(
        pkg.join("Producer.java"),
        "package com.example;\n\nimport com.example.Consumer;\n\npublic class Producer {\n    public void send() {\n        Consumer c = new Consumer();\n        c.receive();\n    }\n}\n",
    )
    .expect("test invariant");
    fs::write(
        pkg.join("Consumer.java"),
        "package com.example;\n\npublic class Consumer {\n    public void receive() {}\n}\n",
    )
    .expect("test invariant");

    let result = extract(
        &[pkg.join("Producer.java"), pkg.join("Consumer.java")],
        Some(tmp.path()),
    );
    assert!(!result.nodes.is_empty(), "no nodes for java multi-file");
}

#[test]
fn python_cross_file_with_relative_imports() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let pkg = tmp.path().join("pkg");
    fs::create_dir_all(&pkg).expect("create_dir_all");
    fs::write(pkg.join("__init__.py"), "").expect("test invariant");
    fs::write(
        pkg.join("models.py"),
        "class User:\n    pass\n\nclass Order:\n    pass\n",
    )
    .expect("test invariant");
    fs::write(
        pkg.join("service.py"),
        "from .models import User, Order\n\nclass UserService:\n    def find(self):\n        return User()\n\nclass OrderService:\n    def list(self):\n        return [Order()]\n",
    )
    .expect("test invariant");
    fs::write(
        pkg.join("main.py"),
        "from pkg.service import UserService, OrderService\n\ndef run():\n    s = UserService()\n    s.find()\n",
    )
    .expect("test invariant");

    let result = extract(
        &[
            pkg.join("models.py"),
            pkg.join("service.py"),
            pkg.join("main.py"),
        ],
        Some(tmp.path()),
    );
    assert!(!result.nodes.is_empty());
    // Should produce some imports_from edges across files.
    let imports_from: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.get("relation").and_then(|v| v.as_str()) == Some("imports_from"))
        .collect();
    assert!(!imports_from.is_empty(), "expected imports_from edges");
}

#[test]
fn mixed_language_corpus() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let py = tmp.path().join("a.py");
    let rs = tmp.path().join("b.rs");
    let go = tmp.path().join("c.go");
    let md = tmp.path().join("d.md");

    fs::write(&py, "def hello(): return 'py'\n").expect("test invariant");
    fs::write(
        &rs,
        "pub fn hello() -> &'static str { \"rs\" }\nstruct Foo;\n",
    )
    .expect("test invariant");
    fs::write(
        &go,
        "package main\n\nfunc Hello() string {\n    return \"go\"\n}\n",
    )
    .expect("test invariant");
    fs::write(&md, "# Title\n\nContent.\n").expect("write fixture");

    let result = extract(&[py, rs, go, md], Some(tmp.path()));
    // Mixed corpus should produce nodes from each.
    let sources: std::collections::HashSet<_> = result
        .nodes
        .iter()
        .filter_map(|n| n.get("source_file").and_then(|v| v.as_str()))
        .map(|s| {
            // Get extension only.
            std::path::Path::new(s)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();
    // At least 2 of the 4 languages should appear.
    assert!(
        sources.len() >= 2,
        "expected multiple source extensions, got: {sources:?}"
    );
}

#[test]
fn extract_with_blade_and_fortran_and_unknown() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let blade = tmp.path().join("template.blade.php");
    let fortran = tmp.path().join("calc.f90");
    let unknown = tmp.path().join("random.xyz");
    let pascal_inc = tmp.path().join("helper.inc");
    fs::write(
        &blade,
        "@extends('layout')\n@include('partial')\n<button wire:click=\"go\">x</button>\n",
    )
    .expect("test invariant");
    fs::write(
        &fortran,
        "module mymod\ncontains\n  subroutine sub_one()\n  end subroutine\nend module\n",
    )
    .expect("test invariant");
    fs::write(&unknown, "ignored content").expect("write fixture");
    fs::write(&pascal_inc, "procedure Foo; begin end;\n").expect("write fixture");
    let result = extract(&[blade, fortran, unknown, pascal_inc], Some(tmp.path()));
    assert!(!result.nodes.is_empty(), "expected nodes from mixed corpus");
}

#[test]
fn extract_empty_paths_returns_empty() {
    let result = extract(&[], None);
    assert!(result.nodes.is_empty());
    assert!(result.edges.is_empty());
    assert_eq!(result.input_tokens, 0);
}

#[test]
fn extract_single_file_uses_parent_as_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("solo.py");
    fs::write(&path, "def x(): pass\n").expect("test invariant");
    let result = extract(&[path], None);
    assert!(!result.nodes.is_empty());
}

#[test]
fn extract_with_cache_root_uses_provided_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("cached.py");
    fs::write(&path, "def x(): pass\n").expect("test invariant");
    let result = extract(&[path], Some(tmp.path()));
    assert!(!result.nodes.is_empty());
}

/// Python function definitions emit `references` edges with `parameter_type`,
/// `return_type`, and `generic_arg` contexts depending on the annotation shape.
///
/// Ports `tests/test_python_import_resolution.py::test_python_parameter_return_and_generic_contexts`.
#[test]
fn python_parameter_return_and_generic_contexts() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let pkg = tmp.path().join("pkg");
    fs::create_dir_all(&pkg).expect("create_dir_all");
    let model = pkg.join("model.py");
    fs::write(
        &model,
        "class Payload:\n    pass\n\nclass Result:\n    pass\n",
    )
    .expect("write model.py");
    let service = pkg.join("service.py");
    fs::write(
        &service,
        "from .model import Payload, Result\n\n\
         def process(item: Payload) -> Result:\n    return Result()\n\n\
         def process_many(items: list[Payload]) -> Result:\n    return Result()\n",
    )
    .expect("write service.py");

    let result = extract(&[model.clone(), service.clone()], Some(tmp.path()));
    let id_to_label: std::collections::HashMap<String, String> = result
        .nodes
        .iter()
        .filter_map(|n| Some((lookup_str(n, "id")?, lookup_str(n, "label")?)))
        .collect();
    let pairs: std::collections::HashSet<(String, String, Option<String>)> = result
        .edges
        .iter()
        .filter(|e| lookup_str(e, "relation").as_deref() == Some("references"))
        .map(|e| {
            let src = lookup_str(e, "source").unwrap_or_default();
            let tgt = lookup_str(e, "target").unwrap_or_default();
            (
                id_to_label.get(&src).cloned().unwrap_or(src),
                id_to_label.get(&tgt).cloned().unwrap_or(tgt),
                lookup_str(e, "context"),
            )
        })
        .collect();

    assert!(
        pairs.contains(&(
            "process()".to_string(),
            "Payload".to_string(),
            Some("parameter_type".to_string())
        )),
        "expected process() → Payload parameter_type edge, got {pairs:?}"
    );
    assert!(
        pairs.contains(&(
            "process()".to_string(),
            "Result".to_string(),
            Some("return_type".to_string())
        )),
        "expected process() → Result return_type edge, got {pairs:?}"
    );
    assert!(
        pairs.contains(&(
            "process_many()".to_string(),
            "Payload".to_string(),
            Some("generic_arg".to_string())
        )),
        "expected process_many() → Payload generic_arg edge, got {pairs:?}"
    );
}

/// TypeScript class declarations emit `inherits` / `implements` edges from
/// `class_heritage`, and method signatures emit `references` edges with
/// `parameter_type`, `return_type`, `generic_arg` contexts.
///
/// Approximates `tests/test_js_import_resolution.py::test_ts_type_relationships_and_contexts`
/// without exercising cross-file symbol-fact resolution (TS facts collection
/// is not in the Rust port yet — this test pins the in-file shape so when
/// that pass lands it can be extended to cross-file).
#[test]
fn ts_type_relationships_and_contexts() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let lib = tmp.path().join("src").join("lib");
    fs::create_dir_all(&lib).expect("create_dir_all");
    let path = lib.join("impl.ts");
    fs::write(
        &path,
        "interface IProcessor<T> {}\n\
         abstract class BaseProcessor {}\n\
         type Result<T> = { value: T };\n\
         class Payload {}\n\
         export abstract class DataProcessor extends BaseProcessor implements IProcessor<Payload> {\n  \
             current!: Result<Payload>;\n  \
             run(input: Payload): Result<Payload> { return this.current; }\n\
         }\n",
    )
    .expect("write impl.ts");

    let result = extract(&[path], Some(tmp.path()));
    let id_to_label: std::collections::HashMap<String, String> = result
        .nodes
        .iter()
        .filter_map(|n| Some((lookup_str(n, "id")?, lookup_str(n, "label")?)))
        .collect();
    let label_of = |id: &str| -> String {
        id_to_label
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
            .trim_end_matches("()")
            .trim_start_matches('.')
            .to_string()
    };
    let triples: std::collections::HashSet<(String, String, String, Option<String>)> = result
        .edges
        .iter()
        .filter_map(|e| {
            Some((
                label_of(&lookup_str(e, "source")?),
                label_of(&lookup_str(e, "target")?),
                lookup_str(e, "relation")?,
                lookup_str(e, "context"),
            ))
        })
        .collect();

    assert!(
        triples.contains(&(
            "DataProcessor".to_string(),
            "BaseProcessor".to_string(),
            "inherits".to_string(),
            None,
        )),
        "expected DataProcessor inherits BaseProcessor, got {triples:?}"
    );
    assert!(
        triples.contains(&(
            "DataProcessor".to_string(),
            "IProcessor".to_string(),
            "implements".to_string(),
            None,
        )),
        "expected DataProcessor implements IProcessor, got {triples:?}"
    );
    assert!(
        triples.contains(&(
            "run".to_string(),
            "Payload".to_string(),
            "references".to_string(),
            Some("parameter_type".to_string()),
        )),
        "expected run → Payload parameter_type, got {triples:?}"
    );
    assert!(
        triples.contains(&(
            "run".to_string(),
            "Result".to_string(),
            "references".to_string(),
            Some("return_type".to_string()),
        )),
        "expected run → Result return_type, got {triples:?}"
    );
    assert!(
        triples.contains(&(
            "run".to_string(),
            "Payload".to_string(),
            "references".to_string(),
            Some("generic_arg".to_string()),
        )),
        "expected run → Payload generic_arg, got {triples:?}"
    );
}

/// Every emitted reference edge carries the canonical shape the Python helper
/// `_semantic_reference_edge` builds: `relation=references`,
/// `confidence=EXTRACTED`, `weight=1.0`, a non-empty `source_file`, and an
/// `Lnn` `source_location`.
///
/// Mirrors `tests/test_extract.py::test_semantic_reference_edges_carry_context_and_source`.
#[test]
fn semantic_reference_edges_carry_context_and_source() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("svc.py");
    fs::write(
        &path,
        "class Foo:\n    pass\n\nclass Bar:\n    pass\n\ndef call(x: Foo) -> Bar: return Bar()\n",
    )
    .expect("write svc.py");

    let result = extract(&[path], Some(tmp.path()));
    let ref_edge = result
        .edges
        .iter()
        .find(|e| {
            lookup_str(e, "relation").as_deref() == Some("references")
                && lookup_str(e, "context").as_deref() == Some("parameter_type")
        })
        .expect("expected at least one parameter_type reference edge");

    assert_eq!(
        lookup_str(ref_edge, "confidence").as_deref(),
        Some("EXTRACTED")
    );
    assert!(
        ref_edge
            .get("weight")
            .and_then(serde_json::Value::as_f64)
            .is_some_and(|w| (w - 1.0).abs() < f64::EPSILON),
        "weight should be 1.0, got {:?}",
        ref_edge.get("weight")
    );
    let source_file = lookup_str(ref_edge, "source_file").unwrap_or_default();
    assert!(
        source_file.ends_with("svc.py"),
        "source_file should end with svc.py, got {source_file:?}"
    );
    let loc = lookup_str(ref_edge, "source_location").unwrap_or_default();
    assert!(
        loc.strip_prefix('L')
            .is_some_and(|rest| rest.parse::<u32>().is_ok()),
        "source_location should be `Lnn`, got {loc:?}"
    );
}

// ── JS/TS default-import symbol resolution (#6dc23db) ──────────────────────────

use std::path::Path;

use graphify_extract::{file_node_id, file_stem, make_id};

/// Mirror of graphify-py's `_write`: write a file, creating parents.
fn write_file(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("create_dir_all");
    fs::write(path, text).expect("write");
}

/// `(file_node_id(source_file), make_id(file_stem(target_file), symbol), relation)`
/// edge present? Mirrors graphify-py `_has_symbol_edge`.
fn has_symbol_edge(
    result: &graphify_extract::ExtractOutput,
    source_file: &str,
    target_file: &str,
    symbol: &str,
    relation: &str,
) -> bool {
    let src = file_node_id(Path::new(source_file));
    let tgt = make_id(&[&file_stem(Path::new(target_file)), symbol]);
    result.edges.iter().any(|e| {
        lookup_str(e, "source").as_deref() == Some(src.as_str())
            && lookup_str(e, "target").as_deref() == Some(tgt.as_str())
            && lookup_str(e, "relation").as_deref() == Some(relation)
    })
}

/// `(make_id(stem(src_file), src_sym), make_id(stem(tgt_file), tgt_sym), relation)`
/// edge present? Mirrors graphify-py `_has_symbol_to_symbol_edge`.
fn has_symbol_to_symbol_edge(
    result: &graphify_extract::ExtractOutput,
    source_file: &str,
    source_symbol: &str,
    target_file: &str,
    target_symbol: &str,
    relation: &str,
) -> bool {
    let src = make_id(&[&file_stem(Path::new(source_file)), source_symbol]);
    let tgt = make_id(&[&file_stem(Path::new(target_file)), target_symbol]);
    result.edges.iter().any(|e| {
        lookup_str(e, "source").as_deref() == Some(src.as_str())
            && lookup_str(e, "target").as_deref() == Some(tgt.as_str())
            && lookup_str(e, "relation").as_deref() == Some(relation)
    })
}

#[test]
fn default_import_resolves_to_default_exported_class() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let target = tmp.path().join("src/lib/foo.ts");
    write_file(&target, "export default class Foo { id = '' }\n");
    let importer = tmp.path().join("src/routes/page.ts");
    write_file(&importer, "import Foo from '../lib/foo'\nnew Foo()\n");

    let result = extract(&[target, importer], Some(tmp.path()));
    assert!(has_symbol_edge(
        &result,
        "src/routes/page.ts",
        "src/lib/foo.ts",
        "Foo",
        "imports"
    ));
}

#[test]
fn default_import_with_renamed_binding_resolves_to_origin() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let target = tmp.path().join("src/lib/foo.ts");
    write_file(&target, "export default class Foo { id = '' }\n");
    let importer = tmp.path().join("src/routes/page.ts");
    write_file(
        &importer,
        "import Renamed from '../lib/foo'\nnew Renamed()\n",
    );

    let result = extract(&[target, importer], Some(tmp.path()));
    // Edge must target the origin symbol `Foo`, not the local binding `Renamed`.
    assert!(has_symbol_edge(
        &result,
        "src/routes/page.ts",
        "src/lib/foo.ts",
        "Foo",
        "imports"
    ));
}

#[test]
fn export_default_identifier_resolves_default_import() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let target = tmp.path().join("src/lib/foo.ts");
    write_file(&target, "class Foo { id = '' }\nexport default Foo\n");
    let importer = tmp.path().join("src/routes/page.ts");
    write_file(&importer, "import Foo from '../lib/foo'\nnew Foo()\n");

    let result = extract(&[target, importer], Some(tmp.path()));
    assert!(has_symbol_edge(
        &result,
        "src/routes/page.ts",
        "src/lib/foo.ts",
        "Foo",
        "imports"
    ));
}

#[test]
fn default_import_call_resolves_to_default_exported_function() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let target = tmp.path().join("src/lib/foo.ts");
    write_file(&target, "export default function makeFoo() { return 1 }\n");
    let importer = tmp.path().join("src/routes/page.ts");
    write_file(
        &importer,
        "import mk from '../lib/foo'\nconst X = () => mk()\n",
    );

    let result = extract(&[target, importer], Some(tmp.path()));
    // The call through the renamed default binding resolves to the origin.
    assert!(has_symbol_to_symbol_edge(
        &result,
        "src/routes/page.ts",
        "X",
        "src/lib/foo.ts",
        "makeFoo",
        "calls"
    ));
}

// ── #1446: qualified ClassName.method() call resolution ──────────────────────

type NodeMap<'a> =
    std::collections::HashMap<&'a str, &'a indexmap::IndexMap<String, serde_json::Value>>;

fn index_nodes(nodes: &[indexmap::IndexMap<String, serde_json::Value>]) -> NodeMap<'_> {
    nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(|v| v.as_str()).map(|id| (id, n)))
        .collect()
}

/// `field` of the node referenced by `edge[endpoint]` (a source/target id), or "".
fn endpoint_field(
    idx: &NodeMap,
    edge: &indexmap::IndexMap<String, serde_json::Value>,
    endpoint: &str,
    field: &str,
) -> String {
    edge.get(endpoint)
        .and_then(|v| v.as_str())
        .and_then(|id| idx.get(id))
        .and_then(|n| n.get(field))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

#[test]
fn python_qualified_class_method_call_resolves_extracted() {
    // `ClassName.method()` across files resolves to the class-qualified method
    // node with an EXTRACTED `calls` edge (#1446).
    let tmp = tempfile::tempdir().expect("tempdir");
    let actions = tmp.path().join("actions.py");
    let viewset = tmp.path().join("viewset.py");
    fs::write(
        &actions,
        "class TaskActions:\n    @staticmethod\n    def approve(pk):\n        return pk\n",
    )
    .expect("write actions");
    fs::write(
        &viewset,
        "from actions import TaskActions\n\nclass TaskViewSet:\n    def handle(self, request):\n        return TaskActions.approve(request)\n",
    )
    .expect("write viewset");
    let result = extract(&[viewset, actions], Some(tmp.path()));
    let idx = index_nodes(&result.nodes);
    let call_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| {
            e.get("relation").and_then(|v| v.as_str()) == Some("calls")
                && endpoint_field(&idx, e, "source", "label").contains("handle")
                && endpoint_field(&idx, e, "target", "label").contains("approve")
                && endpoint_field(&idx, e, "target", "source_file").contains("actions.py")
        })
        .collect();
    assert_eq!(
        call_edges.len(),
        1,
        "expected one handle->approve edge, got {call_edges:?}"
    );
    assert_eq!(
        call_edges[0].get("confidence").and_then(|v| v.as_str()),
        Some("EXTRACTED")
    );
}

#[test]
fn python_qualified_call_resolves_when_method_name_collides_with_caller() {
    // A viewset action `approve()` delegates to a SERVICE action of the SAME
    // name via `TaskActions.approve()`. The bare-name in-file lookup would match
    // the caller's own node and silently drop the call; the qualified receiver
    // must still resolve it cross-file (#1446).
    let tmp = tempfile::tempdir().expect("tempdir");
    let actions = tmp.path().join("actions.py");
    let viewset = tmp.path().join("viewset.py");
    fs::write(
        &actions,
        "class TaskActions:\n    @staticmethod\n    def approve(pk):\n        return pk\n",
    )
    .expect("write actions");
    fs::write(
        &viewset,
        "from actions import TaskActions\n\nclass TaskViewSet:\n    def approve(self, request):\n        return TaskActions.approve(request)\n",
    )
    .expect("write viewset");
    let result = extract(&[viewset, actions], Some(tmp.path()));
    let idx = index_nodes(&result.nodes);
    let cross: Vec<_> = result
        .edges
        .iter()
        .filter(|e| {
            e.get("relation").and_then(|v| v.as_str()) == Some("calls")
                && endpoint_field(&idx, e, "source", "source_file").contains("viewset.py")
                && endpoint_field(&idx, e, "target", "source_file").contains("actions.py")
                && endpoint_field(&idx, e, "target", "label").contains("approve")
        })
        .collect();
    assert_eq!(
        cross.len(),
        1,
        "expected viewset->service approve edge, got {cross:?}"
    );
    assert_eq!(
        cross[0].get("confidence").and_then(|v| v.as_str()),
        Some("EXTRACTED")
    );
}

#[test]
fn python_instance_member_call_not_overconnected() {
    // A lowercase-receiver member call (`obj.run()`) must NOT resolve cross-file
    // — the god-node guard stays intact (#1446).
    let tmp = tempfile::tempdir().expect("tempdir");
    let svc = tmp.path().join("svc.py");
    let worker = tmp.path().join("worker.py");
    fs::write(
        &svc,
        "class Service:\n    def run(self):\n        return 1\n",
    )
    .expect("write svc");
    fs::write(
        &worker,
        "class Worker:\n    def go(self, obj):\n        return obj.run()\n",
    )
    .expect("write worker");
    let result = extract(&[worker, svc], Some(tmp.path()));
    let idx = index_nodes(&result.nodes);
    let bad: Vec<_> = result
        .edges
        .iter()
        .filter(|e| {
            e.get("relation").and_then(|v| v.as_str()) == Some("calls")
                && endpoint_field(&idx, e, "source", "label").contains("go")
                && endpoint_field(&idx, e, "target", "label").contains("run")
        })
        .collect();
    assert!(
        bad.is_empty(),
        "instance member call must not connect cross-file: {bad:?}"
    );
}

#[test]
fn python_qualified_call_ambiguous_class_bails() {
    // When the class name is defined in 2+ files, the qualified call must not
    // resolve — single-definition god-node guard (#1446).
    let tmp = tempfile::tempdir().expect("tempdir");
    let a = tmp.path().join("a.py");
    let b = tmp.path().join("b.py");
    let caller = tmp.path().join("caller.py");
    fs::write(&a, "class Helper:\n    def do(self):\n        return 1\n").expect("write a");
    fs::write(&b, "class Helper:\n    def do(self):\n        return 2\n").expect("write b");
    fs::write(
        &caller,
        "from a import Helper\n\nclass C:\n    def f(self):\n        return Helper.do(self)\n",
    )
    .expect("write caller");
    let result = extract(&[caller, a, b], Some(tmp.path()));
    let idx = index_nodes(&result.nodes);
    let resolved: Vec<_> = result
        .edges
        .iter()
        .filter(|e| {
            e.get("relation").and_then(|v| v.as_str()) == Some("calls")
                && endpoint_field(&idx, e, "source", "label")
                    .trim_matches(|c| matches!(c, '(' | ')' | '.'))
                    == "f"
                && endpoint_field(&idx, e, "target", "label").contains("do")
        })
        .collect();
    assert!(
        resolved.is_empty(),
        "ambiguous class name must not resolve: {resolved:?}"
    );
}

#[test]
fn imported_type_stubs_do_not_collide_across_source_files() {
    // #1462: imported stdlib/type stubs with the same label are distinct uses
    // when there is no single project definition to rewire onto. They need the
    // referencing file as a disambiguator while still keeping `source_file` empty
    // so a real project definition can still be rewired by #1402. Mirrors
    // test_extract.py::test_imported_type_stubs_do_not_collide_across_source_files.
    let tmp = tempfile::tempdir().expect("tempdir");
    let pkg = tmp.path().join("pkg");
    fs::create_dir_all(&pkg).expect("create_dir_all");
    fs::write(
        pkg.join("a.py"),
        "from pathlib import Path\ndef use_a(p: Path):\n    return p\n",
    )
    .expect("test invariant");
    fs::write(
        pkg.join("b.py"),
        "from pathlib import Path\ndef use_b(p: Path):\n    return p\n",
    )
    .expect("test invariant");

    let result = extract(&[pkg.join("a.py"), pkg.join("b.py")], Some(tmp.path()));
    let path_nodes: Vec<_> = result
        .nodes
        .iter()
        .filter(|n| lookup_str(n, "label").as_deref() == Some("Path"))
        .collect();

    assert_eq!(path_nodes.len(), 2, "expected two distinct Path stubs");
    let ids: std::collections::HashSet<_> = path_nodes
        .iter()
        .filter_map(|n| lookup_str(n, "id"))
        .collect();
    assert_eq!(ids.len(), 2, "Path stubs must have distinct ids");
    assert!(
        path_nodes
            .iter()
            .all(|n| lookup_str(n, "source_file").unwrap_or_default().is_empty()),
        "Path stubs must stay sourceless so a real definition can be rewired on"
    );
}

#[test]
fn go_cross_file_type_refs_resolve_to_single_node() {
    // #1500: same-package Go references to a type defined once must resolve to the
    // single canonical node, not 1+N phantom duplicates with the referencing
    // file's path baked into the id. Mirrors
    // test_extract.py::test_go_cross_file_type_refs_resolve_to_single_node.
    let tmp = tempfile::tempdir().expect("tempdir");
    let pkg = tmp.path().join("pkg");
    fs::create_dir_all(&pkg).expect("create_dir_all");
    fs::write(
        pkg.join("thing.go"),
        "package pkg\n\ntype Thing struct{}\n\nfunc (t Thing) Run() int { return 1 }\n",
    )
    .expect("test invariant");
    fs::write(
        pkg.join("a.go"),
        "package pkg\n\nfunc UseA(obj Thing) Thing { return obj }\n",
    )
    .expect("test invariant");
    fs::write(
        pkg.join("b.go"),
        "package pkg\n\nfunc UseB(obj Thing) Thing { return obj }\n",
    )
    .expect("test invariant");

    let result = extract(
        &[pkg.join("thing.go"), pkg.join("a.go"), pkg.join("b.go")],
        Some(tmp.path()),
    );
    let thing_ids: Vec<String> = result
        .nodes
        .iter()
        .filter(|n| lookup_str(n, "label").as_deref() == Some("Thing"))
        .filter_map(|n| lookup_str(n, "id"))
        .collect();

    assert_eq!(
        thing_ids.len(),
        1,
        "expected one canonical Thing node, got {thing_ids:?}"
    );
    // The phantom signature is the referencing file's path (with .go extension)
    // baked into the id — must not appear.
    assert!(
        !thing_ids[0].contains("_go"),
        "phantom path-in-id: {}",
        thing_ids[0]
    );
    // Stronger than the substring guard: the surviving node must be the real
    // definition from thing.go, not a stub keyed off a referencing file (a.go/b.go).
    let thing_source = result
        .nodes
        .iter()
        .find(|n| lookup_str(n, "label").as_deref() == Some("Thing"))
        .and_then(|n| lookup_str(n, "source_file"))
        .unwrap_or_default();
    assert!(
        thing_source.ends_with("thing.go"),
        "Thing must be the thing.go definition, got {thing_source:?}"
    );
}
