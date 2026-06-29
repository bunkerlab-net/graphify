//! Parity tests for Java records, constructor calls, and cross-file type-reference
//! resolution (#1373, #1318), ported from
//! `graphify-py/tests/test_java_type_resolution.py`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use graphify_build::build_from_json;
use graphify_extract::{ExtractOutput, extract};

fn write_file(root: &Path, rel: &str, text: &str) -> PathBuf {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("create_dir_all");
    std::fs::write(&p, text).expect("write");
    p
}

fn node_source_file<'a>(res: &'a ExtractOutput, nid: &str) -> Option<&'a str> {
    res.nodes
        .iter()
        .find(|n| n.get("id").and_then(|v| v.as_str()) == Some(nid))
        .and_then(|n| n.get("source_file").and_then(|v| v.as_str()))
}

fn edges_with_relation<'a>(res: &'a ExtractOutput, relation: &str) -> Vec<&'a str> {
    res.edges
        .iter()
        .filter(|e| e.get("relation").and_then(|v| v.as_str()) == Some(relation))
        .filter_map(|e| e.get("target").and_then(|v| v.as_str()))
        .collect()
}

#[test]
fn java_cross_file_implements_resolves_to_real_def() {
    // #1318: a cross-file `implements` must land on the real interface def, not a
    // bare no-source shadow stub.
    let tmp = tempfile::tempdir().unwrap();
    let iface = write_file(
        tmp.path(),
        "src/com/x/handler/AIResponseHandler.java",
        "package com.x.handler;\npublic interface AIResponseHandler {}\n",
    );
    let imp = write_file(
        tmp.path(),
        "src/com/x/service/DifyAiServiceImpl.java",
        "package com.x.service;\n\
         import com.x.handler.AIResponseHandler;\n\
         public class DifyAiServiceImpl implements AIResponseHandler {}\n",
    );
    let res = extract(&[iface, imp], Some(tmp.path()));

    let implements = edges_with_relation(&res, "implements");
    assert!(!implements.is_empty(), "expected an implements edge");
    for tgt in &implements {
        let sf = node_source_file(&res, tgt).unwrap_or("");
        assert!(!sf.is_empty(), "implements landed on shadow stub: {tgt}");
        assert!(sf.contains("handler"), "{sf}");
    }
}

#[test]
fn java_ambiguous_implements_disambiguated_by_import() {
    // #1318 core case: two interfaces with the SAME simple name in different
    // packages; the importing file's `import` must pick the right one, with no
    // orphan shadow node left behind.
    let tmp = tempfile::tempdir().unwrap();
    let a = write_file(
        tmp.path(),
        "src/com/a/handler/AIResponseHandler.java",
        "package com.a.handler;\npublic interface AIResponseHandler {}\n",
    );
    let b = write_file(
        tmp.path(),
        "src/com/b/handler/AIResponseHandler.java",
        "package com.b.handler;\npublic interface AIResponseHandler {}\n",
    );
    let imp = write_file(
        tmp.path(),
        "src/com/x/service/Impl.java",
        "package com.x.service;\n\
         import com.a.handler.AIResponseHandler;\n\
         public class Impl implements AIResponseHandler {}\n",
    );
    let res = extract(&[a, b, imp], Some(tmp.path()));

    let shadow: Vec<&str> = res
        .nodes
        .iter()
        .filter(|n| {
            n.get("label").and_then(|v| v.as_str()) == Some("AIResponseHandler")
                && n.get("source_file")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .is_empty()
        })
        .filter_map(|n| n.get("id").and_then(|v| v.as_str()))
        .collect();
    assert!(
        shadow.is_empty(),
        "orphan shadow node(s) remain: {shadow:?}"
    );

    let implements = edges_with_relation(&res, "implements");
    assert_eq!(implements.len(), 1, "{implements:?}");
    let sf = node_source_file(&res, implements[0]).unwrap_or("");
    assert!(!sf.is_empty(), "implements landed on shadow stub");
    assert!(sf.contains("com/a/handler"), "{sf}");
    assert!(!sf.contains("com/b/handler"), "{sf}");
}

#[test]
fn java_implements_edge_survives_build() {
    // #1318: the re-pointed edge must connect real nodes after graph assembly, so
    // the interface is not classified as an isolated community.
    let tmp = tempfile::tempdir().unwrap();
    let iface = write_file(
        tmp.path(),
        "src/com/x/handler/Handler.java",
        "package com.x.handler;\npublic interface Handler {}\n",
    );
    let imp = write_file(
        tmp.path(),
        "src/com/x/service/Svc.java",
        "package com.x.service;\n\
         import com.x.handler.Handler;\n\
         public class Svc implements Handler {}\n",
    );
    let res = extract(&[iface, imp], Some(tmp.path()));
    let edge = res
        .edges
        .iter()
        .find(|e| e.get("relation").and_then(|v| v.as_str()) == Some("implements"))
        .expect("implements edge");
    let src = edge.get("source").and_then(|v| v.as_str()).expect("source");
    let tgt = edge.get("target").and_then(|v| v.as_str()).expect("target");

    let g = build_from_json(serde_json::to_value(&res).unwrap(), true, None).expect("build");
    assert!(g.contains_node(tgt), "interface node missing after build");
    assert!(
        g.edge_data(src, tgt).is_some(),
        "implements edge pruned during build"
    );
}

#[test]
fn java_record_becomes_type_node() {
    // #1373: a Java `record` must produce a first-class type node (with a
    // `contains` edge from its file), not be left as an isolated file node.
    let tmp = tempfile::tempdir().unwrap();
    let rec = write_file(
        tmp.path(),
        "Foo.java",
        "package com.app;\npublic record Foo(int x, String y) {}\n",
    );
    let res = extract(&[rec], Some(tmp.path()));

    let foo = res.nodes.iter().any(|n| {
        n.get("label").and_then(|v| v.as_str()) == Some("Foo")
            && !n
                .get("source_file")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .is_empty()
    });
    assert!(
        foo,
        "record Foo should be a type node, not just the file node"
    );

    let label_of = |nid: Option<&str>| -> Option<&str> {
        let nid = nid?;
        res.nodes
            .iter()
            .find(|n| n.get("id").and_then(|v| v.as_str()) == Some(nid))
            .and_then(|n| n.get("label").and_then(|v| v.as_str()))
    };
    let contains = res.edges.iter().any(|e| {
        e.get("relation").and_then(|v| v.as_str()) == Some("contains")
            && label_of(e.get("source").and_then(|v| v.as_str())) == Some("Foo.java")
            && label_of(e.get("target").and_then(|v| v.as_str())) == Some("Foo")
    });
    assert!(contains, "expected (Foo.java, contains, Foo)");
}

#[test]
fn java_record_implements_interface() {
    // Records reuse class interface handling: `record Foo implements I` emits it.
    let tmp = tempfile::tempdir().unwrap();
    let iface = write_file(
        tmp.path(),
        "I.java",
        "package com.app;\npublic interface I {}\n",
    );
    let rec = write_file(
        tmp.path(),
        "Foo.java",
        "package com.app;\npublic record Foo(int x) implements I {}\n",
    );
    let res = extract(&[iface, rec], Some(tmp.path()));
    assert!(
        !edges_with_relation(&res, "implements").is_empty(),
        "record implementing an interface should emit an implements edge"
    );
}

#[test]
fn java_cross_file_constructor_call_resolves() {
    // #1373: `new Foo(...)` in a method body must produce a cross-file edge to the
    // Foo definition. Foo is NOT a return type here, so the edge can only come
    // from the constructor call (object_creation_expression).
    let tmp = tempfile::tempdir().unwrap();
    let foo = write_file(
        tmp.path(),
        "Foo.java",
        "package com.app;\npublic record Foo(int x, String y) {}\n",
    );
    let caller = write_file(
        tmp.path(),
        "Helper.java",
        "package com.app;\n\
         public class Helper {\n\
         \x20   public void build() {\n\
         \x20       Object o = new Foo(1, \"a\");\n\
         \x20       System.out.println(o);\n\
         \x20   }\n\
         }\n",
    );
    let res = extract(&[foo, caller], Some(tmp.path()));

    let foo_id = res
        .nodes
        .iter()
        .find(|n| {
            n.get("label").and_then(|v| v.as_str()) == Some("Foo")
                && !n
                    .get("source_file")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .is_empty()
        })
        .and_then(|n| n.get("id").and_then(|v| v.as_str()))
        .expect("Foo node");

    let call_targets: HashSet<&str> = res
        .edges
        .iter()
        .filter(|e| {
            matches!(
                e.get("relation").and_then(|v| v.as_str()),
                Some("calls" | "references")
            )
        })
        .filter_map(|e| e.get("target").and_then(|v| v.as_str()))
        .collect();
    assert!(
        call_targets.contains(foo_id),
        "new Foo(...) should produce a calls/references edge to Foo"
    );

    let g = build_from_json(serde_json::to_value(&res).unwrap(), false, None).expect("build");
    assert!(g.contains_node(foo_id), "Foo node missing after build");
}

#[test]
fn java_type_parameters_do_not_resolve_to_real_class() {
    // #1518: a generic field `List<T>` must not emit a references edge to a real
    // same-named class `T` — `T` is a type variable, not a type.
    let tmp = tempfile::tempdir().unwrap();
    let real_type = write_file(tmp.path(), "T.java", "public class T {}\n");
    let generic = write_file(
        tmp.path(),
        "Generic.java",
        "public class Generic<T> { java.util.List<T> values; }\n",
    );
    let res = extract(&[real_type, generic], Some(tmp.path()));

    let id_to_label: HashMap<&str, &str> = res
        .nodes
        .iter()
        .filter_map(|n| Some((n.get("id")?.as_str()?, n.get("label")?.as_str()?)))
        .collect();
    let has_generic_t_ref = res.edges.iter().any(|e| {
        e.get("relation").and_then(|v| v.as_str()) == Some("references")
            && e.get("source")
                .and_then(|v| v.as_str())
                .and_then(|s| id_to_label.get(s).copied())
                == Some("Generic")
            && e.get("target")
                .and_then(|v| v.as_str())
                .and_then(|t| id_to_label.get(t).copied())
                == Some("T")
    });
    assert!(
        !has_generic_t_ref,
        "type parameter T must not resolve to the real T class"
    );
}
