//! Parity tests for Objective-C macro handling, quoted-import resolution, and
//! alloc/init type references (#1475), ported from `graphify-py/tests/test_languages.py`.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::case_sensitive_file_extension_comparisons
)]

use std::collections::{HashMap, HashSet};

use graphify_extract::{ExtractOutput, FileResult, extract, extract_objc};
use serde_json::Value;

/// `(source_label, target_label)` pairs for `relation` edges of a single-file
/// `FileResult`.
fn fr_label_edges(r: &FileResult, relation: &str) -> HashSet<(String, String)> {
    let id2label: HashMap<&str, &str> = r
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    r.edges
        .iter()
        .filter(|e| e.relation == relation)
        .filter_map(|e| {
            Some((
                (*id2label.get(e.source.as_str())?).to_string(),
                (*id2label.get(e.target.as_str())?).to_string(),
            ))
        })
        .collect()
}

/// `(source_label, target_label)` pairs for `relation` edges of a multi-file
/// `ExtractOutput`.
fn eo_label_edges(r: &ExtractOutput, relation: &str) -> HashSet<(String, String)> {
    let id2label: HashMap<&str, &str> = r
        .nodes
        .iter()
        .filter_map(|n| Some((n.get("id")?.as_str()?, n.get("label")?.as_str()?)))
        .collect();
    r.edges
        .iter()
        .filter(|e| e.get("relation").and_then(Value::as_str) == Some(relation))
        .filter_map(|e| {
            let s = e.get("source").and_then(Value::as_str)?;
            let t = e.get("target").and_then(Value::as_str)?;
            Some((
                (*id2label.get(s)?).to_string(),
                (*id2label.get(t)?).to_string(),
            ))
        })
        .collect()
}

#[test]
fn objc_ns_assume_nonnull_macro_does_not_break_parsing() {
    // `NS_ASSUME_NONNULL_BEGIN` before `@interface` made tree-sitter-objc fail to
    // emit a class_interface node, swallowing the whole interface; blanking the
    // argument-less macro restores it (#1475).
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("AlertManager.h");
    std::fs::write(
        &p,
        "#import <Foundation/Foundation.h>\n\
         NS_ASSUME_NONNULL_BEGIN\n\
         @class Other;\n\
         @interface AlertManager : NSObject\n\
         - (void)show;\n\
         @end\n\
         NS_ASSUME_NONNULL_END\n",
    )
    .unwrap();
    let r = extract_objc(&p);
    let labels: HashSet<&str> = r.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains("AlertManager"), "{labels:?}");
    assert!(
        fr_label_edges(&r, "inherits").contains(&("AlertManager".into(), "NSObject".into())),
        "AlertManager should inherit NSObject"
    );
    // `@class Other;` is only a forward declaration; it must not mint a class node.
    assert!(!labels.contains("Other"), "{labels:?}");
}

#[test]
fn objc_macro_free_header_unchanged() {
    // A macro-free header still parses exactly as before (regression).
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("Plain.h");
    std::fs::write(&p, "@interface Plain : NSObject\n- (void)go;\n@end\n").unwrap();
    let r = extract_objc(&p);
    let labels: HashSet<&str> = r.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains("Plain"), "{labels:?}");
    assert!(fr_label_edges(&r, "inherits").contains(&("Plain".into(), "NSObject".into())));
}

#[test]
fn objc_alloc_init_emits_type_reference() {
    // `[[Foo alloc] init]` must emit a `references` edge to the project class Foo (#1475).
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("Foo.h"),
        "@interface Foo : NSObject\n@end\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("Foo.m"),
        "#import \"Foo.h\"\n@implementation Foo\n@end\n",
    )
    .unwrap();
    let user = tmp.path().join("User.m");
    std::fs::write(
        &user,
        "#import \"Foo.h\"\n\
         @implementation User\n\
         - (void)build { Foo *x = [[Foo alloc] init]; }\n\
         @end\n",
    )
    .unwrap();
    let r = extract(
        &[tmp.path().join("Foo.h"), tmp.path().join("Foo.m"), user],
        Some(tmp.path()),
    );
    assert!(
        eo_label_edges(&r, "references").contains(&("-build".into(), "Foo".into())),
        "expected -build -> Foo reference; edges: {:?}",
        r.edges
    );
}

#[test]
fn objc_alloc_init_unknown_class_no_resolved_edge() {
    // `[[Unknown alloc] init]` with no such class must not produce a resolved
    // reference edge (the sourceless stub is collapsed only when a real class exists).
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("Caller.m");
    std::fs::write(
        &p,
        "@implementation Caller\n\
         - (void)build { id x = [[Unknown alloc] init]; }\n\
         - (void)other { [self build]; [x doStuff]; }\n\
         @end\n",
    )
    .unwrap();
    let r = extract_objc(&p);
    let sourced_ids: HashSet<&str> = r
        .nodes
        .iter()
        .filter(|n| !n.source_file.is_empty())
        .map(|n| n.id.as_str())
        .collect();
    for e in r.edges.iter().filter(|e| e.relation == "references") {
        assert!(
            !sourced_ids.contains(e.target.as_str()),
            "unexpected resolved ref: {} -> {}",
            e.source,
            e.target
        );
    }
}

#[test]
fn objc_quoted_import_edges_resolve_to_real_nodes() {
    // Quoted `#import "X.h"` edges must target the real (disambiguated) header
    // file node, not the bare stem, which gets salted away when a `.h`/`.m` pair
    // exists and left the import edge dangling (#1475).
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for (name, body) in [
        ("Product.h", "@interface Product : NSObject\n@end\n"),
        (
            "Product.m",
            "#import \"Product.h\"\n@implementation Product\n@end\n",
        ),
        ("Order.h", "@interface Order : NSObject\n@end\n"),
        (
            "Order.m",
            "#import \"Order.h\"\n@implementation Order\n@end\n",
        ),
        (
            "ConsumerA.m",
            "#import \"Product.h\"\n@implementation ConsumerA\n@end\n",
        ),
        (
            "ConsumerB.m",
            "#import \"Order.h\"\n@implementation ConsumerB\n@end\n",
        ),
    ] {
        std::fs::write(root.join(name), body).unwrap();
    }
    let files: Vec<_> = [
        "Product.h",
        "Product.m",
        "Order.h",
        "Order.m",
        "ConsumerA.m",
        "ConsumerB.m",
    ]
    .iter()
    .map(|n| root.join(n))
    .collect();
    let r = extract(&files, Some(root));

    let node_ids: HashSet<&str> = r
        .nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(Value::as_str))
        .collect();
    let id_to_label: HashMap<&str, &str> = r
        .nodes
        .iter()
        .filter_map(|n| Some((n.get("id")?.as_str()?, n.get("label")?.as_str()?)))
        .collect();
    let import_edges: Vec<(&str, &str)> = r
        .edges
        .iter()
        .filter(|e| {
            matches!(
                e.get("relation").and_then(Value::as_str),
                Some("imports" | "imports_from")
            )
        })
        .filter_map(|e| Some((e.get("source")?.as_str()?, e.get("target")?.as_str()?)))
        .collect();
    assert!(!import_edges.is_empty(), "no import edges");
    for (src, tgt) in &import_edges {
        // No dangling targets, no self-loops, and every quoted import targets a
        // header (.h) file node.
        assert!(node_ids.contains(tgt), "dangling import target: {tgt}");
        assert_ne!(src, tgt, "self-loop import edge: {src} -> {tgt}");
        let tgt_label = id_to_label.get(tgt).copied().unwrap_or("");
        assert!(
            tgt_label.ends_with(".h"),
            "import target is not a header node: {tgt} -> {tgt_label}"
        );
    }
    // Product.m -> Product.h specifically lands on the .h variant (not salted
    // back to the importing .m).
    let prod_imports: Vec<_> = import_edges
        .iter()
        .filter(|(s, _)| {
            id_to_label
                .get(s)
                .copied()
                .unwrap_or("")
                .ends_with("Product.m")
        })
        .collect();
    assert!(!prod_imports.is_empty(), "no Product.m import edge");
    assert!(
        prod_imports
            .iter()
            .all(|(_, t)| id_to_label.get(t).copied() == Some("Product.h")),
        "Product.m should import the Product.h node"
    );
}
