//! Cross-file extraction tests — exercise the multi-file import resolution
//! paths in `extractors/multi.rs`.

#![allow(clippy::expect_used)]

use std::fs;

use graphify_extract::extract;

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
