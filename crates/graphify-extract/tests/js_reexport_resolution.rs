//! Cross-file JS/TS barrel re-export resolution.
//!
//! Ports the barrel-chain cases from `graphify-py/tests/test_js_import_resolution.py`:
//! named/aliased/star/type re-exports, local-alias re-exports, and
//! call-through-barrel — each must resolve the consumer's import (and calls) to
//! the origin symbol, not the barrel.
#![allow(clippy::expect_used)]

use std::error::Error;
use std::path::Path;

use graphify_extract::{ExtractOutput, extract, file_node_id, file_stem, make_id};
use serde_json::Value;
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn Error>>;

fn write(path: &Path, text: &str) -> TestResult {
    std::fs::create_dir_all(path.parent().ok_or("path has no parent")?)?;
    std::fs::write(path, text)?;
    Ok(())
}

fn edge_exists(out: &ExtractOutput, source: &str, target: &str, relation: &str) -> bool {
    out.edges.iter().any(|e| {
        e.get("source").and_then(Value::as_str) == Some(source)
            && e.get("target").and_then(Value::as_str) == Some(target)
            && e.get("relation").and_then(Value::as_str) == Some(relation)
    })
}

/// File→file edge keyed by `file_node_id`, mirroring Python `_has_edge`.
fn has_file_edge(out: &ExtractOutput, source_rel: &str, target_rel: &str, relation: &str) -> bool {
    edge_exists(
        out,
        &file_node_id(Path::new(source_rel)),
        &file_node_id(Path::new(target_rel)),
        relation,
    )
}

/// Consumer-file → origin-symbol `imports` edge, mirroring Python `_has_symbol_edge`.
fn has_symbol_edge(out: &ExtractOutput, source_rel: &str, target_file: &str, symbol: &str) -> bool {
    let target = make_id(&[&file_stem(Path::new(target_file)), symbol]);
    edge_exists(
        out,
        &file_node_id(Path::new(source_rel)),
        &target,
        "imports",
    )
}

/// Symbol → symbol edge, mirroring Python `_has_symbol_to_symbol_edge`.
fn has_symbol_to_symbol_edge(
    out: &ExtractOutput,
    source_file: &str,
    source_symbol: &str,
    target_file: &str,
    target_symbol: &str,
    relation: &str,
) -> bool {
    let source = make_id(&[&file_stem(Path::new(source_file)), source_symbol]);
    let target = make_id(&[&file_stem(Path::new(target_file)), target_symbol]);
    edge_exists(out, &source, &target, relation)
}

#[test]
fn ts_named_reexport_alias_from_index_resolves_imported_symbol_to_origin() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("src/lib/foo.ts"),
        "export class InternalFoo { id = '' }\n",
    )?;
    write(
        &root.join("src/lib/index.ts"),
        "export { InternalFoo as Foo } from './foo'\n",
    )?;
    write(
        &root.join("src/routes/page.ts"),
        "import type { Foo } from '../lib/index'\nexport type X = Foo\n",
    )?;
    let out = extract(
        &[
            root.join("src/lib/foo.ts"),
            root.join("src/lib/index.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(root),
    );
    assert!(has_file_edge(
        &out,
        "src/lib/index.ts",
        "src/lib/foo.ts",
        "re_exports"
    ));
    assert!(has_symbol_edge(
        &out,
        "src/routes/page.ts",
        "src/lib/foo.ts",
        "InternalFoo"
    ));
    Ok(())
}

#[test]
fn ts_export_star_from_index_resolves_imported_symbol_to_origin() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("src/lib/foo.ts"),
        "export class Foo { id = '' }\n",
    )?;
    write(&root.join("src/lib/index.ts"), "export * from './foo'\n")?;
    write(
        &root.join("src/routes/page.ts"),
        "import type { Foo } from '../lib/index'\nexport type X = Foo\n",
    )?;
    let out = extract(
        &[
            root.join("src/lib/foo.ts"),
            root.join("src/lib/index.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(root),
    );
    assert!(has_file_edge(
        &out,
        "src/lib/index.ts",
        "src/lib/foo.ts",
        "re_exports"
    ));
    assert!(has_symbol_edge(
        &out,
        "src/routes/page.ts",
        "src/lib/foo.ts",
        "Foo"
    ));
    Ok(())
}

#[test]
fn ts_import_alias_then_reexport_alias_resolves_imported_symbol_to_origin() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("src/lib/foo.ts"),
        "export class Foo { id = '' }\n",
    )?;
    write(
        &root.join("src/lib/index.ts"),
        "import type { Foo as LocalFoo } from './foo'\nexport type { LocalFoo as PublicFoo }\n",
    )?;
    write(
        &root.join("src/routes/page.ts"),
        "import type { PublicFoo } from '../lib/index'\nexport type X = PublicFoo\n",
    )?;
    let out = extract(
        &[
            root.join("src/lib/foo.ts"),
            root.join("src/lib/index.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(root),
    );
    assert!(has_file_edge(
        &out,
        "src/lib/index.ts",
        "src/lib/foo.ts",
        "re_exports"
    ));
    assert!(has_symbol_edge(
        &out,
        "src/routes/page.ts",
        "src/lib/foo.ts",
        "Foo"
    ));
    Ok(())
}

#[test]
fn ts_import_from_index_then_exported_type_alias_resolves_to_origin_symbol() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("src/lib/foo.ts"),
        "export class Foo { id = '' }\n",
    )?;
    write(
        &root.join("src/lib/index.ts"),
        "export { Foo } from './foo'\n",
    )?;
    write(
        &root.join("src/routes/page.ts"),
        "import type { Foo } from '../lib/index'\nexport type X = Foo\n",
    )?;
    let out = extract(
        &[
            root.join("src/lib/foo.ts"),
            root.join("src/lib/index.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(root),
    );
    assert!(has_file_edge(
        &out,
        "src/lib/index.ts",
        "src/lib/foo.ts",
        "re_exports"
    ));
    assert!(has_symbol_edge(
        &out,
        "src/routes/page.ts",
        "src/lib/foo.ts",
        "Foo"
    ));
    Ok(())
}

#[test]
fn ts_reexported_interface_resolves_imported_symbol_to_origin() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("src/lib/foo.ts"),
        "export interface Foo { id: string }\n",
    )?;
    write(
        &root.join("src/lib/index.ts"),
        "export type { Foo } from './foo'\n",
    )?;
    write(
        &root.join("src/routes/page.ts"),
        "import type { Foo } from '../lib/index'\nexport type X = Foo\n",
    )?;
    let out = extract(
        &[
            root.join("src/lib/foo.ts"),
            root.join("src/lib/index.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(root),
    );
    assert!(has_file_edge(
        &out,
        "src/lib/index.ts",
        "src/lib/foo.ts",
        "re_exports"
    ));
    assert!(has_symbol_edge(
        &out,
        "src/routes/page.ts",
        "src/lib/foo.ts",
        "Foo"
    ));
    Ok(())
}

#[test]
fn ts_reexported_abstract_class_resolves_imported_symbol_to_origin() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("src/lib/foo.ts"),
        "export abstract class Foo { abstract run(): void }\n",
    )?;
    write(
        &root.join("src/lib/index.ts"),
        "export { Foo } from './foo'\n",
    )?;
    write(
        &root.join("src/routes/page.ts"),
        "import { Foo } from '../lib/index'\nclass Impl extends Foo { run() {} }\n",
    )?;
    let out = extract(
        &[
            root.join("src/lib/foo.ts"),
            root.join("src/lib/index.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(root),
    );
    assert!(has_file_edge(
        &out,
        "src/lib/index.ts",
        "src/lib/foo.ts",
        "re_exports"
    ));
    assert!(has_symbol_edge(
        &out,
        "src/routes/page.ts",
        "src/lib/foo.ts",
        "Foo"
    ));
    Ok(())
}

#[test]
fn ts_const_alias_reexport_resolves_imported_symbol_to_origin() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("src/lib/foo.ts"),
        "export class Foo { id = '' }\n",
    )?;
    write(
        &root.join("src/lib/index.ts"),
        "import { Foo } from './foo'\nexport const PublicFoo = Foo\n",
    )?;
    write(
        &root.join("src/routes/page.ts"),
        "import { PublicFoo } from '../lib/index'\nnew PublicFoo()\n",
    )?;
    let out = extract(
        &[
            root.join("src/lib/foo.ts"),
            root.join("src/lib/index.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(root),
    );
    assert!(has_file_edge(
        &out,
        "src/lib/index.ts",
        "src/lib/foo.ts",
        "re_exports"
    ));
    assert!(has_symbol_edge(
        &out,
        "src/routes/page.ts",
        "src/lib/foo.ts",
        "Foo"
    ));
    Ok(())
}

#[test]
fn ts_local_const_alias_then_named_reexport_resolves_imported_symbol_to_origin() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("src/lib/foo.ts"),
        "export function makeFoo() { return {} }\n",
    )?;
    write(
        &root.join("src/lib/index.ts"),
        "import { makeFoo } from './foo'\nconst PublicFactory = makeFoo\nexport { PublicFactory }\n",
    )?;
    write(
        &root.join("src/routes/page.ts"),
        "import { PublicFactory } from '../lib/index'\nPublicFactory()\n",
    )?;
    let out = extract(
        &[
            root.join("src/lib/foo.ts"),
            root.join("src/lib/index.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(root),
    );
    assert!(has_file_edge(
        &out,
        "src/lib/index.ts",
        "src/lib/foo.ts",
        "re_exports"
    ));
    assert!(has_symbol_edge(
        &out,
        "src/routes/page.ts",
        "src/lib/foo.ts",
        "makeFoo"
    ));
    Ok(())
}

#[test]
fn ts_arrow_function_call_through_barrel_targets_origin_symbol() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("src/lib/foo.ts"),
        "export function Foo() { return 1 }\n",
    )?;
    write(
        &root.join("src/other/foo.ts"),
        "export function Foo() { return 2 }\n",
    )?;
    write(
        &root.join("src/lib/index.ts"),
        "export { Foo } from './foo'\n",
    )?;
    write(
        &root.join("src/routes/page.ts"),
        "import { Foo } from '../lib/index'\nconst X = () => Foo()\n",
    )?;
    let out = extract(
        &[
            root.join("src/lib/foo.ts"),
            root.join("src/other/foo.ts"),
            root.join("src/lib/index.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(root),
    );
    assert!(has_symbol_to_symbol_edge(
        &out,
        "src/routes/page.ts",
        "X",
        "src/lib/foo.ts",
        "Foo",
        "calls",
    ));
    Ok(())
}

#[test]
fn ts_import_alias_does_not_affect_same_named_local_symbol_when_unused() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("src/lib/foo.ts"),
        "export function Foo() { return 1 }\n",
    )?;
    write(
        &root.join("src/lib/index.ts"),
        "export { Foo } from './foo'\n",
    )?;
    write(
        &root.join("src/routes/page.ts"),
        "import { Foo as Bar } from '../lib/index'\nconst Foo = () => {}\n",
    )?;
    let out = extract(
        &[
            root.join("src/lib/foo.ts"),
            root.join("src/lib/index.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(root),
    );
    assert!(!has_symbol_to_symbol_edge(
        &out,
        "src/routes/page.ts",
        "Foo",
        "src/lib/foo.ts",
        "Foo",
        "calls",
    ));
    Ok(())
}

#[test]
fn ts_import_alias_call_from_same_named_local_symbol_targets_origin() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("src/lib/foo.ts"),
        "export function Foo() { return 1 }\n",
    )?;
    write(
        &root.join("src/lib/index.ts"),
        "export { Foo } from './foo'\n",
    )?;
    write(
        &root.join("src/routes/page.ts"),
        "import { Foo as Bar } from '../lib/index'\nconst Foo = () => Bar()\n",
    )?;
    let out = extract(
        &[
            root.join("src/lib/foo.ts"),
            root.join("src/lib/index.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(root),
    );
    assert!(has_symbol_to_symbol_edge(
        &out,
        "src/routes/page.ts",
        "Foo",
        "src/lib/foo.ts",
        "Foo",
        "calls",
    ));
    Ok(())
}

#[test]
fn ts_reexported_type_alias_resolves_imported_symbol_to_origin() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("src/lib/foo.ts"),
        "export type Foo = { id: string }\n",
    )?;
    write(
        &root.join("src/lib/index.ts"),
        "export type { Foo } from './foo'\n",
    )?;
    write(
        &root.join("src/routes/page.ts"),
        "import type { Foo } from '../lib/index'\nexport type X = Foo\n",
    )?;
    let out = extract(
        &[
            root.join("src/lib/foo.ts"),
            root.join("src/lib/index.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(root),
    );
    assert!(has_file_edge(
        &out,
        "src/lib/index.ts",
        "src/lib/foo.ts",
        "re_exports"
    ));
    assert!(has_symbol_edge(
        &out,
        "src/routes/page.ts",
        "src/lib/foo.ts",
        "Foo"
    ));
    Ok(())
}
