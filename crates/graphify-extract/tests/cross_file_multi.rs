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
