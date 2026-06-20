//! JS/TS workspace import resolution via `package.json` `workspaces` (#9a7dbfb).
//!
//! Ports the 3 new `test_js_import_resolution.py` cases (npm list, yarn-berry
//! object, and cache-refresh-between-extract-calls) plus the pnpm-precedence
//! guard. The pnpm-only resolution and `.`-package cases already live in
//! `parity.rs`.

use std::error::Error;
use std::path::Path;

use graphify_extract::{extract, file_node_id};
use serde_json::Value;
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn Error>>;

fn write(path: &Path, text: &str) -> TestResult {
    std::fs::create_dir_all(path.parent().ok_or("path has no parent")?)?;
    std::fs::write(path, text)?;
    Ok(())
}

/// `true` if an `imports_from` edge connects the file nodes for `source_rel` and
/// `target_rel` (paths relative to the corpus root), mirroring the Python
/// `_has_edge` helper which keys both endpoints by `_file_node_id`.
fn has_imports_from(
    out: &graphify_extract::ExtractOutput,
    source_rel: &str,
    target_rel: &str,
) -> bool {
    let s = file_node_id(Path::new(source_rel));
    let t = file_node_id(Path::new(target_rel));
    out.edges.iter().any(|e| {
        e.get("source").and_then(Value::as_str) == Some(s.as_str())
            && e.get("target").and_then(Value::as_str) == Some(t.as_str())
            && e.get("relation").and_then(Value::as_str) == Some("imports_from")
    })
}

/// Write the `@workspace/types` package + an importer under `root`.
fn write_package_and_importer(root: &Path) -> TestResult {
    write(
        &root.join("packages/types/package.json"),
        r#"{"name": "@workspace/types", "exports": "./src/index.ts"}"#,
    )?;
    write(
        &root.join("packages/types/src/index.ts"),
        "export interface SomeDto { id: string }\n",
    )?;
    write(
        &root.join("apps/web/src/page.ts"),
        "import type { SomeDto } from '@workspace/types'\nconst dto: SomeDto = { id: '1' }\n",
    )
}

#[test]
fn npm_workspace_package_import_resolves_package_entry() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("package.json"),
        r#"{"workspaces": ["apps/*", "packages/*"]}"#,
    )?;
    write_package_and_importer(root)?;

    let out = extract(
        &[
            root.join("packages/types/src/index.ts"),
            root.join("apps/web/src/page.ts"),
        ],
        Some(root),
    );
    assert!(
        has_imports_from(&out, "apps/web/src/page.ts", "packages/types/src/index.ts"),
        "edges: {:?}",
        out.edges
    );
    Ok(())
}

#[test]
fn yarn_workspace_package_import_resolves_package_entry() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    // yarn berry object form: { "packages": [...] }.
    write(
        &root.join("package.json"),
        r#"{"workspaces": {"packages": ["apps/*", "packages/*"]}}"#,
    )?;
    write_package_and_importer(root)?;

    let out = extract(
        &[
            root.join("packages/types/src/index.ts"),
            root.join("apps/web/src/page.ts"),
        ],
        Some(root),
    );
    assert!(
        has_imports_from(&out, "apps/web/src/page.ts", "packages/types/src/index.ts"),
        "edges: {:?}",
        out.edges
    );
    Ok(())
}

#[test]
fn pnpm_workspace_takes_precedence_over_package_json_workspaces() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("pnpm-workspace.yaml"),
        "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
    )?;
    // A misleading package.json `workspaces` pointing elsewhere must be ignored
    // because pnpm-workspace.yaml wins.
    write(&root.join("package.json"), r#"{"workspaces": ["other/*"]}"#)?;
    write_package_and_importer(root)?;

    let out = extract(
        &[
            root.join("packages/types/src/index.ts"),
            root.join("apps/web/src/page.ts"),
        ],
        Some(root),
    );
    assert!(
        has_imports_from(&out, "apps/web/src/page.ts", "packages/types/src/index.ts"),
        "edges: {:?}",
        out.edges
    );
    Ok(())
}

#[test]
fn workspace_package_cache_refreshes_between_extract_calls() -> TestResult {
    // The workspace-package cache must not serve a stale (empty) result from a
    // prior extract() once the package appears. extract() clears the cache at the
    // start of every run, so the second call re-scans and resolves the import.
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("pnpm-workspace.yaml"),
        "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
    )?;
    write(
        &root.join("apps/web/src/page.ts"),
        "import type { SomeDto } from '@workspace/types'\nconst dto: SomeDto = { id: '1' }\n",
    )?;

    let first = extract(&[root.join("apps/web/src/page.ts")], Some(root));
    assert!(
        !has_imports_from(
            &first,
            "apps/web/src/page.ts",
            "packages/types/src/index.ts"
        ),
        "package does not exist yet; edges: {:?}",
        first.edges
    );

    write(
        &root.join("packages/types/package.json"),
        r#"{"name": "@workspace/types", "exports": "./src/index.ts"}"#,
    )?;
    write(
        &root.join("packages/types/src/index.ts"),
        "export interface SomeDto { id: string }\n",
    )?;

    let second = extract(
        &[
            root.join("packages/types/src/index.ts"),
            root.join("apps/web/src/page.ts"),
        ],
        Some(root),
    );
    assert!(
        has_imports_from(
            &second,
            "apps/web/src/page.ts",
            "packages/types/src/index.ts"
        ),
        "cache must refresh once the package appears; edges: {:?}",
        second.edges
    );
    Ok(())
}

#[test]
fn malformed_inner_workspaces_does_not_shadow_real_root() -> TestResult {
    // A nested package.json carrying a present-but-malformed `workspaces` value
    // (here an object with no `packages` array, which resolves to zero globs)
    // must NOT terminate the workspace-root walk. Before the shape check in
    // `find_workspace_root`, this inner manifest shadowed the real root one
    // level up and the package import silently failed to resolve.
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("package.json"),
        r#"{"workspaces": ["apps/*", "packages/*"]}"#,
    )?;
    // Intermediate package between the importer and the real root.
    write(
        &root.join("apps/web/package.json"),
        r#"{"name": "web", "workspaces": {}}"#,
    )?;
    write_package_and_importer(root)?;

    let out = extract(
        &[
            root.join("packages/types/src/index.ts"),
            root.join("apps/web/src/page.ts"),
        ],
        Some(root),
    );
    assert!(
        has_imports_from(&out, "apps/web/src/page.ts", "packages/types/src/index.ts"),
        "edges: {:?}",
        out.edges
    );
    Ok(())
}
