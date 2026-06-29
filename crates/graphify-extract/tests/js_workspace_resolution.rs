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

// ── #1531: tsconfig path-alias fallback targets ──────────────────────────────

#[test]
fn tsconfig_alias_resolves_second_target_when_first_missing() -> TestResult {
    // tsc tries each `paths` target in declared order until one resolves on
    // disk. The file lives only at the SECOND target, so keeping only the first
    // entry (#1531) dropped the edge.
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("tsconfig.json"),
        r#"{"compilerOptions": {"baseUrl": ".", "paths": {"$lib/*": ["generated/*", "src/lib/*"]}}}"#,
    )?;
    write(&root.join("src/lib/utils.ts"), "export const helper = 1\n")?;
    write(
        &root.join("src/routes/page.ts"),
        "import { helper } from '$lib/utils'\nconsole.log(helper)\n",
    )?;

    let out = extract(
        &[
            root.join("src/lib/utils.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(root),
    );
    assert!(
        has_imports_from(&out, "src/routes/page.ts", "src/lib/utils.ts"),
        "edges: {:?}",
        out.edges
    );
    Ok(())
}

#[test]
fn tsconfig_alias_first_target_wins_when_both_exist() -> TestResult {
    // When the file exists at BOTH targets, tsc resolves to the FIRST. The edge
    // must target the generated/ copy, not src/lib.
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("tsconfig.json"),
        r#"{"compilerOptions": {"baseUrl": ".", "paths": {"$lib/*": ["generated/*", "src/lib/*"]}}}"#,
    )?;
    write(
        &root.join("generated/utils.ts"),
        "export const helper = 1\n",
    )?;
    write(&root.join("src/lib/utils.ts"), "export const helper = 2\n")?;
    write(
        &root.join("src/routes/page.ts"),
        "import { helper } from '$lib/utils'\nconsole.log(helper)\n",
    )?;

    let out = extract(
        &[
            root.join("generated/utils.ts"),
            root.join("src/lib/utils.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(root),
    );
    assert!(
        has_imports_from(&out, "src/routes/page.ts", "generated/utils.ts"),
        "edges: {:?}",
        out.edges
    );
    assert!(
        !has_imports_from(&out, "src/routes/page.ts", "src/lib/utils.ts"),
        "first target must win; edges: {:?}",
        out.edges
    );
    Ok(())
}

#[test]
fn tsconfig_alias_none_exist_creates_no_false_edge() -> TestResult {
    // The file exists at neither target; no concrete imports_from edge to either
    // candidate may be fabricated (it stays an external/phantom target).
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("tsconfig.json"),
        r#"{"compilerOptions": {"baseUrl": ".", "paths": {"$lib/*": ["generated/*", "src/lib/*"]}}}"#,
    )?;
    write(&root.join("src/routes/other.ts"), "export const x = 1\n")?;
    write(
        &root.join("src/routes/page.ts"),
        "import { helper } from '$lib/utils'\nconsole.log(helper)\n",
    )?;

    let out = extract(
        &[
            root.join("src/routes/other.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(root),
    );
    assert!(
        !has_imports_from(&out, "src/routes/page.ts", "generated/utils.ts"),
        "edges: {:?}",
        out.edges
    );
    assert!(
        !has_imports_from(&out, "src/routes/page.ts", "src/lib/utils.ts"),
        "edges: {:?}",
        out.edges
    );
    Ok(())
}

// ── #1308: workspace subpath `exports` map resolution ────────────────────────

#[test]
fn workspace_subpath_export_string_resolves() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("pnpm-workspace.yaml"),
        "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
    )?;
    write(
        &root.join("packages/pkg-a/package.json"),
        r#"{"name": "@example/pkg-a", "exports": {".": "./src/index.ts", "./browser": "./src/browser.ts"}}"#,
    )?;
    write(
        &root.join("packages/pkg-a/src/browser.ts"),
        "export const value = \"ok\"\n",
    )?;
    write(
        &root.join("apps/web/src/consumer.ts"),
        "import { value } from '@example/pkg-a/browser'\nexport const v = value\n",
    )?;

    let out = extract(
        &[
            root.join("packages/pkg-a/src/browser.ts"),
            root.join("apps/web/src/consumer.ts"),
        ],
        Some(root),
    );
    assert!(
        has_imports_from(
            &out,
            "apps/web/src/consumer.ts",
            "packages/pkg-a/src/browser.ts"
        ),
        "edges: {:?}",
        out.edges
    );
    Ok(())
}

#[test]
fn workspace_subpath_export_condition_object_resolves() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("pnpm-workspace.yaml"),
        "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
    )?;
    write(
        &root.join("packages/pkg-a/package.json"),
        r#"{"name": "@example/pkg-a", "exports": {"./browser": {"source": "./src/browser.ts", "import": "./dist/esm/browser.js", "require": "./dist/cjs/browser.js", "types": "./dist/types/browser.d.ts"}}}"#,
    )?;
    write(
        &root.join("packages/pkg-a/src/browser.ts"),
        "export const value = \"ok\"\n",
    )?;
    write(
        &root.join("apps/web/src/consumer.ts"),
        "import { value } from '@example/pkg-a/browser'\nexport const v = value\n",
    )?;

    let out = extract(
        &[
            root.join("packages/pkg-a/src/browser.ts"),
            root.join("apps/web/src/consumer.ts"),
        ],
        Some(root),
    );
    assert!(
        has_imports_from(
            &out,
            "apps/web/src/consumer.ts",
            "packages/pkg-a/src/browser.ts"
        ),
        "edges: {:?}",
        out.edges
    );
    Ok(())
}

#[test]
fn workspace_subpath_export_wildcard_resolves() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("pnpm-workspace.yaml"),
        "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
    )?;
    write(
        &root.join("packages/pkg-a/package.json"),
        r#"{"name": "@example/pkg-a", "exports": {"./*": {"source": "./src/*.ts"}}}"#,
    )?;
    write(
        &root.join("packages/pkg-a/src/utils.ts"),
        "export function add(a: number, b: number) { return a + b }\n",
    )?;
    write(
        &root.join("apps/web/src/consumer.ts"),
        "import { add } from '@example/pkg-a/utils'\nexport const sum = add(1, 2)\n",
    )?;

    let out = extract(
        &[
            root.join("packages/pkg-a/src/utils.ts"),
            root.join("apps/web/src/consumer.ts"),
        ],
        Some(root),
    );
    assert!(
        has_imports_from(
            &out,
            "apps/web/src/consumer.ts",
            "packages/pkg-a/src/utils.ts"
        ),
        "edges: {:?}",
        out.edges
    );
    Ok(())
}

#[test]
fn workspace_subpath_export_falls_back_to_filesystem() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("pnpm-workspace.yaml"),
        "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
    )?;
    write(
        &root.join("packages/pkg-a/package.json"),
        r#"{"name": "@example/pkg-a"}"#,
    )?;
    write(
        &root.join("packages/pkg-a/browser.ts"),
        "export const value = \"ok\"\n",
    )?;
    write(
        &root.join("apps/web/src/consumer.ts"),
        "import { value } from '@example/pkg-a/browser'\nexport const v = value\n",
    )?;

    let out = extract(
        &[
            root.join("packages/pkg-a/browser.ts"),
            root.join("apps/web/src/consumer.ts"),
        ],
        Some(root),
    );
    assert!(
        has_imports_from(
            &out,
            "apps/web/src/consumer.ts",
            "packages/pkg-a/browser.ts"
        ),
        "edges: {:?}",
        out.edges
    );
    Ok(())
}

#[test]
fn workspace_subpath_export_rejects_path_escape() -> TestResult {
    // An exports target that escapes the package dir must NOT resolve to the
    // outside path (path-containment guard). Resolution falls through to the
    // bare-path fallback, which has no real file here, so no edge lands.
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("pnpm-workspace.yaml"),
        "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
    )?;
    write(
        &root.join("packages/pkg-a/package.json"),
        r#"{"name": "@example/pkg-a", "exports": {"./evil": "../../../../secret.ts"}}"#,
    )?;
    write(&root.join("secret.ts"), "export const leak = \"secret\"\n")?;
    write(
        &root.join("apps/web/src/consumer.ts"),
        "import { leak } from '@example/pkg-a/evil'\nexport const v = leak\n",
    )?;

    let out = extract(
        &[
            root.join("secret.ts"),
            root.join("apps/web/src/consumer.ts"),
        ],
        Some(root),
    );
    assert!(
        !has_imports_from(&out, "apps/web/src/consumer.ts", "secret.ts"),
        "escaped export must not resolve; edges: {:?}",
        out.edges
    );
    Ok(())
}

#[test]
fn workspace_subpath_export_default_consulted_last() -> TestResult {
    // When both `default` and an earlier condition match, the earlier condition
    // (import) must win — `default` is Node's catch-all.
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("pnpm-workspace.yaml"),
        "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
    )?;
    write(
        &root.join("packages/pkg-a/package.json"),
        r#"{"name": "@example/pkg-a", "exports": {"./browser": {"default": "./src/default-entry.ts", "import": "./src/import-entry.ts"}}}"#,
    )?;
    write(
        &root.join("packages/pkg-a/src/import-entry.ts"),
        "export const value = \"import\"\n",
    )?;
    write(
        &root.join("packages/pkg-a/src/default-entry.ts"),
        "export const value = \"default\"\n",
    )?;
    write(
        &root.join("apps/web/src/consumer.ts"),
        "import { value } from '@example/pkg-a/browser'\nexport const v = value\n",
    )?;

    let out = extract(
        &[
            root.join("packages/pkg-a/src/import-entry.ts"),
            root.join("packages/pkg-a/src/default-entry.ts"),
            root.join("apps/web/src/consumer.ts"),
        ],
        Some(root),
    );
    assert!(
        has_imports_from(
            &out,
            "apps/web/src/consumer.ts",
            "packages/pkg-a/src/import-entry.ts"
        ),
        "import condition must win; edges: {:?}",
        out.edges
    );
    assert!(
        !has_imports_from(
            &out,
            "apps/web/src/consumer.ts",
            "packages/pkg-a/src/default-entry.ts"
        ),
        "default must not win over import; edges: {:?}",
        out.edges
    );
    Ok(())
}
