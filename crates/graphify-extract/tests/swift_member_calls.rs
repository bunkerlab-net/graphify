//! Parity tests for Swift cross-file member-call resolution (#1356), ported from
//! `graphify-py/tests/test_swift_cross_file_calls.py`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use graphify_build::build_from_json;
use graphify_extract::{ExtractOutput, extract};

fn write_file(root: &Path, rel: &str, text: &str) -> PathBuf {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("create_dir_all");
    std::fs::write(&p, text).expect("write");
    p
}

fn label_of<'a>(res: &'a ExtractOutput, nid: &str) -> &'a str {
    res.nodes
        .iter()
        .find(|n| n.get("id").and_then(|v| v.as_str()) == Some(nid))
        .and_then(|n| n.get("label").and_then(|v| v.as_str()))
        .unwrap_or("")
}

/// `{(source_label, relation, target_label)}` for the given relations.
fn edge_labels(res: &ExtractOutput, relations: &[&str]) -> HashSet<(String, String, String)> {
    res.edges
        .iter()
        .filter_map(|e| {
            let rel = e.get("relation").and_then(|v| v.as_str())?;
            if !relations.contains(&rel) {
                return None;
            }
            let src = label_of(res, e.get("source").and_then(|v| v.as_str())?);
            let tgt = label_of(res, e.get("target").and_then(|v| v.as_str())?);
            Some((src.to_string(), rel.to_string(), tgt.to_string()))
        })
        .collect()
}

/// The three cross-file patterns from #1356, plus a constructor-in-initializer.
fn issue_fixture(base: &Path) -> Vec<PathBuf> {
    vec![
        write_file(
            base,
            "Models/SessionViewModel.swift",
            "class SessionViewModel {\n    func update() {}\n}\n",
        ),
        write_file(
            base,
            "Services/NetworkService.swift",
            "class NetworkService {\n    func fetch() {}\n}\n",
        ),
        write_file(
            base,
            "Core/SessionType.swift",
            "enum SessionType {\n    static func staticMethod() {}\n}\n",
        ),
        write_file(
            base,
            "Core/Singleton.swift",
            "class Singleton {\n    static let shared = Singleton()\n    func method() {}\n}\n",
        ),
        write_file(
            base,
            "Views/HomeView.swift",
            "class HomeView {\n\
             \x20   let vm = SessionViewModel()\n\
             \x20   var svc: NetworkService\n\n\
             \x20   func go() {\n\
             \x20       vm.update()\n\
             \x20       SessionType.staticMethod()\n\
             \x20       Singleton.shared.method()\n\
             \x20       self.svc.fetch()\n\
             \x20   }\n\
             }\n",
        ),
    ]
}

#[test]
fn swift_cross_file_member_calls_resolve() {
    let tmp = tempfile::tempdir().unwrap();
    let files = issue_fixture(&tmp.path().join("src"));
    let res = extract(&files, Some(&tmp.path().join("cache")));
    let edges = edge_labels(&res, &["calls", "references"]);
    let want =
        |s: &str, r: &str, t: &str| edges.contains(&(s.to_string(), r.to_string(), t.to_string()));
    // Stage 1: constructor in a property initializer.
    assert!(want("HomeView", "calls", "SessionViewModel"), "{edges:?}");
    // Stage 2: receiver typed via the file's local type table.
    assert!(want(".go()", "calls", ".update()"), "{edges:?}");
    assert!(want(".go()", "calls", ".fetch()"), "{edges:?}");
    // Stage 2: upper-cased receiver is itself a type.
    assert!(want(".go()", "calls", ".staticMethod()"), "{edges:?}");
    assert!(want(".go()", "calls", ".method()"), "{edges:?}");
}

#[test]
fn swift_cross_file_member_calls_are_inferred_and_resolve_to_real_nodes() {
    let tmp = tempfile::tempdir().unwrap();
    let files = issue_fixture(&tmp.path().join("src"));
    let res = extract(&files, Some(&tmp.path().join("cache")));

    let node_ids: HashSet<&str> = res
        .nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(|v| v.as_str()))
        .collect();
    let member_targets: HashSet<&str> =
        [".update()", ".fetch()", ".staticMethod()", ".method()"].into();
    let mut seen: HashSet<String> = HashSet::new();
    let mut inferred_calls = 0;
    for e in &res.edges {
        let rel = e.get("relation").and_then(|v| v.as_str()).unwrap_or("");
        let conf = e.get("confidence").and_then(|v| v.as_str()).unwrap_or("");
        if rel == "calls" && conf == "INFERRED" {
            inferred_calls += 1;
        }
        let tgt = e.get("target").and_then(|v| v.as_str()).unwrap_or("");
        let tgt_label = label_of(&res, tgt);
        if rel == "calls" && member_targets.contains(tgt_label) {
            assert_eq!(conf, "INFERRED");
            assert_eq!(
                e.get("confidence_score")
                    .and_then(serde_json::Value::as_f64),
                Some(0.8)
            );
            assert!(node_ids.contains(tgt), "member target not a node: {tgt}");
            let sf = res
                .nodes
                .iter()
                .find(|n| n.get("id").and_then(|v| v.as_str()) == Some(tgt))
                .and_then(|n| n.get("source_file").and_then(|v| v.as_str()))
                .unwrap_or("");
            assert!(!sf.is_empty(), "member target not source-backed: {tgt}");
            seen.insert(tgt_label.to_string());
        }
    }
    let want: HashSet<String> = member_targets.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(seen, want);
    assert!(
        inferred_calls >= 5,
        "expected >=5 INFERRED calls, got {inferred_calls}"
    );

    // Edges survive graph construction (targets are real nodes).
    let g = build_from_json(serde_json::to_value(&res).unwrap(), true, None).expect("build");
    for t in &member_targets {
        let nid = res
            .nodes
            .iter()
            .find(|n| n.get("label").and_then(|v| v.as_str()) == Some(*t))
            .and_then(|n| n.get("id").and_then(|v| v.as_str()))
            .unwrap_or("");
        assert!(g.contains_node(nid), "member node {t} pruned by build");
    }
}

#[test]
fn swift_ambiguous_type_does_not_over_connect() {
    // #543/#1219 guard: a receiver type defined in 2+ files must bail.
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("src");
    let mut files: Vec<PathBuf> = ["a", "b", "c"]
        .iter()
        .map(|sub| {
            write_file(
                &base,
                &format!("{sub}/Widget.swift"),
                "class Widget {\n    func update() {}\n}\n",
            )
        })
        .collect();
    files.push(write_file(
        &base,
        "Caller.swift",
        "class Caller {\n\
         \x20   var w: Widget\n\
         \x20   func run() {\n\
         \x20       w.update()\n\
         \x20       unknown.update()\n\
         \x20   }\n\
         }\n",
    ));
    files.sort();
    let res = extract(&files, Some(&tmp.path().join("cache")));

    let inferred: Vec<_> = res
        .edges
        .iter()
        .filter(|e| {
            e.get("relation").and_then(|v| v.as_str()) == Some("calls")
                && e.get("confidence").and_then(|v| v.as_str()) == Some("INFERRED")
        })
        .collect();
    assert!(
        inferred.is_empty(),
        "ambiguous/unknown must not connect: {inferred:?}"
    );
}

#[test]
fn swift_unknown_receiver_emits_no_edge() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("src");
    let mut files = vec![
        write_file(
            &base,
            "Helper.swift",
            "class Helper {\n    func help() {}\n}\n",
        ),
        write_file(
            &base,
            "Caller.swift",
            "class Caller {\n    func run() {\n        mystery.help()\n    }\n}\n",
        ),
    ];
    files.sort();
    let res = extract(&files, Some(&tmp.path().join("cache")));
    let edges = edge_labels(&res, &["calls"]);
    assert!(
        !edges.contains(&(
            ".run()".to_string(),
            "calls".to_string(),
            ".help()".to_string()
        )),
        "unknown receiver should not resolve: {edges:?}"
    );
}
