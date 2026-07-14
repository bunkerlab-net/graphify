//! Cross-file Pascal/Delphi inherited-method-call resolution (#1739).
//!
//! Ports `graphify-py/tests/test_pascal_resolution.py`. The per-file extractor
//! resolves calls only within one file; `resolve_pascal_inherited_calls` closes
//! the generated-base / manual-descendant gap as a corpus-wide post-pass walking
//! the `inherits` chain across files.
//!
//! Per AGENTS.md every filesystem test is isolated in a `tempdir()`; the fixture
//! sources are written fresh per test (the Python suite used static fixtures to
//! dodge the pascal project-root walk-up finding stray `.pas` files at a shared
//! temp ancestor — nextest gives each test its own process + fresh tempdir, and
//! the base-class file naming still drives `pascal_resolve_class`).
//!
//! `test_pascal_resolver_registered` (a named-registry lookup) has no Rust
//! equivalent — the resolver registry is suffix-gated, not name-keyed — but its
//! contract (the pascal pass runs) is exercised by the cross-file positive test.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use graphify_extract::{ExtractOutput, FileResult, extract, extract_pascal};
use serde_json::Value;
use tempfile::tempdir;

const BASE_GADGET: &str = "unit BaseGadget;\n\ninterface\n\ntype\n  TBaseGadget = class(TObject)\n  public\n    procedure Prepare;\n  end;\n\nimplementation\n\nprocedure TBaseGadget.Prepare;\nbegin\n  { base prepare }\nend;\n\nend.\n";
const OTHER_GADGET: &str = "unit OtherGadget;\n\ninterface\n\ntype\n  TOtherGadget = class(TObject)\n  public\n    procedure Prepare;\n  end;\n\nimplementation\n\nprocedure TOtherGadget.Prepare;\nbegin\n  { unrelated prepare }\nend;\n\nend.\n";
const DERIVED_GADGET: &str = "unit DerivedGadget;\n\ninterface\n\nuses\n  BaseGadget;\n\ntype\n  TDerivedGadget = class(TBaseGadget)\n  public\n    procedure Run;\n  end;\n\nimplementation\n\nprocedure TDerivedGadget.Run;\nbegin\n  Prepare;\nend;\n\nend.\n";

/// Write the named `.pas` fixtures into `dir` and return their paths.
fn write_fixtures(dir: &Path, files: &[(&str, &str)]) -> Vec<PathBuf> {
    files
        .iter()
        .map(|(name, body)| {
            let p = dir.join(name);
            std::fs::write(&p, body).expect("write pascal fixture");
            p
        })
        .collect()
}

/// The `calls` edge whose source/target node labels match, or `None`.
fn call_edge<'a>(
    out: &'a ExtractOutput,
    src_label: &str,
    tgt_label: &str,
) -> Option<&'a indexmap::IndexMap<String, Value>> {
    let label_of = |id: &str| -> String {
        out.nodes
            .iter()
            .find(|n| n.get("id").and_then(Value::as_str) == Some(id))
            .and_then(|n| n.get("label").and_then(Value::as_str))
            .unwrap_or("")
            .to_string()
    };
    out.edges.iter().find(|e| {
        if e.get("relation").and_then(Value::as_str) != Some("calls") {
            return false;
        }
        let Some(s) = e.get("source").and_then(Value::as_str) else {
            return false;
        };
        let Some(t) = e.get("target").and_then(Value::as_str) else {
            return false;
        };
        label_of(s) == src_label && label_of(t) == tgt_label
    })
}

/// `source_file` of the node with id `id`.
fn source_file_of(out: &ExtractOutput, id: &str) -> String {
    out.nodes
        .iter()
        .find(|n| n.get("id").and_then(Value::as_str) == Some(id))
        .and_then(|n| n.get("source_file").and_then(Value::as_str))
        .unwrap_or("")
        .to_string()
}

/// A `calls` edge in a single-file `FileResult` between the given node labels.
fn file_call_edge(r: &FileResult, src_label: &str, tgt_label: &str) -> bool {
    let label_of = |id: &str| -> &str {
        r.nodes
            .iter()
            .find(|n| n.id == id)
            .map_or("", |n| n.label.as_str())
    };
    r.edges.iter().any(|e| {
        e.relation == "calls"
            && label_of(&e.source) == src_label
            && label_of(&e.target) == tgt_label
    })
}

#[test]
fn single_file_extraction_reports_unresolved_inherited_call() {
    // The per-file extractor cannot see BaseGadget.pas while extracting
    // DerivedGadget.pas, so it must NOT emit a Run -> Prepare `calls` edge, and
    // must report it via `raw_calls` rather than silently dropping it.
    let tmp = tempdir().expect("tempdir");
    let paths = write_fixtures(tmp.path(), &[("DerivedGadget.pas", DERIVED_GADGET)]);
    let r = extract_pascal(&paths[0]);
    assert!(
        !file_call_edge(&r, "Run()", "Prepare()"),
        "single-file extraction must not resolve the inherited call"
    );
    let rc = r
        .raw_calls
        .iter()
        .find(|rc| rc.callee == "prepare")
        .expect("unresolved `prepare` call must be reported as a raw_call");
    assert!(!rc.caller_nid.is_empty());
}

#[test]
fn calls_resolve_across_files_via_inherits_chain() {
    let tmp = tempdir().expect("tempdir");
    let paths = write_fixtures(
        tmp.path(),
        &[
            ("BaseGadget.pas", BASE_GADGET),
            ("DerivedGadget.pas", DERIVED_GADGET),
        ],
    );
    let out = extract(&paths, Some(tmp.path()));
    let edge = call_edge(&out, "Run()", "Prepare()")
        .expect("cross-file inherited call Run -> Prepare must resolve");
    assert_eq!(
        edge.get("confidence").and_then(Value::as_str),
        Some("EXTRACTED")
    );
}

#[test]
fn cross_file_calls_do_not_cross_unrelated_classes() {
    // TDerivedGadget inherits only from TBaseGadget. TOtherGadget declares an
    // unrelated same-named Prepare — Run() must resolve to TBaseGadget.Prepare,
    // never TOtherGadget.Prepare.
    let tmp = tempdir().expect("tempdir");
    let paths = write_fixtures(
        tmp.path(),
        &[
            ("BaseGadget.pas", BASE_GADGET),
            ("OtherGadget.pas", OTHER_GADGET),
            ("DerivedGadget.pas", DERIVED_GADGET),
        ],
    );
    let out = extract(&paths, Some(tmp.path()));
    let edge =
        call_edge(&out, "Run()", "Prepare()").expect("cross-file inherited call must resolve");
    let target = edge.get("target").and_then(Value::as_str).unwrap_or("");
    let target_sf = source_file_of(&out, target);
    assert!(
        target_sf.contains("BaseGadget.pas"),
        "Run() must resolve to TBaseGadget.Prepare; target source_file: {target_sf}"
    );
    assert!(
        !target_sf.contains("OtherGadget.pas"),
        "Run() must not resolve to the unrelated TOtherGadget.Prepare"
    );
}

/// Case-insensitive dispatch (#1671): uppercase `.PAS` files are still Pascal, so
/// the inherited-call post-pass must resolve across them. The resolver-activation
/// gate and the raw-call suffix guard both lowercase; a case-sensitive gate would
/// skip the whole pass and drop the `Run() -> Prepare()` edge.
#[test]
fn uppercase_pas_extension_still_resolves_inherited_call() {
    let tmp = tempdir().expect("tempdir");
    let paths = write_fixtures(
        tmp.path(),
        &[
            ("BaseGadget.PAS", BASE_GADGET),
            ("DerivedGadget.PAS", DERIVED_GADGET),
        ],
    );
    let out = extract(&paths, Some(tmp.path()));
    call_edge(&out, "Run()", "Prepare()")
        .expect("uppercase-.PAS cross-file inherited call must resolve");
}
