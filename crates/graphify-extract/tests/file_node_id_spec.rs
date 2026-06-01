//! Regression tests for issue #1033 / #1096: AST file-level node IDs must match
//! the skill.md `{parent_dir}_{stem}` spec (one parent level, no extension) so
//! AST and semantic extraction produce the SAME node for a file instead of two
//! disconnected ghosts.
//!
//! Mirrors `graphify-py/tests/test_file_node_id_spec.py`.
//!
//! skill.md spec:
//!
//! ```text
//! stem = {parent_dir}_{filename_without_ext}, lowercased, non-alphanumeric -> _
//! examples:
//!     src/auth/session.py + ValidateToken -> auth_session_validatetoken
//!     match/script/pipeline_step.py (file node) -> script_pipeline_step
//!     setup.py (top-level) -> setup
//! ```

use std::collections::HashSet;
use std::error::Error;
use std::path::Path;

use graphify_extract::extract;
use indexmap::IndexMap;
use serde_json::Value;
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn Error>>;

/// All node ids in the extraction result.
fn ids(nodes: &[IndexMap<String, Value>]) -> HashSet<String> {
    nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(Value::as_str).map(str::to_string))
        .collect()
}

fn write(path: &Path, text: &str) -> TestResult {
    std::fs::create_dir_all(path.parent().ok_or("path has no parent")?)?;
    std::fs::write(path, text)?;
    Ok(())
}

#[test]
fn file_node_id_uses_parent_dir_and_stem_no_extension() -> TestResult {
    // match/script/pipeline_step.py -> file node id 'script_pipeline_step'.
    let tmp = tempdir()?;
    let root = tmp.path().canonicalize()?;
    let f = root.join("match").join("script").join("pipeline_step.py");
    write(&f, "def run():\n    pass\n")?;

    let result = extract(&[f], Some(&root));
    let ids = ids(&result.nodes);

    assert!(
        ids.contains("script_pipeline_step"),
        "expected spec-format file id 'script_pipeline_step', got {ids:?}"
    );
    // The old buggy full-path-with-extension id must be gone.
    assert!(!ids.contains("match_script_pipeline_step_py"));
    assert!(
        !ids.iter()
            .any(|i| i.ends_with("_py") && i.contains("pipeline_step"))
    );
    Ok(())
}

#[test]
fn top_level_file_node_id_is_bare_stem() -> TestResult {
    // A file directly at the project root collapses to just its stem.
    let tmp = tempdir()?;
    let root = tmp.path().canonicalize()?;
    let f = root.join("setup.py");
    write(&f, "def configure():\n    pass\n")?;

    let result = extract(&[f], Some(&root));
    let ids = ids(&result.nodes);

    assert!(
        ids.contains("setup"),
        "expected bare stem 'setup', got {ids:?}"
    );
    assert!(!ids.contains("setup_py"));
    Ok(())
}

#[test]
fn top_level_file_symbol_ids_use_bare_stem() -> TestResult {
    // A SYMBOL in a root-level file must use the bare-stem prefix (`setup_configure`),
    // not pick up the project-root directory name (`<rootdir>_setup_configure`) (#1096).
    let tmp = tempdir()?;
    let root = tmp.path().canonicalize()?;
    let f = root.join("main.py");
    write(&f, "def run():\n    return 1\n")?;

    let result = extract(&[f], Some(&root));
    let ids = ids(&result.nodes);

    assert!(
        ids.contains("main_run"),
        "expected bare-stem symbol 'main_run', got {ids:?}"
    );
    // The root directory name must NOT appear in any symbol id.
    let rootname = root
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase().replace('-', "_"))
        .unwrap_or_default();
    assert!(
        rootname.is_empty() || !ids.iter().any(|i| i.contains(&rootname)),
        "root dir name leaked into ids: {ids:?}"
    );

    // contains edge file -> symbol must connect with the canonical ids.
    let contains: Vec<_> = result
        .edges
        .iter()
        .filter(|e| {
            e.get("relation").and_then(Value::as_str) == Some("contains")
                && e.get("target").and_then(Value::as_str) == Some("main_run")
        })
        .collect();
    assert!(!contains.is_empty());
    assert_eq!(
        contains[0].get("source").and_then(Value::as_str),
        Some("main")
    );
    Ok(())
}

#[test]
fn nested_file_symbol_ids_unchanged() -> TestResult {
    // Regression guard: nested files (immediate parent identical in abs/rel form)
    // must be completely unaffected by the symbol remap.
    let tmp = tempdir()?;
    let root = tmp.path().canonicalize()?;
    let f = root.join("sub").join("mod.py");
    write(&f, "def work():\n    return 2\n")?;

    let result = extract(&[f], Some(&root));
    let ids = ids(&result.nodes);
    assert!(ids.contains("sub_mod"));
    assert!(ids.contains("sub_mod_work"));
    Ok(())
}

#[test]
fn symbol_and_file_ids_share_the_same_stem() -> TestResult {
    // Symbol ids already use {parent}_{stem}_{name}; the file node must share
    // that stem prefix so 'contains' edges connect file -> symbol.
    let tmp = tempdir()?;
    let root = tmp.path().canonicalize()?;
    let f = root.join("match").join("script").join("pipeline_step.py");
    write(&f, "def run():\n    pass\n\nclass Stage:\n    pass\n")?;

    let result = extract(&[f], Some(&root));
    let ids = ids(&result.nodes);

    assert!(ids.contains("script_pipeline_step")); // file node
    assert!(ids.contains("script_pipeline_step_stage")); // class symbol shares stem

    // The file -> class 'contains' edge must reference the real file node id.
    let contains: Vec<_> = result
        .edges
        .iter()
        .filter(|e| {
            e.get("relation").and_then(Value::as_str) == Some("contains")
                && e.get("target").and_then(Value::as_str) == Some("script_pipeline_step_stage")
        })
        .collect();
    assert!(
        !contains.is_empty(),
        "no 'contains' edge to the class symbol"
    );
    assert_eq!(
        contains[0].get("source").and_then(Value::as_str),
        Some("script_pipeline_step"),
    );
    Ok(())
}

#[test]
fn cross_file_import_edges_stay_connected() -> TestResult {
    // Changing the file-id format must not orphan import edges.
    let tmp = tempdir()?;
    let root = tmp.path().canonicalize()?;
    let pkg = root.join("pkg");
    write(&pkg.join("models.py"), "class User:\n    pass\n")?;
    write(
        &pkg.join("auth.py"),
        "from models import User\n\nclass Session:\n    def check(self):\n        return User()\n",
    )?;

    let files = vec![pkg.join("models.py"), pkg.join("auth.py")];
    let result = extract(&files, Some(&root));
    let ids = ids(&result.nodes);

    assert!(ids.contains("pkg_models"));
    assert!(ids.contains("pkg_auth"));

    // No edge endpoint may keep the old extension-suffixed `*_py` format.
    for e in &result.edges {
        for key in ["source", "target"] {
            let endpoint = e.get(key).and_then(Value::as_str).unwrap_or("");
            assert!(
                !endpoint.ends_with("_py"),
                "edge endpoint {endpoint:?} kept the old extension-suffixed format"
            );
        }
    }
    Ok(())
}
