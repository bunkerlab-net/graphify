//! #1529: alias/workspace import targets orphaned by the full-path id migration.
//!
//! Ports `test_alias_import_edge_resolves_with_relative_input_paths` from
//! `graphify-py/tests/test_js_import_resolution.py`. The bug only reproduces with
//! RELATIVE input paths (so the input-form file id differs from the
//! absolute-resolved form an alias import keys its edge target off of); absolute
//! / `tmp_path` inputs hide it because the two forms coincide.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use graphify_extract::{ExtractOutput, extract, file_node_id, file_stem, make_id};
use serde_json::Value;
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Restore the process CWD on drop so this chdir-based test can't leak into
/// other tests (nextest also isolates each test in its own process).
struct CwdGuard(PathBuf);

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}

fn write(path: &Path, text: &str) -> TestResult {
    std::fs::create_dir_all(path.parent().ok_or("path has no parent")?)?;
    std::fs::write(path, text)?;
    Ok(())
}

fn edge_targets_from(out: &ExtractOutput, source_id: &str, relation: &str) -> Vec<String> {
    out.edges
        .iter()
        .filter(|e| {
            e.get("source").and_then(Value::as_str) == Some(source_id)
                && e.get("relation").and_then(Value::as_str) == Some(relation)
        })
        .filter_map(|e| e.get("target").and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// Mirrors Python `_has_symbol_edge`: a named-symbol `imports` edge from
/// `source` to `make_id(file_stem(target_file), symbol)`.
fn has_symbol_edge(out: &ExtractOutput, source: &str, target_file: &str, symbol: &str) -> bool {
    let s = file_node_id(Path::new(source));
    let t = make_id(&[&file_stem(Path::new(target_file)), symbol]);
    out.edges.iter().any(|e| {
        e.get("source").and_then(Value::as_str) == Some(s.as_str())
            && e.get("target").and_then(Value::as_str) == Some(t.as_str())
            && e.get("relation").and_then(Value::as_str) == Some("imports")
    })
}

#[test]
fn alias_import_edge_resolves_with_relative_input_paths() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path().canonicalize()?;
    write(
        &root.join("tsconfig.json"),
        r#"{"compilerOptions": {"baseUrl": ".", "paths": {"@/*": ["src/*"]}}}"#,
    )?;
    write(
        &root.join("src/lib/utils.ts"),
        "export function formatDate(d) { return d }\n",
    )?;
    write(
        &root.join("src/components/Button.tsx"),
        "import { formatDate } from '@/lib/utils'\nexport function Button() { return formatDate(1) }\n",
    )?;

    let original = std::env::current_dir()?;
    let _guard = CwdGuard(original);
    std::env::set_current_dir(&root)?;

    // CRUCIAL: relative input paths. Alias imports resolve specifiers through an
    // absolute path, so the import-target id is keyed off the ABSOLUTE form; with
    // relative inputs the id-remap (keyed on the input form) must also cover the
    // absolute form or the edge orphans and is dropped (#1529).
    let rel_paths = [
        PathBuf::from("src/lib/utils.ts"),
        PathBuf::from("src/components/Button.tsx"),
    ];
    let out = extract(&rel_paths, Some(Path::new(".")));

    let node_ids: HashSet<&str> = out
        .nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(Value::as_str))
        .collect();
    let target_id = file_node_id(Path::new("src/lib/utils.ts"));
    let button_id = file_node_id(Path::new("src/components/Button.tsx"));

    // The file-level imports_from edge must target the REAL utils file node.
    let import_targets = edge_targets_from(&out, &button_id, "imports_from");
    assert!(
        node_ids.contains(target_id.as_str()),
        "real utils file node missing; nodes: {node_ids:?}"
    );
    assert!(
        import_targets.iter().any(|t| t == &target_id),
        "imports_from must target the real utils node; targets: {import_targets:?}"
    );

    // No surviving edge target may carry an absolute-path prefix from the sandbox.
    let abs_prefix = file_node_id(&Path::new("src/lib/utils.ts").canonicalize()?);
    assert!(
        import_targets
            .iter()
            .all(|t| !t.starts_with(&format!("{abs_prefix}_")) && t != &abs_prefix),
        "an import target kept an absolute prefix ({abs_prefix}); targets: {import_targets:?}"
    );

    // The named-symbol edge to formatDate must resolve to the real symbol node.
    assert!(
        has_symbol_edge(
            &out,
            "src/components/Button.tsx",
            "src/lib/utils.ts",
            "formatDate"
        ),
        "named-symbol import edge to formatDate did not resolve; edges: {:?}",
        out.edges
    );
    Ok(())
}

/// File→file `imports_from` edge, mirroring Python `_has_edge`.
fn has_file_edge(out: &ExtractOutput, source_rel: &str, target_rel: &str) -> bool {
    let s = file_node_id(Path::new(source_rel));
    let t = file_node_id(Path::new(target_rel));
    out.edges.iter().any(|e| {
        e.get("source").and_then(Value::as_str) == Some(s.as_str())
            && e.get("target").and_then(Value::as_str) == Some(t.as_str())
            && e.get("relation").and_then(Value::as_str) == Some("imports_from")
    })
}

/// 5746964 (#927): `@*` → `features/*/src/` substitutes the captured segment.
#[test]
fn tsconfig_wildcard_alias_substitutes_captured_path() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path().canonicalize()?;
    write(
        &root.join("tsconfig.json"),
        r#"{"compilerOptions": {"baseUrl": ".", "paths": {"@*": ["features/*/src/"]}}}"#,
    )?;
    write(
        &root.join("features/communicate/documentv2/src/index.ts"),
        "export const FileChipComponent = {}\n",
    )?;
    write(
        &root.join("src/routes/page.ts"),
        "import { FileChipComponent } from '@communicate/documentv2'\n",
    )?;
    let out = extract(
        &[
            root.join("features/communicate/documentv2/src/index.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(&root),
    );
    assert!(has_file_edge(
        &out,
        "src/routes/page.ts",
        "features/communicate/documentv2/src/index.ts"
    ));
    Ok(())
}

/// The star substitutes before a fixed suffix (`@*/interfaces`).
#[test]
fn tsconfig_wildcard_alias_substitutes_before_suffix() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path().canonicalize()?;
    write(
        &root.join("tsconfig.json"),
        r#"{"compilerOptions": {"baseUrl": ".", "paths": {"@*/interfaces": ["features/*/src/interfaces.ts"]}}}"#,
    )?;
    write(
        &root.join("features/communicate/src/interfaces.ts"),
        "export interface Message { id: string }\n",
    )?;
    write(
        &root.join("src/routes/page.ts"),
        "import type { Message } from '@communicate/interfaces'\n",
    )?;
    let out = extract(
        &[
            root.join("features/communicate/src/interfaces.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(&root),
    );
    assert!(has_file_edge(
        &out,
        "src/routes/page.ts",
        "features/communicate/src/interfaces.ts"
    ));
    Ok(())
}

/// Substitution happens before normalising the target (`generated/*/../shared`).
#[test]
fn tsconfig_wildcard_alias_substitutes_before_normalizing_target() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path().canonicalize()?;
    write(
        &root.join("tsconfig.json"),
        r#"{"compilerOptions": {"baseUrl": ".", "paths": {"@/*": ["generated/*/../shared"]}}}"#,
    )?;
    write(
        &root.join("generated/feature/shared/index.ts"),
        "export const shared = 1\n",
    )?;
    write(
        &root.join("src/routes/page.ts"),
        "import { shared } from '@/feature/nested'\n",
    )?;
    let out = extract(
        &[
            root.join("generated/feature/shared/index.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(&root),
    );
    assert!(has_file_edge(
        &out,
        "src/routes/page.ts",
        "generated/feature/shared/index.ts"
    ));
    Ok(())
}

/// A wildcard prefix matches with an empty capture: `app*` resolves `'app'`.
#[test]
fn tsconfig_wildcard_alias_allows_empty_capture() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path().canonicalize()?;
    write(
        &root.join("tsconfig.json"),
        r#"{"compilerOptions": {"baseUrl": ".", "paths": {"app*": ["src/config/index.ts"]}}}"#,
    )?;
    write(
        &root.join("src/config/index.ts"),
        "export const config = {}\n",
    )?;
    write(
        &root.join("src/routes/page.ts"),
        "import { config } from 'app'\n",
    )?;
    let out = extract(
        &[
            root.join("src/config/index.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(&root),
    );
    assert!(has_file_edge(
        &out,
        "src/routes/page.ts",
        "src/config/index.ts"
    ));
    Ok(())
}

/// The longest matching prefix wins (`@/common/integration/*` over `@/*`).
#[test]
fn tsconfig_wildcard_alias_prefers_longest_matching_prefix() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path().canonicalize()?;
    write(
        &root.join("tsconfig.json"),
        r#"{"compilerOptions": {"baseUrl": ".", "paths": {"@/*": ["fallback/*"], "@/common/integration/*": ["preferred/*"]}}}"#,
    )?;
    write(
        &root.join("fallback/common/integration/foo.ts"),
        "export const Foo = 1\n",
    )?;
    write(&root.join("preferred/foo.ts"), "export const Foo = 2\n")?;
    write(
        &root.join("src/routes/page.ts"),
        "import { Foo } from '@/common/integration/foo'\n",
    )?;
    let out = extract(
        &[
            root.join("fallback/common/integration/foo.ts"),
            root.join("preferred/foo.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(&root),
    );
    assert!(has_file_edge(
        &out,
        "src/routes/page.ts",
        "preferred/foo.ts"
    ));
    assert!(!has_file_edge(
        &out,
        "src/routes/page.ts",
        "fallback/common/integration/foo.ts"
    ));
    Ok(())
}

/// An exact (non-wildcard) alias still resolves (`app-config`).
#[test]
fn tsconfig_exact_alias_still_resolves() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path().canonicalize()?;
    write(
        &root.join("tsconfig.json"),
        r#"{"compilerOptions": {"baseUrl": ".", "paths": {"app-config": ["src/config/index.ts"]}}}"#,
    )?;
    write(
        &root.join("src/config/index.ts"),
        "export const config = {}\n",
    )?;
    write(
        &root.join("src/routes/page.ts"),
        "import { config } from 'app-config'\n",
    )?;
    let out = extract(
        &[
            root.join("src/config/index.ts"),
            root.join("src/routes/page.ts"),
        ],
        Some(&root),
    );
    assert!(has_file_edge(
        &out,
        "src/routes/page.ts",
        "src/config/index.ts"
    ));
    Ok(())
}
