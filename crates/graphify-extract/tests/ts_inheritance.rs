//! Parity tests for issue #1095: TypeScript inheritance capture.
//!
//! Mirrors `graphify-py/tests/test_ts_inheritance.py`. Two gaps on v0.8.26:
//!   1. `interface A extends B` produced no `inherits` edge (the walker only
//!      looked at `class_heritage`, but interface heritage is an
//!      `extends_type_clause` node).
//!   2. `class X extends Y` where `Y` is same-file produced no edge (the
//!      use-fact resolver only consulted the import table, never same-file
//!      symbol nodes).
//!
//! Files live under a `src/` subdir so the one-parent-level node-ID stem is
//! stable (a root-level file would derive its stem from the tmp dir name).
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};

use graphify_extract::{ExtractOutput, extract, file_stem, make_id};
use serde_json::Value;
use tempfile::tempdir;

fn write(path: &Path, text: &str) -> PathBuf {
    std::fs::create_dir_all(path.parent().expect("has parent")).expect("create dirs");
    std::fs::write(path, text).expect("write file");
    path.to_path_buf()
}

/// Write each `(relative path, contents)` pair under a fresh tempdir, then run
/// `extract` over all of them in the given order. The returned `TempDir` must
/// be kept in scope by the caller so the written files survive for the duration
/// of the test.
fn extract_files(files: &[(&str, &str)]) -> (tempfile::TempDir, ExtractOutput) {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canonicalize");
    let paths: Vec<PathBuf> = files
        .iter()
        .map(|(rel, contents)| write(&root.join(rel), contents))
        .collect();
    let result = extract(&paths, Some(&root));
    (tmp, result)
}

/// `_make_id(_file_stem(src_file), src_sym)` → symbol node id.
fn sym_id(file: &str, sym: &str) -> String {
    make_id(&[&file_stem(Path::new(file)), sym])
}

fn has_edge(
    edges: &[indexmap::IndexMap<String, Value>],
    src_file: &str,
    src_sym: &str,
    tgt_file: &str,
    tgt_sym: &str,
    relation: &str,
) -> bool {
    let src = sym_id(src_file, src_sym);
    let tgt = sym_id(tgt_file, tgt_sym);
    edges.iter().any(|e| {
        e.get("source").and_then(Value::as_str) == Some(src.as_str())
            && e.get("target").and_then(Value::as_str) == Some(tgt.as_str())
            && e.get("relation").and_then(Value::as_str) == Some(relation)
    })
}

#[test]
fn interface_extends_same_file() {
    let (_tmp, result) = extract_files(&[(
        "src/a.ts",
        "export interface Base { x: number; }\n\
         export interface Derived extends Base { y: number; }\n",
    )]);
    assert!(
        has_edge(
            &result.edges,
            "src/a.ts",
            "Derived",
            "src/a.ts",
            "Base",
            "inherits"
        ),
        "expected same-file `inherits` edge Derived -> Base"
    );
}

#[test]
fn interface_extends_multiple_same_file() {
    let (_tmp, result) = extract_files(&[(
        "src/a.ts",
        "interface A { a: number; }\n\
         interface B { b: number; }\n\
         interface M extends A, B { m: number; }\n",
    )]);
    assert!(
        has_edge(&result.edges, "src/a.ts", "M", "src/a.ts", "A", "inherits"),
        "expected same-file `inherits` edge M -> A"
    );
    assert!(
        has_edge(&result.edges, "src/a.ts", "M", "src/a.ts", "B", "inherits"),
        "expected same-file `inherits` edge M -> B"
    );
}

#[test]
fn class_extends_same_file() {
    let (_tmp, result) =
        extract_files(&[("src/a.ts", "class Animal {}\nclass Dog extends Animal {}\n")]);
    assert!(
        has_edge(
            &result.edges,
            "src/a.ts",
            "Dog",
            "src/a.ts",
            "Animal",
            "inherits"
        ),
        "expected same-file `inherits` edge Dog -> Animal"
    );
}

#[test]
fn interface_extends_generic_base_same_file() {
    let (_tmp, result) = extract_files(&[(
        "src/a.ts",
        "interface Base<T> { x: T; }\n\
         interface G extends Base<number> { y: number; }\n",
    )]);
    assert!(
        has_edge(
            &result.edges,
            "src/a.ts",
            "G",
            "src/a.ts",
            "Base",
            "inherits"
        ),
        "expected same-file `inherits` edge G -> Base (generic base)"
    );
}

#[test]
fn interface_extends_imported() {
    let (_tmp, result) = extract_files(&[
        (
            "src/a.ts",
            "import { Imported } from './b';\n\
             export interface D extends Imported { d: number; }\n",
        ),
        ("src/b.ts", "export interface Imported { z: number; }\n"),
    ]);
    assert!(
        has_edge(
            &result.edges,
            "src/a.ts",
            "D",
            "src/b.ts",
            "Imported",
            "inherits"
        ),
        "expected cross-file `inherits` edge D -> Imported"
    );
}

#[test]
fn imported_class_extends_still_works() {
    // Regression guard: the originally-working imported-class case must stay.
    let (_tmp, result) = extract_files(&[
        (
            "src/a.ts",
            "import { Imported } from './b';\nclass Cat extends Imported {}\n",
        ),
        ("src/b.ts", "export class Imported {}\n"),
    ]);
    assert!(
        has_edge(
            &result.edges,
            "src/a.ts",
            "Cat",
            "src/b.ts",
            "Imported",
            "inherits"
        ),
        "expected cross-file `inherits` edge Cat -> Imported"
    );
}

#[test]
fn class_implements_same_file_interface() {
    let (_tmp, result) = extract_files(&[(
        "src/a.ts",
        "interface Walker { walk(): void; }\n\
         class Person implements Walker { walk() {} }\n",
    )]);
    assert!(
        has_edge(
            &result.edges,
            "src/a.ts",
            "Person",
            "src/a.ts",
            "Walker",
            "implements"
        ),
        "expected same-file `implements` edge Person -> Walker"
    );
}
