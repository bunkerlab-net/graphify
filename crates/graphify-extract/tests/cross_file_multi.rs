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
    assert_ne!(result.nodes, Vec::<CorpusObj>::new());
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
    assert_eq!(result.nodes, Vec::<CorpusObj>::new());
    assert_eq!(result.edges, Vec::<CorpusObj>::new());
    assert_eq!(result.input_tokens, 0);
}

#[test]
fn extract_single_file_uses_parent_as_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("solo.py");
    fs::write(&path, "def x(): pass\n").expect("test invariant");
    let result = extract(&[path], None);
    assert_ne!(result.nodes, Vec::<CorpusObj>::new());
}

#[test]
fn extract_with_cache_root_uses_provided_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("cached.py");
    fs::write(&path, "def x(): pass\n").expect("test invariant");
    let result = extract(&[path], Some(tmp.path()));
    assert_ne!(result.nodes, Vec::<CorpusObj>::new());
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
fn python_module_qualified_call_resolves_extracted() {
    // `module.func()` where `module` is imported resolves to the callable that
    // module contains, with an EXTRACTED `calls` edge (#1883). A lowercase
    // module receiver was previously dropped alongside instance calls.
    let tmp = tempfile::tempdir().expect("tempdir");
    let mathlib = tmp.path().join("mathlib.py");
    let caller = tmp.path().join("caller.py");
    fs::write(&mathlib, "def compute(x):\n    return x * 2\n").expect("write mathlib");
    fs::write(
        &caller,
        "import mathlib\n\ndef use_qualified(n):\n    return mathlib.compute(n)\n",
    )
    .expect("write caller");
    let result = extract(&[caller, mathlib], Some(tmp.path()));
    let idx = index_nodes(&result.nodes);
    let call_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| {
            e.get("relation").and_then(|v| v.as_str()) == Some("calls")
                && endpoint_field(&idx, e, "source", "label").contains("use_qualified")
                && endpoint_field(&idx, e, "target", "label").contains("compute")
                && endpoint_field(&idx, e, "target", "source_file").contains("mathlib.py")
        })
        .collect();
    assert_eq!(
        call_edges.len(),
        1,
        "expected one use_qualified->compute edge, got {call_edges:?}"
    );
    assert_eq!(
        call_edges[0].get("confidence").and_then(|v| v.as_str()),
        Some("EXTRACTED")
    );
}

#[test]
fn python_module_qualified_call_requires_the_import() {
    // A `module.func()` call resolves only against a module the caller's own file
    // imports — a local instance `o.compute()` (o is a parameter) must NOT link
    // to a same-named function in another module (#1883 false-edge guard).
    let tmp = tempfile::tempdir().expect("tempdir");
    let mathlib = tmp.path().join("mathlib.py");
    let caller = tmp.path().join("caller.py");
    fs::write(&mathlib, "def compute(x):\n    return x * 2\n").expect("write mathlib");
    // no `import mathlib`; `o` is just a parameter that happens to expose compute()
    fs::write(&caller, "def via_obj(o):\n    return o.compute(3)\n").expect("write caller");
    let result = extract(&[caller, mathlib], Some(tmp.path()));
    let idx = index_nodes(&result.nodes);
    let bad: Vec<_> = result
        .edges
        .iter()
        .filter(|e| {
            e.get("relation").and_then(|v| v.as_str()) == Some("calls")
                && endpoint_field(&idx, e, "source", "label").contains("via_obj")
                && endpoint_field(&idx, e, "target", "label").contains("compute")
        })
        .collect();
    assert!(
        bad.is_empty(),
        "non-imported receiver must not link cross-file: {bad:?}"
    );
}

#[test]
fn rake_files_extract_and_resolve_like_rb() {
    // #1784: `.rake` files are plain Ruby — they route to the Ruby extractor and
    // participate in Ruby cross-file member resolution exactly like `.rb`.
    let tmp = tempfile::tempdir().expect("tempdir");
    let rake = tmp.path().join("ops.rake");
    let rb = tmp.path().join("widget.rb");
    fs::write(
        &rake,
        "class RakeHelper\n  def self.run\n    Widget.tally\n  end\nend\n",
    )
    .expect("write rake");
    fs::write(&rb, "class Widget\n  def self.tally\n    42\n  end\nend\n").expect("write rb");
    let result = extract(&[rake, rb], Some(tmp.path()));
    let labels: std::collections::HashSet<&str> = result
        .nodes
        .iter()
        .filter_map(|n| n.get("label").and_then(serde_json::Value::as_str))
        .collect();
    assert!(
        labels.contains("RakeHelper"),
        "RakeHelper node missing: {labels:?}"
    );
    assert!(labels.contains(".run()"), ".run() node missing: {labels:?}");
    let idx = index_nodes(&result.nodes);
    let resolved = result.edges.iter().any(|e| {
        e.get("relation").and_then(|v| v.as_str()) == Some("calls")
            && endpoint_field(&idx, e, "source", "label").contains(".run()")
            && endpoint_field(&idx, e, "target", "label").contains(".tally()")
    });
    assert!(
        resolved,
        "cross-file .rake -> .rb member call did not resolve: {:?}",
        result.edges
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
fn cpp_unresolved_base_class_stubs_stay_disambiguated_by_file() {
    // 9557bf6: two files each inheriting the same undefined base class must
    // produce two distinct SOURCELESS stubs (tagged with `origin_file`), not one
    // shared bare id that could collide with an unrelated same-named real
    // definition elsewhere in the corpus. Mirrors test_extract.py::
    // test_cpp_unresolved_base_class_stubs_stay_disambiguated_by_file.
    let tmp = tempfile::tempdir().expect("tempdir");
    let a = tmp.path().join("a");
    let b = tmp.path().join("b");
    fs::create_dir_all(&a).expect("create_dir_all");
    fs::create_dir_all(&b).expect("create_dir_all");
    fs::write(a.join("Foo.cpp"), "class Foo : public Base {};\n").expect("test invariant");
    fs::write(b.join("Bar.cpp"), "class Bar : public Base {};\n").expect("test invariant");

    let result = extract(&[a.join("Foo.cpp"), b.join("Bar.cpp")], Some(tmp.path()));
    let base_stubs: Vec<_> = result
        .nodes
        .iter()
        .filter(|n| {
            lookup_str(n, "label").as_deref() == Some("Base")
                && lookup_str(n, "source_file").unwrap_or_default().is_empty()
        })
        .collect();
    assert_eq!(
        base_stubs.len(),
        2,
        "expected two distinct Base stubs: {base_stubs:?}"
    );
    let ids: std::collections::HashSet<_> = base_stubs
        .iter()
        .filter_map(|n| lookup_str(n, "id"))
        .collect();
    assert_eq!(ids.len(), 2, "Base stubs must have distinct ids");

    let inherits: Vec<_> = result
        .edges
        .iter()
        .filter(|e| lookup_str(e, "relation").as_deref() == Some("inherits"))
        .collect();
    assert_eq!(
        inherits.len(),
        2,
        "expected two inherits edges: {inherits:?}"
    );
    let targets: std::collections::HashSet<_> = inherits
        .iter()
        .filter_map(|e| lookup_str(e, "target"))
        .collect();
    assert_eq!(
        targets.len(),
        2,
        "inherits edges must target distinct stubs"
    );
}

#[test]
fn mts_cts_route_to_typescript_grammar() {
    // 1226c34: `.mts` (ESM) / `.cts` (CommonJS) must parse under the TypeScript
    // grammar like `.ts` — the TS-only `type`/`interface` declarations below would
    // be dropped by the plain JS grammar, so their presence proves TS routing.
    const TS_SRC: &str = "export type Mode = 'a' | 'b';\n\
         export interface Options { mode: Mode; retries: number; }\n\
         export function greet(name: string): string { return `hi ${name}`; }\n\
         export class Widget { render(): void {} }\n";
    for ext in ["ts", "mts", "cts"] {
        let tmp = tempfile::tempdir().expect("tempdir");
        let f = tmp.path().join(format!("widget.{ext}"));
        fs::write(&f, TS_SRC).expect("test invariant");
        let result = extract(&[f], Some(tmp.path()));
        let labels: std::collections::HashSet<String> = result
            .nodes
            .iter()
            .filter_map(|n| lookup_str(n, "label"))
            .collect();
        assert!(
            labels.iter().any(|l| l.contains("Widget")),
            "{ext}: class Widget node missing: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("greet")),
            "{ext}: function greet node missing: {labels:?}"
        );
        // `interface` is TS-only — its presence proves the TS grammar was used.
        assert!(
            labels.iter().any(|l| l.contains("Options")),
            "{ext}: TS-only interface Options missing (JS grammar was used?): {labels:?}"
        );
    }
}

#[test]
fn cjs_routes_through_dispatch_like_js() -> Result<(), Box<dyn std::error::Error>> {
    // .cjs (explicit CommonJS) must route through the extractor dispatch to the
    // JS grammar exactly like .js — same node set modulo the file node's label.
    // Regression lock for the `.cjs` gap in CODE_EXTENSIONS / dispatch (#1922).
    const CJS_SRC: &str = "const path = require('path');\n\
         const { app, BrowserWindow } = require('electron');\n\
         class WindowManager {\n  open() { return new BrowserWindow(); }\n}\n\
         function createWindow() {\n  const manager = new WindowManager();\n  return manager.open();\n}\n\
         module.exports = { createWindow };\n";
    let non_file_labels =
        |ext: &str| -> Result<std::collections::HashSet<String>, Box<dyn std::error::Error>> {
            let tmp = tempfile::tempdir()?;
            let f = tmp.path().join(format!("main.{ext}"));
            fs::write(&f, CJS_SRC)?;
            let result = extract(&[f], Some(tmp.path()));
            Ok(result
                .nodes
                .iter()
                .filter_map(|n| lookup_str(n, "label"))
                .filter(|l| !l.ends_with(&format!(".{ext}")))
                .collect())
        };
    let cjs = non_file_labels("cjs")?;
    assert!(
        cjs.iter().any(|l| l.contains("WindowManager")),
        ".cjs class declaration missing — not parsed as JS: {cjs:?}"
    );
    assert!(
        cjs.iter().any(|l| l.contains("createWindow")),
        ".cjs function declaration missing — not parsed as JS: {cjs:?}"
    );
    assert_eq!(
        cjs,
        non_file_labels("js")?,
        ".cjs must extract identically to .js"
    );
    Ok(())
}

#[test]
fn js_module_path_resolves_mts_cts_direct_and_index() {
    // 1226c34 + divergence: a bare import must resolve to a `.mts`/`.cts` file
    // directly, AND to a `foo/index.mts` directory barrel (the latter is the
    // graphify-py inconsistency we fix — see resolve_js_module_path).
    use graphify_extract::tsconfig::resolve_js_module_path;
    let tmp = tempfile::tempdir().expect("tempdir");
    // Direct file: `./util` -> util.mts
    fs::write(tmp.path().join("util.mts"), "export const x = 1;\n").expect("write");
    let resolved = resolve_js_module_path(&tmp.path().join("util"));
    assert_eq!(
        resolved,
        tmp.path().join("util.mts"),
        "direct .mts resolution"
    );
    // Direct file: `./legacy` -> legacy.cts
    fs::write(tmp.path().join("legacy.cts"), "export const y = 2;\n").expect("write");
    let resolved = resolve_js_module_path(&tmp.path().join("legacy"));
    assert_eq!(
        resolved,
        tmp.path().join("legacy.cts"),
        "direct .cts resolution"
    );
    // Directory barrel: `./pkg` -> pkg/index.mts
    let pkg = tmp.path().join("pkg");
    fs::create_dir_all(&pkg).expect("mkdir");
    fs::write(pkg.join("index.mts"), "export const z = 3;\n").expect("write");
    let resolved = resolve_js_module_path(&pkg);
    assert_eq!(
        resolved,
        pkg.join("index.mts"),
        "directory index.mts resolution"
    );
    // Directory barrel: `./cjspkg` -> cjspkg/index.cts
    let cjspkg = tmp.path().join("cjspkg");
    fs::create_dir_all(&cjspkg).expect("mkdir");
    fs::write(cjspkg.join("index.cts"), "export const w = 4;\n").expect("write");
    let resolved = resolve_js_module_path(&cjspkg);
    assert_eq!(
        resolved,
        cjspkg.join("index.cts"),
        "directory index.cts resolution"
    );
}

#[test]
fn js_module_path_resolves_cjs_direct() -> Result<(), Box<dyn std::error::Error>> {
    // `.cjs` was added to `_JS_RESOLVE_EXTS` (#1922): a bare import must resolve
    // to a sibling `.cjs` file directly. (Unlike `.mts`/`.cts`, `.cjs` is NOT in
    // the directory-barrel index set — upstream left `_JS_INDEX_FILES` unchanged.)
    use graphify_extract::tsconfig::resolve_js_module_path;
    let tmp = tempfile::tempdir()?;
    fs::write(tmp.path().join("preload.cjs"), "module.exports = {};\n")?;
    let resolved = resolve_js_module_path(&tmp.path().join("preload"));
    assert_eq!(
        resolved,
        tmp.path().join("preload.cjs"),
        "direct .cjs resolution"
    );
    Ok(())
}

#[test]
fn matlab_m_not_force_parsed_as_objc() {
    // 733ad08: `.m` is shared by Objective-C and MATLAB. A real ObjC `.m` still
    // routes to extract_objc, but a MATLAB `.m` (no ObjC directive) must NOT be
    // force-parsed by the ObjC grammar (garbage) — it gets no extractor and
    // produces no nodes. `.mm` is unambiguously Objective-C++.
    let tmp = tempfile::tempdir().expect("tempdir");

    // MATLAB function file — no ObjC directive -> no extractor -> no nodes.
    let matlab_fn = tmp.path().join("solver.m");
    fs::write(&matlab_fn, "function y = solver(x)\n  y = x + 1;\nend\n").expect("write");
    let r = extract(&[matlab_fn], Some(tmp.path()));
    assert!(
        r.nodes.is_empty(),
        "MATLAB .m must yield no garbage nodes: {:?}",
        r.nodes
    );

    // MATLAB classdef file — likewise no extractor.
    let matlab_cls = tmp.path().join("Model.m");
    fs::write(
        &matlab_cls,
        "classdef Model\n  methods\n    function run(obj); end\n  end\nend\n",
    )
    .expect("write");
    let r = extract(&[matlab_cls], Some(tmp.path()));
    assert!(
        r.nodes.is_empty(),
        "MATLAB classdef .m must yield no nodes: {:?}",
        r.nodes
    );

    // A genuine ObjC `.m` (carries @implementation) still routes to extract_objc.
    let objc = tmp.path().join("Foo.m");
    fs::write(
        &objc,
        "#import \"Foo.h\"\n@implementation Foo\n- (void)bar {}\n@end\n",
    )
    .expect("write");
    let r = extract(&[objc], Some(tmp.path()));
    assert!(
        r.nodes
            .iter()
            .any(|n| lookup_str(n, "label").as_deref() == Some("Foo")),
        "ObjC .m must still extract the Foo class: {:?}",
        r.nodes
    );

    // A `.m` whose ONLY ObjC signal is `#import` (no @interface/@implementation)
    // must still route to extract_objc — this exercises the `#import` marker
    // (graphify-py's 5th, restored in Rust); with only the 4 `@`-directives it
    // would be misclassified as MATLAB and dropped.
    let import_only = tmp.path().join("Category.m");
    fs::write(&import_only, "#import \"Base.h\"\nvoid helper(void) {}\n").expect("write");
    let r = extract(&[import_only], Some(tmp.path()));
    assert!(
        !r.nodes.is_empty(),
        "a `#import`-only ObjC .m must route to extract_objc, not be dropped as MATLAB: {:?}",
        r.nodes
    );

    // `.mm` is unambiguously Objective-C++ and is never sniffed.
    let mm = tmp.path().join("x.mm");
    fs::write(&mm, "#import <F/F.h>\n@implementation X\n@end\n").expect("write");
    let r = extract(&[mm], Some(tmp.path()));
    assert!(
        r.nodes
            .iter()
            .any(|n| lookup_str(n, "label").as_deref() == Some("X")),
        ".mm must extract the X class: {:?}",
        r.nodes
    );
}

#[test]
fn case_insensitive_extension_collection_and_dispatch() {
    // #1671: files with capitalized/mixed-case extensions (`.PY`/`.JS`/`.Ts`) must
    // be COLLECTED and routed to the right extractor, not silently skipped. The
    // `.Ts` file additionally parses under the TypeScript grammar (a TS-only
    // `interface` survives) — a DIVERGENCE from graphify-py, whose internal
    // extract_js suffix check is case-sensitive and mis-parses `.Ts` as JS.
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(
        tmp.path().join("app.PY"),
        "class MyPythonClass:\n    pass\n",
    )
    .expect("write");
    fs::write(tmp.path().join("script.JS"), "function myJSFunction() {}\n").expect("write");
    fs::write(
        tmp.path().join("lib.Ts"),
        "export interface MyTSShape { id: number; }\nexport class MyTSClass {}\n",
    )
    .expect("write");

    // Collection is case-insensitive (classify_file already checks lowercase).
    let collected = graphify_detect::collect_files(tmp.path());
    let names: std::collections::HashSet<String> = collected
        .iter()
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_string))
        .collect();
    for f in ["app.PY", "script.JS", "lib.Ts"] {
        assert!(names.contains(f), "{f} must be collected: {names:?}");
    }

    // Dispatch is case-insensitive (get_extractor normalizes the extension).
    let result = extract(&collected, Some(tmp.path()));
    let labels: std::collections::HashSet<String> = result
        .nodes
        .iter()
        .filter_map(|n| lookup_str(n, "label"))
        .collect();
    assert!(
        labels.iter().any(|l| l.contains("MyPythonClass")),
        ".PY not extracted: {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l.contains("myJSFunction")),
        ".JS not extracted: {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l.contains("MyTSClass")),
        ".Ts class not extracted: {labels:?}"
    );
    // A TS-only `interface` proves `.Ts` parsed under the TypeScript grammar.
    assert!(
        labels.iter().any(|l| l.contains("MyTSShape")),
        ".Ts interface (TS grammar) missing: {labels:?}"
    );
}

#[test]
fn extensionless_shebang_scripts_route_to_extractor() {
    // 2ab0867: extensionless CLIs (devctl/manage) resolve their extractor from the
    // shebang, mirroring detect.classify_file — else detect labels them code and
    // extraction silently drops them. Interpreters with a real extractor (bash,
    // python, node via `env`, incl. `env -S`) extract; ones without (perl) and
    // files with no shebang stay unsupported (0 nodes).
    let tmp = tempfile::tempdir().expect("tempdir");

    // bash CLI via `#!/usr/bin/env bash`
    let devctl = tmp.path().join("devctl");
    fs::write(
        &devctl,
        "#!/usr/bin/env bash\nhelper() { echo hi; }\nmain() { helper; }\nmain \"$@\"\n",
    )
    .expect("write");
    let labels: std::collections::HashSet<String> = extract(&[devctl], Some(tmp.path()))
        .nodes
        .iter()
        .filter_map(|n| lookup_str(n, "label"))
        .collect();
    assert!(
        labels.iter().any(|l| l.contains("helper")),
        "bash CLI helper missing: {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l.contains("main")),
        "bash CLI main missing: {labels:?}"
    );

    // python via `#!/usr/bin/env python3`
    let manage = tmp.path().join("manage");
    fs::write(&manage, "#!/usr/bin/env python3\ndef run():\n    pass\n").expect("write");
    let py_labels: std::collections::HashSet<String> = extract(&[manage], Some(tmp.path()))
        .nodes
        .iter()
        .filter_map(|n| lookup_str(n, "label"))
        .collect();
    assert!(
        py_labels.iter().any(|l| l.contains("run")),
        "python CLI `run` function missing; got: {py_labels:?}"
    );

    // node via `#!/usr/bin/env node` (routes through the default JS path).
    let jscli = tmp.path().join("jsctl");
    fs::write(&jscli, "#!/usr/bin/env node\nfunction cli() {}\n").expect("write");
    assert!(
        extract(&[jscli], Some(tmp.path()))
            .nodes
            .iter()
            .any(|n| lookup_str(n, "label").is_some_and(|l| l.contains("cli"))),
        "`env node` CLI `cli` function missing"
    );

    // `env -S bash -eu` split-args form is handled by the shared shebang parser.
    let runner = tmp.path().join("runner");
    fs::write(&runner, "#!/usr/bin/env -S bash -eu\nsetup() { :; }\n").expect("write");
    assert!(
        extract(&[runner], Some(tmp.path()))
            .nodes
            .iter()
            .any(|n| lookup_str(n, "label").is_some_and(|l| l.contains("setup"))),
        "`env -S bash` CLI `setup` function missing"
    );

    // No usable shebang -> unsupported (0 nodes).
    let plain = tmp.path().join("LICENSE-COPY");
    fs::write(&plain, "plain text, no shebang\n").expect("write");
    assert!(
        extract(&[plain], Some(tmp.path())).nodes.is_empty(),
        "plain extensionless text must yield no nodes"
    );

    // Interpreter detect knows but has no extractor (perl) -> skipped, not mis-parsed.
    let perl = tmp.path().join("legacy");
    fs::write(&perl, "#!/usr/bin/env perl\nprint 1;\n").expect("write");
    assert!(
        extract(&[perl], Some(tmp.path())).nodes.is_empty(),
        "perl CLI (no extractor) must yield no nodes"
    );
}

#[test]
fn ts_js_generator_functions_are_nodes() {
    // 09aeb97: generator functions were invisible. The declaration form
    // `function* g()` (generator_function_declaration) must emit a node (via
    // `function_types`), and the expression form `const h = function*(){}`
    // (generator_function) must be captured when assigned (via the JS
    // function-value types). Covered under both JS and TypeScript.
    let has_label = |paths: &[std::path::PathBuf], sym: &str| -> bool {
        // Function-declaration nodes are labeled `name()`; const-bound expression
        // forms `name`. Accept either exact form.
        let want_call = format!("{sym}()");
        extract(paths, None)
            .nodes
            .iter()
            .any(|n| lookup_str(n, "label").is_some_and(|l| l == sym || l == want_call))
    };
    let tmp = tempfile::tempdir().expect("tempdir");

    // Declaration form, TypeScript.
    let gts = tmp.path().join("g.ts");
    fs::write(&gts, "export function* counter() { yield 1; yield 2; }\n").expect("write");
    assert!(
        has_label(&[gts], "counter"),
        "TS `function* counter` node missing"
    );

    // Declaration form, JavaScript.
    let gjs = tmp.path().join("g.js");
    fs::write(&gjs, "function* gen() { yield 42; }\n").expect("write");
    assert!(has_label(&[gjs], "gen"), "JS `function* gen` node missing");

    // Expression form assigned to a const, TypeScript.
    let hts = tmp.path().join("h.ts");
    fs::write(&hts, "export const stream = function* () { yield 'a'; };\n").expect("write");
    assert!(
        has_label(&[hts], "stream"),
        "TS `const stream = function*()` node missing"
    );

    // Expression form assigned to a const, JavaScript.
    let hjs = tmp.path().join("h.js");
    fs::write(&hjs, "const flow = function* () { yield 1; };\n").expect("write");
    assert!(
        has_label(&[hjs], "flow"),
        "JS `const flow = function*()` node missing"
    );

    // Declaration form, TypeScript TSX (the third JS-family config).
    let gtsx = tmp.path().join("g.tsx");
    fs::write(&gtsx, "export function* rows() { yield 1; }\n").expect("write");
    assert!(
        has_label(&[gtsx], "rows"),
        "TSX `function* rows` node missing"
    );
    // Async generator declaration (`async function* pages()`).
    let agts = tmp.path().join("ag.ts");
    fs::write(&agts, "export async function* pages() { yield 1; }\n").expect("write");
    assert!(
        has_label(&[agts], "pages"),
        "TS `async function* pages` node missing"
    );
}

#[test]
fn generator_declaration_is_call_boundary_and_contained() {
    // 09aeb97: a generator declaration is a function boundary — a call in its body
    // resolves FROM the generator (not the enclosing file), proving its body is
    // walked with generator-as-boundary — and the file `contains` it as a node.
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("g.ts");
    fs::write(
        &f,
        "function helper() {}\nfunction* producer() { helper(); yield 1; }\n",
    )
    .expect("write");
    let result = extract(&[f], Some(tmp.path()));
    let label = |id: &str| -> Option<String> {
        result
            .nodes
            .iter()
            .find(|n| lookup_str(n, "id").as_deref() == Some(id))
            .and_then(|n| lookup_str(n, "label"))
    };
    // Call from the generator body is attributed to the generator, not the file.
    assert!(
        result.edges.iter().any(|e| {
            lookup_str(e, "relation").as_deref() == Some("calls")
                && label(&lookup_str(e, "source").unwrap_or_default()).as_deref()
                    == Some("producer()")
                && label(&lookup_str(e, "target").unwrap_or_default()).as_deref()
                    == Some("helper()")
        }),
        "generator body call must resolve producer() -> helper(): {:?}",
        result.edges
    );
    // The file structurally `contains` the generator node.
    assert!(
        result.edges.iter().any(|e| {
            lookup_str(e, "relation").as_deref() == Some("contains")
                && label(&lookup_str(e, "source").unwrap_or_default()).as_deref() == Some("g.ts")
                && label(&lookup_str(e, "target").unwrap_or_default()).as_deref()
                    == Some("producer()")
        }),
        "file must `contains` the generator node"
    );
}

#[test]
fn ts_namespace_and_module_containers_are_nodes() {
    // 869aaf7: `namespace Foo {}` (internal_module) / `module Bar {}` (module) /
    // ambient `declare module "pkg" {}` emit a container node; nested names and
    // quote-stripped string names are handled; members stay extracted; TS-only.
    let has = |ext: &str, src: &str, sym: &str| -> bool {
        let tmp = tempfile::tempdir().expect("tempdir");
        let f = tmp.path().join(format!("n.{ext}"));
        fs::write(&f, src).expect("write");
        extract(&[f], None)
            .nodes
            .iter()
            .any(|n| lookup_str(n, "label").as_deref() == Some(sym))
    };
    // `namespace` -> internal_module container node.
    assert!(
        has(
            "ts",
            "export namespace Geometry { export const PI = 3.14; }\n",
            "Geometry"
        ),
        "namespace container node missing"
    );
    // `module` keyword -> module container node.
    assert!(
        has("ts", "module Legacy { export class Thing {} }\n", "Legacy"),
        "module container node missing"
    );
    // Nested namespace name (nested_identifier) is used verbatim as the label.
    assert!(
        has(
            "ts",
            "namespace App.Core.Util { export const v = 1; }\n",
            "App.Core.Util"
        ),
        "nested namespace name missing"
    );
    // Ambient string module -> surrounding quotes stripped from the label.
    assert!(
        has(
            "ts",
            "declare module \"pkg-name\" { export const z = 3; }\n",
            "pkg-name"
        ),
        "string module quotes not stripped"
    );
    // TS-only: the handler must not fire for plain JS (no namespace syntax).
    assert!(
        has("js", "function ok() {}\n", "ok()"),
        "plain JS extraction must be unaffected by the TS namespace handler"
    );

    // Members inside a namespace are still extracted alongside the container.
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("shapes.ts");
    fs::write(
        &f,
        "namespace Shapes {\n  export class Circle {}\n  export function area() { return 1; }\n}\n",
    )
    .expect("write");
    let labels: std::collections::HashSet<String> = extract(&[f], None)
        .nodes
        .iter()
        .filter_map(|n| lookup_str(n, "label"))
        .collect();
    assert!(
        labels.contains("Shapes"),
        "namespace container missing: {labels:?}"
    );
    assert!(
        labels.contains("Circle"),
        "namespace member class missing: {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l == "area" || l == "area()"),
        "namespace member function missing: {labels:?}"
    );
}

#[test]
fn ts_decorator_reference_edges() {
    // 3540416: `@Component`/`@Injectable`/`@Input`/`@Inject`/... emit a
    // `references[decorator]` edge from the decorated entity to the decorator
    // symbol. Class decorators -> class; method/param decorators -> the method;
    // field decorators -> the class; `@ns.Deco` uses the property name.
    use graphify_extract::{file_stem, make_id};
    use std::path::Path;
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("c.ts");
    fs::write(
        &f,
        "@Component({ selector: 'app' })\nexport class AppComponent {}\n\
         @Injectable()\nclass Service {}\n\
         @Injectable()\n@Entity()\nexport class Repo {}\n\
         @core.Component({})\nexport class Widget {}\n\
         class Ctrl {\n\
         \x20 @HostListener('click') onClick() {}\n\
         \x20 @Get('/') @UseGuards(Auth) list() {}\n\
         \x20 @Input() name: string;\n\
         \x20 @Column() age: number;\n\
         \x20 constructor(@Inject(TOKEN) private s: Svc) {}\n\
         }\n",
    )
    .expect("write");
    let result = extract(&[f], Some(tmp.path()));
    // Owner NIDs, reconstructed exactly like graphify-py `_class_nid`/`_method_nid`.
    let stem = file_stem(Path::new("c.ts"));
    let class_nid = |name: &str| make_id(&[&stem, name]);
    let method_nid = |cls: &str, m: &str| make_id(&[&make_id(&[&stem, cls]), m]);
    let id2label: std::collections::HashMap<String, String> = result
        .nodes
        .iter()
        .filter_map(|n| Some((lookup_str(n, "id")?, lookup_str(n, "label")?)))
        .collect();
    // (edge source NID, decorator target label) pairs.
    let decos: std::collections::HashSet<(String, String)> = result
        .edges
        .iter()
        .filter(|e| {
            lookup_str(e, "relation").as_deref() == Some("references")
                && lookup_str(e, "context").as_deref() == Some("decorator")
        })
        .filter_map(|e| {
            Some((
                lookup_str(e, "source")?,
                id2label.get(&lookup_str(e, "target")?)?.clone(),
            ))
        })
        .collect();
    let has = |owner: String, deco: &str| decos.contains(&(owner, deco.to_string()));
    // Class-level: plain, exported, stacked, namespaced (`@core.Component` -> property).
    assert!(
        has(class_nid("AppComponent"), "Component"),
        "exported class deco: {decos:?}"
    );
    assert!(
        has(class_nid("Service"), "Injectable"),
        "plain class deco: {decos:?}"
    );
    assert!(
        has(class_nid("Repo"), "Injectable") && has(class_nid("Repo"), "Entity"),
        "stacked class decos: {decos:?}"
    );
    assert!(
        has(class_nid("Widget"), "Component"),
        "namespaced deco -> property: {decos:?}"
    );
    // Method decorators attribute to the method, NOT the class.
    assert!(
        has(method_nid("Ctrl", "onClick"), "HostListener"),
        "method deco -> method: {decos:?}"
    );
    assert!(
        !has(class_nid("Ctrl"), "HostListener"),
        "method deco must not hit the class: {decos:?}"
    );
    assert!(
        has(method_nid("Ctrl", "list"), "Get") && has(method_nid("Ctrl", "list"), "UseGuards"),
        "stacked method decos: {decos:?}"
    );
    // Field decorators attribute to the class; parameter decorators to the constructor.
    assert!(
        has(class_nid("Ctrl"), "Input") && has(class_nid("Ctrl"), "Column"),
        "field decos -> class: {decos:?}"
    );
    assert!(
        has(method_nid("Ctrl", "constructor"), "Inject"),
        "param deco -> constructor: {decos:?}"
    );
}

#[test]
fn ts_decorator_external_stub_disambiguated_per_file() {
    // 3540416: an unresolved decorator symbol (`@Injectable` with no local def)
    // becomes a per-file stub — files a.ts and b.ts each emit exactly one edge to
    // an `Injectable`-labelled node, but the target ids differ (no phantom merge).
    use graphify_extract::{file_stem, make_id};
    use std::path::Path;
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).expect("create_dir_all");
    let a = src.join("a.ts");
    let b = src.join("b.ts");
    fs::write(&a, "@Injectable()\nexport class A {}\n").expect("write");
    fs::write(&b, "@Injectable()\nexport class B {}\n").expect("write");
    let result = extract(&[a, b], Some(tmp.path()));
    let id2label: std::collections::HashMap<String, String> = result
        .nodes
        .iter()
        .filter_map(|n| Some((lookup_str(n, "id")?, lookup_str(n, "label")?)))
        .collect();
    // Decorator targets keyed by the owning class NID.
    let targets = |owner: &str| -> Vec<String> {
        result
            .edges
            .iter()
            .filter(|e| {
                lookup_str(e, "relation").as_deref() == Some("references")
                    && lookup_str(e, "context").as_deref() == Some("decorator")
                    && lookup_str(e, "source").as_deref() == Some(owner)
            })
            .filter_map(|e| lookup_str(e, "target"))
            .collect()
    };
    let a_nid = make_id(&[&file_stem(Path::new("src/a.ts")), "A"]);
    let b_nid = make_id(&[&file_stem(Path::new("src/b.ts")), "B"]);
    let a_targets = targets(&a_nid);
    let b_targets = targets(&b_nid);
    assert_eq!(a_targets.len(), 1, "a.ts one decorator edge: {a_targets:?}");
    assert_eq!(b_targets.len(), 1, "b.ts one decorator edge: {b_targets:?}");
    assert_eq!(
        id2label.get(&a_targets[0]).map(String::as_str),
        Some("Injectable")
    );
    assert_eq!(
        id2label.get(&b_targets[0]).map(String::as_str),
        Some("Injectable")
    );
    assert_ne!(
        a_targets[0], b_targets[0],
        "external stubs must be per-file distinct"
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

// ── Java receiver-typed member-call resolution (dae602c / #1696) ──────────────

const JAVA_AMBIGUOUS_METHODS: &str = "class PaymentGateway { static void ping() {} void charge() {} }\n\
     class AuditLog { static void ping() {} void charge() {} }\n";

/// Write `files` under `tmp`, extract, and return the set of `(source, target)`
/// `calls` edges plus the output. Mirrors Python `_calls`.
fn java_calls(
    tmp: &Path,
    files: &[(&str, &str)],
) -> (
    std::collections::HashSet<(String, String)>,
    graphify_extract::ExtractOutput,
) {
    let mut paths = Vec::new();
    for (name, body) in files {
        let path = tmp.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(&path, body).expect("write");
        paths.push(path);
    }
    let result = extract(&paths, Some(tmp));
    let calls = result
        .edges
        .iter()
        .filter(|e| lookup_str(e, "relation").as_deref() == Some("calls"))
        .filter_map(|e| Some((lookup_str(e, "source")?, lookup_str(e, "target")?)))
        .collect();
    (calls, result)
}

/// Node id whose label == `label` and whose id contains `id_contains`. Mirrors
/// Python `_find`.
fn jfind(result: &graphify_extract::ExtractOutput, label: &str, id_contains: &str) -> String {
    result
        .nodes
        .iter()
        .find(|n| {
            lookup_str(n, "label").as_deref() == Some(label)
                && lookup_str(n, "id").is_some_and(|id| id.contains(id_contains))
        })
        .and_then(|n| lookup_str(n, "id"))
        .unwrap_or_else(|| panic!("node not found: label={label:?} id~{id_contains:?}"))
}

#[test]
fn java_explicit_type_receiver_resolves_to_owned_method() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, result) = java_calls(
        tmp.path(),
        &[
            ("Services.java", JAVA_AMBIGUOUS_METHODS),
            (
                "Checkout.java",
                "class Checkout { void run() { PaymentGateway.ping(); } }\n",
            ),
        ],
    );
    let run = jfind(&result, ".run()", "checkout");
    let gateway_ping = jfind(&result, ".ping()", "paymentgateway");
    let audit_ping = jfind(&result, ".ping()", "auditlog");
    assert!(
        calls.contains(&(run.clone(), gateway_ping)),
        "run -> PaymentGateway.ping"
    );
    assert!(
        !calls.contains(&(run, audit_ping)),
        "must not hit AuditLog.ping"
    );
}

/// #1671: uppercase `.JAVA` files still resolve cross-file member calls.
#[test]
fn java_uppercase_ext_member_call_resolves() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, result) = java_calls(
        tmp.path(),
        &[
            ("Services.JAVA", JAVA_AMBIGUOUS_METHODS),
            (
                "Checkout.JAVA",
                "class Checkout { void run() { PaymentGateway.ping(); } }\n",
            ),
        ],
    );
    let run = jfind(&result, ".run()", "checkout");
    let gateway_ping = jfind(&result, ".ping()", "paymentgateway");
    assert!(
        calls.contains(&(run, gateway_ping)),
        "uppercase-.JAVA run -> PaymentGateway.ping: {calls:?}"
    );
}

#[test]
fn java_field_receiver_resolves_to_declared_type() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, result) = java_calls(
        tmp.path(),
        &[
            ("Services.java", JAVA_AMBIGUOUS_METHODS),
            (
                "Checkout.java",
                "class Checkout {\n    void run() { gateway.charge(); }\n    PaymentGateway gateway;\n}\n",
            ),
        ],
    );
    let run = jfind(&result, ".run()", "checkout");
    let gateway_charge = jfind(&result, ".charge()", "paymentgateway");
    let audit_charge = jfind(&result, ".charge()", "auditlog");
    assert!(
        calls.contains(&(run.clone(), gateway_charge)),
        "field-typed receiver"
    );
    assert!(
        !calls.contains(&(run, audit_charge)),
        "must not hit AuditLog.charge"
    );
}

#[test]
fn java_this_field_receiver_resolves_to_declared_type() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, result) = java_calls(
        tmp.path(),
        &[
            ("Services.java", JAVA_AMBIGUOUS_METHODS),
            (
                "Checkout.java",
                "class Checkout {\n    PaymentGateway gateway;\n    void run() { this.gateway.charge(); }\n}\n",
            ),
        ],
    );
    let run = jfind(&result, ".run()", "checkout");
    let gateway_charge = jfind(&result, ".charge()", "paymentgateway");
    assert!(
        calls.contains(&(run, gateway_charge)),
        "this.field-typed receiver"
    );
}

#[test]
fn java_this_field_uses_field_type_when_parameter_shadows_name() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, result) = java_calls(
        tmp.path(),
        &[
            ("Services.java", JAVA_AMBIGUOUS_METHODS),
            (
                "Checkout.java",
                "class Checkout {\n    PaymentGateway service;\n    void run(AuditLog service) {\n        service.charge();\n        this.service.charge();\n    }\n}\n",
            ),
        ],
    );
    let run = jfind(&result, ".run()", "checkout");
    let gateway_charge = jfind(&result, ".charge()", "paymentgateway");
    let audit_charge = jfind(&result, ".charge()", "auditlog");
    // Parameter shadows the field for a bare `service`; `this.service` keeps the field type.
    assert!(
        calls.contains(&(run.clone(), gateway_charge)),
        "this.service -> field type"
    );
    assert!(
        calls.contains(&(run, audit_charge)),
        "service -> parameter type"
    );
}

#[test]
fn java_parameter_and_local_receivers_resolve_per_method() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, result) = java_calls(
        tmp.path(),
        &[
            ("Services.java", JAVA_AMBIGUOUS_METHODS),
            (
                "Checkout.java",
                "class Checkout {\n    void fromParameter(PaymentGateway service) { service.charge(); }\n    void fromLocal() { AuditLog service = new AuditLog(); service.charge(); }\n}\n",
            ),
        ],
    );
    let from_parameter = jfind(&result, ".fromParameter()", "checkout");
    let from_local = jfind(&result, ".fromLocal()", "checkout");
    let gateway_charge = jfind(&result, ".charge()", "paymentgateway");
    let audit_charge = jfind(&result, ".charge()", "auditlog");
    assert!(calls.contains(&(from_parameter.clone(), gateway_charge.clone())));
    assert!(!calls.contains(&(from_parameter, audit_charge.clone())));
    assert!(calls.contains(&(from_local.clone(), audit_charge)));
    assert!(!calls.contains(&(from_local, gateway_charge)));
}

#[test]
fn java_nested_receiver_bindings_do_not_escape_their_scope() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, result) = java_calls(
        tmp.path(),
        &[
            ("Services.java", JAVA_AMBIGUOUS_METHODS),
            (
                "Checkout.java",
                "class Checkout {\n    PaymentGateway service;\n    void blockLocal() {\n        service.charge();\n        { AuditLog service = null; service.charge(); }\n    }\n    void anonymousClass() {\n        new Object() { void nested() { AuditLog service = null; } };\n        service.charge();\n    }\n}\n",
            ),
        ],
    );
    let block_local = jfind(&result, ".blockLocal()", "checkout");
    let anonymous_class = jfind(&result, ".anonymousClass()", "checkout");
    let gateway_charge = jfind(&result, ".charge()", "paymentgateway");
    let audit_charge = jfind(&result, ".charge()", "auditlog");
    // A block-local of a different type makes the name ambiguous -> no edge.
    assert!(
        !calls
            .iter()
            .any(|(s, t)| *s == block_local && t.contains("charge")),
        "block-local shadow must not resolve"
    );
    // A binding inside a nested (anonymous) class does not leak out.
    assert!(
        calls.contains(&(anonymous_class.clone(), gateway_charge)),
        "field survives nested class"
    );
    assert!(!calls.contains(&(anonymous_class, audit_charge)));
}

#[test]
fn java_lambda_shadowing_does_not_reuse_enclosing_receiver_type() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, result) = java_calls(
        tmp.path(),
        &[
            ("Services.java", JAVA_AMBIGUOUS_METHODS),
            (
                "Checkout.java",
                "class Checkout {\n    PaymentGateway service;\n    void captured() {\n        Runnable task = () -> service.charge();\n    }\n    void shadowed() {\n        java.util.function.Consumer<AuditLog> task =\n            service -> service.charge();\n    }\n    void parenthesized() {\n        java.util.function.Consumer<AuditLog> task =\n            (service) -> service.charge();\n    }\n    void typed() {\n        java.util.function.Consumer<AuditLog> task =\n            (AuditLog service) -> service.charge();\n    }\n    void sameType() {\n        java.util.function.Consumer<PaymentGateway> task =\n            (PaymentGateway service) -> service.charge();\n    }\n}\n",
            ),
        ],
    );
    let captured = jfind(&result, ".captured()", "checkout");
    let same_type = jfind(&result, ".sameType()", "checkout");
    let gateway_charge = jfind(&result, ".charge()", "paymentgateway");
    let shadowed: std::collections::HashSet<String> = ["shadowed", "parenthesized", "typed"]
        .iter()
        .map(|n| jfind(&result, &format!(".{n}()"), "checkout"))
        .collect();
    // A captured field and a same-type lambda param resolve; a differently-typed
    // or untyped lambda param shadows the name and must resolve to nothing.
    assert!(
        calls.contains(&(captured, gateway_charge.clone())),
        "captured field"
    );
    assert!(
        calls.contains(&(same_type, gateway_charge)),
        "same-type lambda param"
    );
    assert!(
        !calls
            .iter()
            .any(|(s, t)| shadowed.contains(s) && t.contains("charge")),
        "shadowing lambda params must not reuse the field type"
    );
}

#[test]
fn java_overloaded_callers_keep_body_scoped_receiver_types() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, result) = java_calls(
        tmp.path(),
        &[
            ("Services.java", JAVA_AMBIGUOUS_METHODS),
            (
                "Checkout.java",
                "class Checkout {\n    void run(int value) { PaymentGateway service = null; service.charge(); }\n    void run(String value) { AuditLog service = null; service.charge(); }\n}\n",
            ),
        ],
    );
    // Overloads collapse to one `run` NID, but each body's local type resolves.
    let run = jfind(&result, ".run()", "checkout");
    let gateway_charge = jfind(&result, ".charge()", "paymentgateway");
    let audit_charge = jfind(&result, ".charge()", "auditlog");
    assert!(
        calls.contains(&(run.clone(), gateway_charge)),
        "int overload local"
    );
    assert!(
        calls.contains(&(run, audit_charge)),
        "String overload local"
    );
}

#[test]
fn java_ambiguous_receiver_type_emits_no_edge() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, result) = java_calls(
        tmp.path(),
        &[
            (
                "a/Gateway.java",
                "package a; public class Gateway { public void send() {} }\n",
            ),
            (
                "b/Gateway.java",
                "package b; public class Gateway { public void send() {} }\n",
            ),
            (
                "Caller.java",
                "class Caller { void run(Gateway gateway) { gateway.send(); } }\n",
            ),
        ],
    );
    let run = jfind(&result, ".run()", "caller");
    // Two same-named target types -> the god-node guard refuses to guess.
    assert!(
        !calls.iter().any(|(s, t)| *s == run && t.contains("send")),
        "ambiguous receiver type must not resolve"
    );
}

#[test]
fn java_inherited_field_and_chained_receiver_are_deferred() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, result) = java_calls(
        tmp.path(),
        &[(
            "Services.java",
            "class Gateway { void charge() {} Gateway create() { return this; } }\nclass Base { Gateway gateway; }\nclass Checkout extends Base {\n    Gateway factory;\n    void inherited() { this.gateway.charge(); }\n    void chained() { factory.create().charge(); }\n}\n",
        )],
    );
    let inherited = jfind(&result, ".inherited()", "checkout");
    let chained = jfind(&result, ".chained()", "checkout");
    // Inherited fields and chained receivers need type identity we don't track.
    assert!(
        !calls
            .iter()
            .any(|(s, t)| (*s == inherited || *s == chained) && t.contains("charge")),
        "inherited/chained receivers must stay deferred"
    );
}

#[test]
fn java_unqualified_call_still_resolves() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, result) = java_calls(
        tmp.path(),
        &[(
            "Checkout.java",
            "class Checkout {\n    void run() { helper(); this.other(); }\n    void helper() {}\n    void other() {}\n}\n",
        )],
    );
    let run = jfind(&result, ".run()", "checkout");
    let helper = jfind(&result, ".helper()", "checkout");
    let other = jfind(&result, ".other()", "checkout");
    // A bare call resolves by name; `this.other()` binds to the enclosing type.
    assert!(calls.contains(&(run.clone(), helper)), "unqualified call");
    assert!(
        calls.contains(&(run, other)),
        "this.method() via enclosing type"
    );
}

// ── C# receiver-typed member-call resolution (eebc406 / #1609) ────────────────

const CS_AMBIG: &str = "public class Server { public bool Save() => true; }\n\
     public class Cache  { public bool Save() => false; }\n\
     public class Repo {\n\
     \x20   private Server _server = new Server();\n\
     \x20   public bool Commit() { return _server.Save(); }\n\
     }\n";

#[test]
fn csharp_field_receiver_resolves_to_declared_type_not_bare_match() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, result) = java_calls(tmp.path(), &[("S.cs", CS_AMBIG)]);
    let commit = jfind(&result, ".Commit()", "commit");
    let server_save = jfind(&result, ".Save()", "server");
    let cache_save = jfind(&result, ".Save()", "cache");
    assert!(
        calls.contains(&(commit.clone(), server_save)),
        "field.Method() -> field type"
    );
    assert!(
        !calls.contains(&(commit, cache_save)),
        "must not mis-bind a same-named method"
    );
}

#[test]
fn csharp_parameter_receiver_resolves() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, _) = java_calls(
        tmp.path(),
        &[(
            "S.cs",
            "public class Server { public bool Save() => true; }\n\
             public class Cache  { public bool Save() => false; }\n\
             public class Svc { public static bool Copy(Server server) { return server.Save(); } }\n",
        )],
    );
    assert!(
        calls
            .iter()
            .any(|(s, t)| s.contains("copy") && t.contains("server_save"))
    );
    assert!(
        !calls
            .iter()
            .any(|(s, t)| s.contains("copy") && t.contains("cache_save"))
    );
}

#[test]
fn csharp_local_var_receiver_resolves() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, _) = java_calls(
        tmp.path(),
        &[(
            "S.cs",
            "public class Server { public bool Save() => true; }\n\
             public class R {\n\
             \x20   public bool A() { Server s = new Server(); return s.Save(); }\n\
             \x20   public bool B() { var v = new Server(); return v.Save(); }\n\
             }\n",
        )],
    );
    assert!(
        calls
            .iter()
            .any(|(s, t)| s.contains("_r_a") && t.contains("server_save")),
        "explicit-typed local"
    );
    assert!(
        calls
            .iter()
            .any(|(s, t)| s.contains("_r_b") && t.contains("server_save")),
        "var = new T() local"
    );
}

/// #1671: an uppercase `.CS` file is dispatched and extracted as C#, so its
/// cross-file member calls must resolve too — the resolver's suffix guards are
/// case-insensitive. A case-sensitive `.cs` guard would silently drop the edge.
#[test]
fn csharp_uppercase_ext_member_call_resolves() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, _) = java_calls(
        tmp.path(),
        &[(
            "S.CS",
            "public class Server { public bool Save() => true; }\n\
             public class R {\n\
             \x20   public bool B() { var v = new Server(); return v.Save(); }\n\
             }\n",
        )],
    );
    assert!(
        calls
            .iter()
            .any(|(s, t)| s.contains("_r_b") && t.contains("server_save")),
        "uppercase-.CS member call must resolve: {calls:?}"
    );
}

#[test]
fn csharp_cross_file_receiver_resolves() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, _) = java_calls(
        tmp.path(),
        &[
            (
                "Server.cs",
                "public class Server { public bool Save() => true; }\n\
                 public class Cache  { public bool Save() => false; }\n",
            ),
            (
                "Repo.cs",
                "public class Repo { private Server _s = new Server(); public bool Commit() { return _s.Save(); } }\n",
            ),
        ],
    );
    assert!(
        calls
            .iter()
            .any(|(s, t)| s.contains("commit") && t.contains("server_save"))
    );
    assert!(
        !calls
            .iter()
            .any(|(s, t)| s.contains("commit") && t.contains("cache_save"))
    );
}

#[test]
fn csharp_this_and_static_receivers() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, _) = java_calls(
        tmp.path(),
        &[(
            "S.cs",
            "public class Util { public static int F() => 1; }\n\
             public class R {\n\
             \x20   public bool A() { return this.B(); }\n\
             \x20   public bool B() => true;\n\
             \x20   public int G() { return Util.F(); }\n\
             }\n",
        )],
    );
    assert!(
        calls
            .iter()
            .any(|(s, t)| s.contains("_r_a") && t.contains("_r_b")),
        "this.B() -> R.B"
    );
    assert!(
        calls
            .iter()
            .any(|(s, t)| s.contains("_r_g") && t.contains("util_f")),
        "Util.F() -> Util.F"
    );
}

#[test]
fn csharp_untyped_receiver_emits_no_edge() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, _) = java_calls(
        tmp.path(),
        &[(
            "S.cs",
            "public class Server { public bool Save() => true; }\n\
             public class R { public bool C(dynamic x) { return x.Save(); } }\n",
        )],
    );
    assert!(
        !calls.iter().any(|(_, t)| t.to_lowercase().contains("save")),
        "dynamic receiver must not resolve"
    );
}

#[test]
fn csharp_method_absent_on_type_emits_no_edge() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, _) = java_calls(
        tmp.path(),
        &[(
            "S.cs",
            "public class Server { public bool Save() => true; }\n\
             public class R { private Server _s = new Server(); public bool C() { return _s.Missing(); } }\n",
        )],
    );
    assert!(
        !calls
            .iter()
            .any(|(s, t)| s.contains("_r_c") && t.to_lowercase().contains("save")),
        "receiver typed but no such method -> no edge"
    );
}

#[test]
fn csharp_unqualified_call_still_resolves() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (calls, _) = java_calls(
        tmp.path(),
        &[(
            "S.cs",
            "public class R { public bool A() { Helper(); return true; } private void Helper() {} }\n",
        )],
    );
    assert!(
        calls
            .iter()
            .any(|(s, t)| s.contains("_r_a") && t.contains("helper")),
        "no regression on unqualified calls"
    );
}

// ── 3bc3fee (#1547/#1556): header/impl class merge + .h C++/ObjC routing ──────

type CorpusObj = indexmap::IndexMap<String, serde_json::Value>;

/// Write `(relpath, content)` fixtures into a tempdir (preserving nested dirs)
/// and run the full corpus `extract()`. The tempdir is returned so the caller
/// keeps it alive for the duration of the assertions.
fn corpus(files: &[(&str, &str)]) -> (tempfile::TempDir, graphify_extract::ExtractOutput) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut paths = Vec::new();
    for (rel, content) in files {
        let p = tmp.path().join(rel);
        fs::create_dir_all(p.parent().expect("fixture rel has a parent")).expect("create_dir_all");
        fs::write(&p, content).expect("write fixture");
        paths.push(p);
    }
    let out = extract(&paths, Some(tmp.path()));
    (tmp, out)
}

fn nodes_with_label<'a>(
    out: &'a graphify_extract::ExtractOutput,
    label: &str,
) -> Vec<&'a CorpusObj> {
    out.nodes
        .iter()
        .filter(|n| n.get("label").and_then(serde_json::Value::as_str) == Some(label))
        .collect()
}

fn cf_str<'a>(m: &'a CorpusObj, key: &str) -> &'a str {
    m.get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

/// Every edge endpoint must reference a real node.
fn assert_no_dangling(out: &graphify_extract::ExtractOutput) {
    let ids: std::collections::HashSet<&str> = out
        .nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(serde_json::Value::as_str))
        .collect();
    for e in &out.edges {
        assert!(ids.contains(cf_str(e, "source")), "dangling source: {e:?}");
        assert!(ids.contains(cf_str(e, "target")), "dangling target: {e:?}");
    }
}

const FOO_H: &str = "#ifndef FOO_H\n#define FOO_H\n\nclass Foo {\npublic:\n    void bar();\n    int value;\n};\n\n#endif\n";
const FOO_CPP: &str = "#include \"Foo.h\"\n\nvoid Foo::bar() {\n    value = 1;\n}\n";
const MAIN_CPP: &str =
    "#include \"Foo.h\"\n\nint main() {\n    Foo f;\n    f.bar();\n    return 0;\n}\n";
const WIDGET_H: &str = "@interface Widget\n- (void)render;\n- (void)refresh;\n@end\n";
const WIDGET_M: &str = "#import \"Widget.h\"\n\n@implementation Widget\n- (void)render {\n    [self refresh];\n}\n- (void)refresh {\n}\n@end\n";
const BRIDGING_H: &str = "#import \"Widget.h\"\n";
const WIDGET_EXTRAS_SWIFT: &str =
    "extension Widget {\n    func describe() -> String {\n        return \"widget\"\n    }\n}\n";

/// #1547: a `.h` with a C++ class routes to `extract_cpp` (the C grammar has no
/// `class_specifier` and would drop the class). Observable: `Foo.h` alone yields
/// exactly one `Foo` class node and no junk `class` stub / `foo_foo` node.
#[test]
fn cpp_header_routes_to_cpp_extractor() {
    let (_tmp, out) = corpus(&[("Foo.h", FOO_H)]);
    let foos = nodes_with_label(&out, "Foo");
    assert_eq!(foos.len(), 1, "expected one Foo class node (cpp routing)");
    assert!(
        nodes_with_label(&out, "class").is_empty(),
        "no `class` stub"
    );
    assert!(
        nodes_with_label(&out, "foo_foo").is_empty(),
        "no junk foo_foo node"
    );
}

/// #1547: a plain C header (no C++ signal) keeps `extract_c` routing. Observable:
/// the C grammar has no struct/class member extraction, so `struct Point { int x;
/// int y; }` yields NO `x`/`y` field member nodes — whereas `extract_cpp` WOULD
/// emit them via its `field_declaration` handler (verified: cpp yields `Point`,
/// `x`, `y`; c yields only the file node). Their absence proves C routing.
#[test]
fn plain_c_header_stays_on_c_extractor() {
    let plain = "#ifndef PLAIN_H\n#define PLAIN_H\n\nint add(int a, int b);\nstruct Point { int x; int y; };\n\n#endif\n";
    let (_tmp, out) = corpus(&[("plain.h", plain)]);
    assert!(
        nodes_with_label(&out, "x").is_empty() && nodes_with_label(&out, "y").is_empty(),
        "plain C header must not be parsed as C++ (no struct member field nodes)"
    );
    assert!(
        nodes_with_label(&out, "class").is_empty(),
        "no `class` stub"
    );
}

/// #1547: `Foo.h` (class) + `Foo.cpp` (`Foo::bar` def) + `Main.cpp` yield exactly
/// ONE `Foo` class node — not a `foo_h` + `foo_cpp` pair — and no junk `class` stub.
#[test]
fn cpp_paired_single_class_node() {
    let (_tmp, out) = corpus(&[
        ("Foo.h", FOO_H),
        ("Foo.cpp", FOO_CPP),
        ("Main.cpp", MAIN_CPP),
    ]);
    let foos = nodes_with_label(&out, "Foo");
    assert_eq!(foos.len(), 1, "expected one Foo, got {foos:?}");
    assert!(
        nodes_with_label(&out, "class").is_empty(),
        "no sourceless `class` stub"
    );
    assert_eq!(nodes_with_label(&out, "foo_foo"), Vec::<&CorpusObj>::new());
}

/// #1547: `void bar();` in `Foo.h` and `void Foo::bar() {}` in `Foo.cpp` collapse
/// to ONE method node owned by the single `Foo` class.
#[test]
fn cpp_paired_method_decl_and_def_are_one_node() {
    let (_tmp, out) = corpus(&[
        ("Foo.h", FOO_H),
        ("Foo.cpp", FOO_CPP),
        ("Main.cpp", MAIN_CPP),
    ]);
    let foo_id = nodes_with_label(&out, "Foo")[0]
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let bar_ids: std::collections::HashSet<&str> = out
        .nodes
        .iter()
        .filter(|n| matches!(cf_str(n, "label"), "bar" | "Foo::bar()"))
        .filter_map(|n| n.get("id").and_then(serde_json::Value::as_str))
        .collect();
    assert_eq!(
        bar_ids.len(),
        1,
        "bar decl/def should be one node, got {bar_ids:?}"
    );
    let member_targets: std::collections::HashSet<&str> = out
        .edges
        .iter()
        .filter(|e| {
            cf_str(e, "source") == foo_id
                && matches!(cf_str(e, "relation"), "method" | "defines" | "contains")
        })
        .map(|e| cf_str(e, "target"))
        .collect();
    assert!(
        bar_ids.iter().any(|b| member_targets.contains(b)),
        "the merged bar node should be a member of Foo"
    );
}

/// #1547: `Foo.cpp` and `Main.cpp` `#include "Foo.h"` resolve to the real `Foo.h`
/// file node (no dangling import).
#[test]
fn cpp_paired_includes_resolve_to_real_header() {
    let (_tmp, out) = corpus(&[
        ("Foo.h", FOO_H),
        ("Foo.cpp", FOO_CPP),
        ("Main.cpp", MAIN_CPP),
    ]);
    let ids: std::collections::HashSet<&str> = out
        .nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(serde_json::Value::as_str))
        .collect();
    let foo_h = nodes_with_label(&out, "Foo.h")[0]
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let imports: Vec<&CorpusObj> = out
        .edges
        .iter()
        .filter(|e| cf_str(e, "relation") == "imports")
        .collect();
    assert!(
        imports.len() >= 2,
        "expected >=2 include imports, got {}",
        imports.len()
    );
    for e in &imports {
        assert!(
            ids.contains(cf_str(e, "target")),
            "dangling import target: {e:?}"
        );
    }
    assert!(
        imports.iter().any(|e| cf_str(e, "target") == foo_h),
        "includes should target Foo.h"
    );
}

/// #1547: the C++ paired corpus has no dangling edges after the merge.
#[test]
fn cpp_paired_no_dangling_edges() {
    let (_tmp, out) = corpus(&[
        ("Foo.h", FOO_H),
        ("Foo.cpp", FOO_CPP),
        ("Main.cpp", MAIN_CPP),
    ]);
    assert_no_dangling(&out);
}

/// #1556: a bridging header of only `#import "X.h"` routes to `extract_objc`
/// (`extract_c` parses `#import` as a `preproc_call` and drops the edge).
/// Observable: the bridging header alone emits an `imports` edge.
#[test]
fn objc_header_with_import_routes_to_objc() {
    let (_tmp, out) = corpus(&[("Bridging-Header.h", BRIDGING_H)]);
    let bridge = nodes_with_label(&out, "Bridging-Header.h");
    assert_eq!(bridge.len(), 1, "expected the bridging header file node");
    let bridge_id = bridge[0]
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    assert!(
        out.edges
            .iter()
            .any(|e| cf_str(e, "source") == bridge_id && cf_str(e, "relation") == "imports"),
        "bridging header must emit an imports edge (objc routing, not preproc_call)"
    );
}

/// #1556: `Widget.h` (@interface) + `Widget.m` (@implementation) yield ONE Widget
/// class node with each method present once.
#[test]
fn objc_paired_single_class_methods_not_duplicated() {
    let (_tmp, out) = corpus(&[("Widget.h", WIDGET_H), ("Widget.m", WIDGET_M)]);
    assert_eq!(
        nodes_with_label(&out, "Widget").len(),
        1,
        "expected one Widget"
    );
    assert_eq!(
        nodes_with_label(&out, "-render").len(),
        1,
        "-render duplicated"
    );
    assert_eq!(
        nodes_with_label(&out, "-refresh").len(),
        1,
        "-refresh duplicated"
    );
}

/// #1556: a bridging header of only `#import "Widget.h"` emits an `imports` edge
/// to the real `Widget.h` node (not an isolated node).
#[test]
fn objc_bridging_header_not_isolated() {
    let (_tmp, out) = corpus(&[
        ("Widget.h", WIDGET_H),
        ("Widget.m", WIDGET_M),
        ("Bridging-Header.h", BRIDGING_H),
    ]);
    let bridge = nodes_with_label(&out, "Bridging-Header.h")[0]
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let widget_h = nodes_with_label(&out, "Widget.h")[0]
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let out_edges: Vec<&CorpusObj> = out
        .edges
        .iter()
        .filter(|e| cf_str(e, "source") == bridge && cf_str(e, "relation") == "imports")
        .collect();
    assert!(
        !out_edges.is_empty(),
        "bridging header should emit an imports edge"
    );
    assert!(
        out_edges.iter().any(|e| cf_str(e, "target") == widget_h),
        "bridging import should target Widget.h"
    );
}

/// #1556: the Objective-C paired + bridging corpus has no dangling edges.
#[test]
fn objc_paired_no_dangling_edges() {
    let (_tmp, out) = corpus(&[
        ("Widget.h", WIDGET_H),
        ("Widget.m", WIDGET_M),
        ("Bridging-Header.h", BRIDGING_H),
    ]);
    assert_no_dangling(&out);
}

/// #1556: a Swift `extension Widget` over the Objective-C `Widget` folds onto the single
/// canonical Widget node, with its member anchored there.
#[test]
fn swift_extension_folds_onto_objc_class() {
    let (_tmp, out) = corpus(&[
        ("Widget.h", WIDGET_H),
        ("Widget.m", WIDGET_M),
        ("WidgetExtras.swift", WIDGET_EXTRAS_SWIFT),
    ]);
    let widgets = nodes_with_label(&out, "Widget");
    assert_eq!(widgets.len(), 1, "expected one Widget, got {widgets:?}");
    let wid = widgets[0]
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let method_targets: std::collections::HashSet<&str> = out
        .edges
        .iter()
        .filter(|e| cf_str(e, "relation") == "method" && cf_str(e, "source") == wid)
        .map(|e| cf_str(e, "target"))
        .collect();
    let anchored = out
        .nodes
        .iter()
        .filter(|n| {
            n.get("id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|i| method_targets.contains(i))
        })
        .any(|n| cf_str(n, "label").contains("describe"));
    assert!(anchored, "Swift extension method should anchor on Widget");
    assert_no_dangling(&out);
}

/// God-node guard: two unrelated `class Logger` in DIFFERENT directories (each its
/// own .h/.cpp) must NOT merge — TWO distinct Logger nodes.
#[test]
fn decldef_merge_does_not_merge_across_directories() {
    let logger_h = "class Logger {\npublic:\n    void log();\n};\n";
    let logger_cpp = "#include \"Logger.h\"\n\nvoid Logger::log() {}\n";
    let (_tmp, out) = corpus(&[
        ("a/Logger.h", logger_h),
        ("a/Logger.cpp", logger_cpp),
        ("b/Logger.h", logger_h),
        ("b/Logger.cpp", logger_cpp),
    ]);
    let loggers = nodes_with_label(&out, "Logger");
    assert_eq!(
        loggers.len(),
        2,
        "cross-dir Loggers must stay distinct, got {loggers:?}"
    );
    let ids: std::collections::HashSet<&str> = loggers
        .iter()
        .filter_map(|n| n.get("id").and_then(serde_json::Value::as_str))
        .collect();
    assert_eq!(ids.len(), 2, "distinct ids");
}

/// God-node guard: two same-named `class Dup` in the SAME dir but different base
/// stems (Alpha.h, Beta.h) stay distinct (no unique header/impl sibling pair).
#[test]
fn decldef_merge_does_not_merge_same_name_same_dir_distinct_files() {
    let alpha = "class Dup {\npublic:\n    void a();\n};\n";
    let beta = "class Dup {\npublic:\n    void b();\n};\n";
    let (_tmp, out) = corpus(&[("Alpha.h", alpha), ("Beta.h", beta)]);
    let dups = nodes_with_label(&out, "Dup");
    assert_eq!(
        dups.len(),
        2,
        "same-dir distinct Dups must stay distinct, got {dups:?}"
    );
}

// ── 49252d3 (#1547/#1556): C++/ObjC cross-file member-call resolution ─────────

/// Label of the node with `nid`, or `<nid>` when absent (mirrors `_label`).
fn label_of(out: &graphify_extract::ExtractOutput, nid: &str) -> String {
    out.nodes
        .iter()
        .find(|n| cf_str(n, "id") == nid)
        .map_or_else(|| format!("<{nid}>"), |n| cf_str(n, "label").to_string())
}

/// `(source_label, relation, target_label, confidence)` for edges whose relation
/// is in `relations`. Mirrors `_call_edges`.
fn call_edges(
    out: &graphify_extract::ExtractOutput,
    relations: &[&str],
) -> std::collections::HashSet<(String, String, String, String)> {
    out.edges
        .iter()
        .filter(|e| relations.contains(&cf_str(e, "relation")))
        .map(|e| {
            (
                label_of(out, cf_str(e, "source")),
                cf_str(e, "relation").to_string(),
                label_of(out, cf_str(e, "target")),
                cf_str(e, "confidence").to_string(),
            )
        })
        .collect()
}

/// Count of INFERRED cross-file `calls` edges surviving `build_from_json` (the
/// C-family prune must keep an impl->header call).
fn inferred_calls_after_build(out: &graphify_extract::ExtractOutput) -> usize {
    let value = serde_json::to_value(out).expect("serialize extract output");
    let graph = graphify_build::build_from_json(value, true, None).expect("build_from_json");
    graph
        .edges()
        .filter(|e| {
            e.attrs.get("relation").and_then(serde_json::Value::as_str) == Some("calls")
                && e.attrs
                    .get("confidence")
                    .and_then(serde_json::Value::as_str)
                    == Some("INFERRED")
        })
        .count()
}

const CPP_FOO_H: &str = "class Foo {\npublic:\n  void bar();\n};\n";
const CPP_FOO_CPP: &str = "#include \"Foo.h\"\nvoid Foo::bar() {}\n";

/// #1547 headline: a paired class no longer islands — `Main.cpp`'s `f.bar()`
/// connects to `Foo::bar` across files, and `Foo` is ONE merged node.
#[test]
fn cpp_cross_file_member_call_connects() {
    let (_tmp, out) = corpus(&[
        ("src/Foo.h", CPP_FOO_H),
        ("src/Foo.cpp", CPP_FOO_CPP),
        (
            "src/Main.cpp",
            "#include \"Foo.h\"\nint main() { Foo f; f.bar(); return 0; }\n",
        ),
    ]);
    let foos = nodes_with_label(&out, "Foo");
    assert_eq!(foos.len(), 1, "Foo should be one node, got {foos:?}");
    let main_bar: Vec<&CorpusObj> = out
        .edges
        .iter()
        .filter(|e| {
            cf_str(e, "relation") == "calls"
                && label_of(&out, cf_str(e, "source")).contains("main")
                && cf_str(e, "target").ends_with("_bar")
        })
        .collect();
    assert!(
        !main_bar.is_empty(),
        "main's f.bar() should resolve to Foo::bar across files"
    );
    assert!(
        main_bar.iter().all(|e| cf_str(e, "target").contains("foo")),
        "{main_bar:?}"
    );
}

/// `Foo f; f.bar();` resolves to `Foo::bar` — INFERRED (local-declaration type),
/// exactly one edge (no fan-out / duplicate).
#[test]
fn cpp_instance_member_call_resolves() {
    let (_tmp, out) = corpus(&[
        ("src/Foo.h", CPP_FOO_H),
        ("src/Foo.cpp", CPP_FOO_CPP),
        (
            "src/Main.cpp",
            "#include \"Foo.h\"\nint main() { Foo f; f.bar(); }\n",
        ),
    ]);
    let calls = call_edges(&out, &["calls"]);
    assert!(calls.contains(&(
        "main()".into(),
        "calls".into(),
        "bar".into(),
        "INFERRED".into()
    )));
    let bar_calls = calls
        .iter()
        .filter(|c| c.0 == "main()" && c.2 == "bar")
        .count();
    assert_eq!(bar_calls, 1, "exactly one bar call from main");
}

/// #1547 receiver types are per-function-body and source-ordered:
///  - `main`: `Foo f; { Bar f; } f.shared();` — the outer (source-first) `Foo f`
///    owns the post-block call, so it resolves to `Foo::shared` (a reverse walk
///    would pick the nested `Bar f`, which has no `shared`).
///  - `other`: declares its OWN `Bar f` (no `shared`), so `f.shared()` stays
///    UNRESOLVED — `main`'s `Foo f` must not leak across bodies (the old
///    file-scoped table would have typed it `Foo` and fabricated the edge).
#[test]
fn cpp_receiver_types_are_per_body_and_source_ordered() {
    let (_tmp, out) = corpus(&[
        ("src/Foo.h", "class Foo {\npublic:\n  void shared();\n};\n"),
        ("src/Foo.cpp", "#include \"Foo.h\"\nvoid Foo::shared() {}\n"),
        (
            "src/Bar.h",
            "class Bar {\npublic:\n  void unrelated();\n};\n",
        ),
        (
            "src/Bar.cpp",
            "#include \"Bar.h\"\nvoid Bar::unrelated() {}\n",
        ),
        (
            "src/Main.cpp",
            "#include \"Foo.h\"\n#include \"Bar.h\"\nint main() { Foo f; { Bar f; } f.shared(); }\nvoid other() { Bar f; f.shared(); }\n",
        ),
    ]);
    let calls = call_edges(&out, &["calls"]);
    assert!(
        calls.contains(&(
            "main()".into(),
            "calls".into(),
            "shared".into(),
            "INFERRED".into()
        )),
        "outer `Foo f` (source-first) must own main's `f.shared()`; got {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|(s, _, t, _)| s == "other()" && t == "shared"),
        "`other`'s own `Bar f` lacks `shared`; main's `Foo f` must not leak in: {calls:?}"
    );
}

/// `Foo* f = new Foo(); f->bar();` resolves the same way via pointer-arrow access.
#[test]
fn cpp_pointer_member_call_resolves() {
    let (_tmp, out) = corpus(&[
        ("src/Foo.h", CPP_FOO_H),
        ("src/Foo.cpp", CPP_FOO_CPP),
        (
            "src/Main.cpp",
            "#include \"Foo.h\"\nint main() { Foo* f = new Foo(); f->bar(); }\n",
        ),
    ]);
    let calls = call_edges(&out, &["calls"]);
    assert!(calls.contains(&(
        "main()".into(),
        "calls".into(),
        "bar".into(),
        "INFERRED".into()
    )));
}

/// `Foo::bar()` names the type explicitly in source -> EXTRACTED.
#[test]
fn cpp_qualified_member_call_is_extracted() {
    let (_tmp, out) = corpus(&[
        (
            "src/Foo.h",
            "class Foo {\npublic:\n  static void bar();\n};\n",
        ),
        ("src/Foo.cpp", CPP_FOO_CPP),
        (
            "src/Main.cpp",
            "#include \"Foo.h\"\nint main() { Foo::bar(); }\n",
        ),
    ]);
    let calls = call_edges(&out, &["calls"]);
    assert!(calls.contains(&(
        "main()".into(),
        "calls".into(),
        "bar".into(),
        "EXTRACTED".into()
    )));
}

/// `this->bar()` inside `Foo::baz` resolves to the caller's own class -> EXTRACTED.
#[test]
fn cpp_this_member_call_resolves_to_enclosing_class() {
    let (_tmp, out) = corpus(&[
        (
            "src/Foo.h",
            "class Foo {\npublic:\n  void bar();\n  void baz();\n};\n",
        ),
        (
            "src/Foo.cpp",
            "#include \"Foo.h\"\nvoid Foo::bar() {}\nvoid Foo::baz() { this->bar(); }\n",
        ),
    ]);
    let calls = call_edges(&out, &["calls"]);
    assert!(calls.contains(&(
        "baz".into(),
        "calls".into(),
        "bar".into(),
        "EXTRACTED".into()
    )));
}

/// God-node guard: two classes both define `run()`; an uninferable receiver emits
/// zero edges, and `A a; a.run()` resolves to `A::run` ONLY.
#[test]
fn cpp_godnode_guard_ambiguous_and_unknown_receiver() {
    let (_tmp, out) = corpus(&[
        ("src/A.h", "class A {\npublic:\n  void run();\n};\n"),
        ("src/A.cpp", "#include \"A.h\"\nvoid A::run() {}\n"),
        ("src/B.h", "class B {\npublic:\n  void run();\n};\n"),
        ("src/B.cpp", "#include \"B.h\"\nvoid B::run() {}\n"),
        (
            "src/Main.cpp",
            "#include \"A.h\"\n#include \"B.h\"\nint main() { x.run(); A a; a.run(); }\n",
        ),
    ]);
    let run_calls: Vec<&CorpusObj> = out
        .edges
        .iter()
        .filter(|e| {
            cf_str(e, "relation") == "calls"
                && label_of(&out, cf_str(e, "source")) == "main()"
                && label_of(&out, cf_str(e, "target")) == "run"
        })
        .collect();
    assert_eq!(
        run_calls.len(),
        1,
        "exactly one resolved run() call: {run_calls:?}"
    );
    let tgt = cf_str(run_calls[0], "target");
    let tgt_sf = out
        .nodes
        .iter()
        .find(|n| cf_str(n, "id") == tgt)
        .map_or("", |n| cf_str(n, "source_file"));
    assert!(
        tgt_sf.ends_with("A.h"),
        "run() must bind A::run, got {tgt_sf}"
    );
}

/// The receiver-typed INFERRED call to a header-declared method survives
/// `build_from_json` (C-family prune keeps the impl->header edge).
#[test]
fn cpp_resolved_call_survives_build() {
    let (_tmp, out) = corpus(&[
        ("src/Foo.h", CPP_FOO_H),
        ("src/Foo.cpp", CPP_FOO_CPP),
        (
            "src/Main.cpp",
            "#include \"Foo.h\"\nint main() { Foo f; f.bar(); }\n",
        ),
    ]);
    assert!(inferred_calls_after_build(&out) >= 1);
}

/// A lowercase receiver absent from the file's local type table is never guessed.
#[test]
fn cpp_unknown_receiver_emits_no_edge() {
    let (_tmp, out) = corpus(&[
        (
            "src/Helper.h",
            "class Helper {\npublic:\n  void help();\n};\n",
        ),
        (
            "src/Helper.cpp",
            "#include \"Helper.h\"\nvoid Helper::help() {}\n",
        ),
        (
            "src/Main.cpp",
            "#include \"Helper.h\"\nint main() { mystery.help(); }\n",
        ),
    ]);
    let calls = call_edges(&out, &["calls"]);
    assert!(!calls.iter().any(|c| c.0 == "main()" && c.2 == "help"));
}

const OBJC_FOO_H: &str = "@interface Foo : NSObject\n- (void)doThing;\n@end\n";
const OBJC_FOO_M: &str = "#import \"Foo.h\"\n@implementation Foo\n- (void)doThing {}\n@end\n";
const OBJC_BAR_M: &str = "#import \"Foo.h\"\n@implementation Bar\n- (void)go {\n  Foo *f = [[Foo alloc] init];\n  [f doThing];\n}\n@end\n";

/// `Foo *f = ...; [f doThing];` in Bar.m -> cross-file `calls` to Foo's `-doThing`
/// (INFERRED, receiver typed from the `Foo *f` local).
#[test]
fn objc_instance_message_send_resolves() {
    let (_tmp, out) = corpus(&[
        ("src/Foo.h", OBJC_FOO_H),
        ("src/Foo.m", OBJC_FOO_M),
        ("src/Bar.m", OBJC_BAR_M),
    ]);
    let calls = call_edges(&out, &["calls"]);
    assert!(calls.contains(&(
        "-go".into(),
        "calls".into(),
        "-doThing".into(),
        "INFERRED".into()
    )));
}

/// #1547/#1556 (Objective-C) receiver typing is per-method and source-ordered:
///  - `inner`: an outer `Foo *f` then a nested `{ Bar *f }` — the outer (source
///    -first) binding owns `[f doThing]`, so it resolves to `Foo::doThing`.
///  - `isolated`: declares its OWN `Bar *f` (Bar has no `doThing`), so `[f doThing]`
///    stays UNRESOLVED — `inner`'s `Foo *f` must not leak across methods (the old
///    file-wide table would have typed it `Foo` and fabricated the edge).
#[test]
fn objc_receiver_types_are_per_method_and_source_ordered() {
    let (_tmp, out) = corpus(&[
        ("src/Foo.h", OBJC_FOO_H),
        ("src/Foo.m", OBJC_FOO_M),
        (
            "src/Bar.h",
            "@interface Bar : NSObject\n- (void)other;\n@end\n",
        ),
        (
            "src/Bar.m",
            "#import \"Bar.h\"\n@implementation Bar\n- (void)other {}\n@end\n",
        ),
        (
            "src/Caller.m",
            "#import \"Foo.h\"\n#import \"Bar.h\"\n@implementation Caller\n- (void)inner {\n  Foo *f = [[Foo alloc] init];\n  { Bar *f = [[Bar alloc] init]; }\n  [f doThing];\n}\n- (void)isolated {\n  Bar *f = [[Bar alloc] init];\n  [f doThing];\n}\n@end\n",
        ),
    ]);
    let calls = call_edges(&out, &["calls"]);
    assert!(
        calls
            .iter()
            .any(|(s, _, t, _)| s == "-inner" && t == "-doThing"),
        "nested `Bar f` must not clobber the outer source-first `Foo f`: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|(s, _, t, _)| s == "-isolated" && t == "-doThing"),
        "`isolated`'s own `Bar f` lacks `doThing`; `inner`'s `Foo f` must not leak in: {calls:?}"
    );
}

/// `[self render]` inside Foo resolves to Foo's `-render` -> EXTRACTED.
#[test]
fn objc_self_message_send_resolves_to_enclosing_class() {
    let (_tmp, out) = corpus(&[
        (
            "src/Foo.h",
            "@interface Foo : NSObject\n- (void)render;\n- (void)setup;\n@end\n",
        ),
        (
            "src/Foo.m",
            "#import \"Foo.h\"\n@implementation Foo\n- (void)setup { [self render]; }\n- (void)render {}\n@end\n",
        ),
    ]);
    let calls = call_edges(&out, &["calls"]);
    assert!(calls.contains(&(
        "-setup".into(),
        "calls".into(),
        "-render".into(),
        "EXTRACTED".into()
    )));
}

/// God-node guard: two classes both define `-doStuff`; an uninferable receiver
/// emits ZERO edges across the corpus.
#[test]
fn objc_godnode_guard_ambiguous_selector() {
    let (_tmp, out) = corpus(&[
        (
            "src/A.h",
            "@interface A : NSObject\n- (void)doStuff;\n@end\n",
        ),
        (
            "src/A.m",
            "#import \"A.h\"\n@implementation A\n- (void)doStuff {}\n@end\n",
        ),
        (
            "src/B.h",
            "@interface B : NSObject\n- (void)doStuff;\n@end\n",
        ),
        (
            "src/B.m",
            "#import \"B.h\"\n@implementation B\n- (void)doStuff {}\n@end\n",
        ),
        (
            "src/C.m",
            "#import \"A.h\"\n#import \"B.h\"\n@implementation C\n- (void)go { [thing doStuff]; }\n@end\n",
        ),
    ]);
    let go_calls = out
        .edges
        .iter()
        .filter(|e| {
            cf_str(e, "relation") == "calls" && label_of(&out, cf_str(e, "source")) == "-go"
        })
        .count();
    assert_eq!(go_calls, 0, "ambiguous selector must not fan out");
}

/// The cross-file Objective-C call lands on a real definition, so `build_from_json`
/// keeps it (INFERRED, not pruned).
#[test]
fn objc_resolved_calls_survive_build() {
    let (_tmp, out) = corpus(&[
        ("src/Foo.h", OBJC_FOO_H),
        ("src/Foo.m", OBJC_FOO_M),
        ("src/Bar.m", OBJC_BAR_M),
    ]);
    assert!(inferred_calls_after_build(&out) >= 1);
}
