//! PHP namespace/`use` type-reference resolution (#1923).
//!
//! Ports `graphify-py/tests/test_php_type_resolution.py`. A bare-name reference
//! to a class that shares its simple name with an internal class from a
//! different namespace must not collapse onto the internal one; it resolves via
//! the referencing file's `namespace` + `use` imports instead.

use std::fs;
use std::path::{Path, PathBuf};

use graphify_extract::extract;
use indexmap::IndexMap;
use serde_json::Value;

type Obj = IndexMap<String, Value>;
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn write(path: &Path, text: &str) -> std::io::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)?;
    Ok(path.to_path_buf())
}

#[must_use]
fn s(m: &Obj, k: &str) -> String {
    m.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn node_by_id<'a>(nodes: &'a [Obj], id: &str) -> Option<&'a Obj> {
    nodes.iter().find(|n| s(n, "id") == id)
}

/// Source-backed class definitions with the given label.
#[must_use]
fn class_defs<'a>(nodes: &'a [Obj], label: &str) -> Vec<&'a Obj> {
    nodes
        .iter()
        .filter(|n| s(n, "label") == label && !s(n, "source_file").is_empty())
        .collect()
}

#[must_use]
fn inherits_from<'a>(edges: &'a [Obj], source_needle: &str) -> Vec<&'a Obj> {
    edges
        .iter()
        .filter(|e| {
            s(e, "relation") == "inherits" && s(e, "source").to_lowercase().contains(source_needle)
        })
        .collect()
}

#[test]
fn php_external_namespaced_base_does_not_collapse_onto_internal_class() -> TestResult {
    // #1923: `App\Models\Page` (internal) and `Filament\Pages\Page` (external,
    // via `use`) share the simple name `Page`. The bare-name rewire must NOT
    // collapse the external supertype reference onto the only internal `Page`.
    let tmp = tempfile::tempdir()?;
    let model = write(
        &tmp.path().join("app/Models/Page.php"),
        "<?php\nnamespace App\\Models;\nclass Page extends Model {}\n",
    )?;
    let page = write(
        &tmp.path().join("app/Filament/Pages/ManageSiteSettings.php"),
        "<?php\nnamespace App\\Filament\\Pages;\n\
         use Filament\\Pages\\Page;\n\
         class ManageSiteSettings extends Page {}\n",
    )?;
    let result = extract(&[model, page], Some(tmp.path()));

    let page_defs = class_defs(&result.nodes, "Page");
    assert_eq!(page_defs.len(), 1, "exactly one internal Page def");
    let internal_page_id = s(page_defs[0], "id");
    assert!(s(page_defs[0], "source_file").contains("Models"));

    let inherits = inherits_from(&result.edges, "managesitesettings");
    assert!(!inherits.is_empty(), "expected an inherits edge");
    for e in &inherits {
        assert_ne!(
            s(e, "target"),
            internal_page_id,
            "inherits wrongly collapsed onto internal App\\Models\\Page (#1923)"
        );
        let tgt = node_by_id(&result.nodes, &s(e, "target")).ok_or("target node missing")?;
        assert!(
            s(tgt, "source_file").is_empty(),
            "external stub is sourceless"
        );
        assert_eq!(s(tgt, "label"), "Filament\\Pages\\Page");
    }

    // The file-level import edge must not target the internal Page either.
    for e in result.edges.iter().filter(|e| {
        s(e, "relation") == "imports"
            && s(e, "source").to_lowercase().contains("managesitesettings")
    }) {
        assert_ne!(s(e, "target"), internal_page_id);
    }
    Ok(())
}

#[test]
fn php_ambiguous_base_disambiguated_by_use() -> TestResult {
    // Two internal same-named `Page` classes in different namespaces; `Editor`
    // lives in a THIRD namespace and only its `use App\Cms\Page` can pick the
    // right one — the current-namespace fallback would resolve to a nonexistent
    // `App\Admin\Page`, so a pass proves the `use` (not an incidental
    // same-namespace match) is what disambiguates.
    let tmp = tempfile::tempdir()?;
    let m = write(
        &tmp.path().join("app/Models/Page.php"),
        "<?php\nnamespace App\\Models;\nclass Page {}\n",
    )?;
    let c = write(
        &tmp.path().join("app/Cms/Page.php"),
        "<?php\nnamespace App\\Cms;\nclass Page {}\n",
    )?;
    let editor = write(
        &tmp.path().join("app/Admin/Editor.php"),
        "<?php\nnamespace App\\Admin;\n\
         use App\\Cms\\Page;\n\
         class Editor extends Page {}\n",
    )?;
    let result = extract(&[m, c, editor], Some(tmp.path()));

    let inherits = inherits_from(&result.edges, "editor");
    assert_eq!(inherits.len(), 1);
    let tgt = node_by_id(&result.nodes, &s(inherits[0], "target")).ok_or("target missing")?;
    let src = s(tgt, "source_file");
    assert_ne!(src, "");
    assert!(src.contains("Cms") && !src.contains("Models") && !src.contains("Admin"));
    Ok(())
}

#[test]
fn php_use_alias_resolves() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let bar = write(
        &tmp.path().join("src/Foo/Bar.php"),
        "<?php\nnamespace Foo;\nclass Bar {}\n",
    )?;
    let x = write(
        &tmp.path().join("src/App/X.php"),
        "<?php\nnamespace App;\n\
         use Foo\\Bar as Baz;\n\
         class X extends Baz {}\n",
    )?;
    let result = extract(&[bar, x], Some(tmp.path()));

    let inherits = inherits_from(&result.edges, "_x");
    assert_ne!(inherits, Vec::<&Obj>::new());
    let tgt = node_by_id(&result.nodes, &s(inherits[0], "target")).ok_or("target missing")?;
    assert_ne!(s(tgt, "source_file"), "");
    assert!(s(tgt, "source_file").contains("Foo"));
    Ok(())
}

#[test]
fn php_fully_qualified_base_resolves() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let page = write(
        &tmp.path().join("app/Models/Page.php"),
        "<?php\nnamespace App\\Models;\nclass Page {}\n",
    )?;
    let y = write(
        &tmp.path().join("app/Http/Y.php"),
        "<?php\nnamespace App\\Http;\n\
         class Y extends \\App\\Models\\Page {}\n",
    )?;
    let result = extract(&[page, y], Some(tmp.path()));

    let inherits = inherits_from(&result.edges, "_y");
    assert_ne!(inherits, Vec::<&Obj>::new());
    let tgt = node_by_id(&result.nodes, &s(inherits[0], "target")).ok_or("target missing")?;
    assert_ne!(s(tgt, "source_file"), "");
    assert!(s(tgt, "source_file").contains("Models"));
    Ok(())
}

#[test]
fn php_plain_no_namespace_inheritance_preserved() -> TestResult {
    // Guards the legacy unique-label rewire path: no namespaces anywhere.
    let tmp = tempfile::tempdir()?;
    let base = write(&tmp.path().join("src/Base.php"), "<?php\nclass Base {}\n")?;
    let child = write(
        &tmp.path().join("src/Child.php"),
        "<?php\nclass Child extends Base {}\n",
    )?;
    let result = extract(&[base, child], Some(tmp.path()));

    let inherits: Vec<&Obj> = result
        .edges
        .iter()
        .filter(|e| s(e, "relation") == "inherits")
        .collect();
    assert_ne!(inherits, Vec::<&Obj>::new());
    let tgt = node_by_id(&result.nodes, &s(inherits[0], "target")).ok_or("target missing")?;
    assert!(
        !s(tgt, "source_file").is_empty(),
        "no-namespace inheritance must resolve to the real Base def"
    );
    assert_eq!(s(tgt, "label"), "Base");
    Ok(())
}

#[test]
fn php_variant_extensions_are_extracted() -> TestResult {
    // #1923: `.phtml`/`.php3-7`/`.phps` are PHP and must produce nodes rather
    // than being silently skipped (graphify-py's extractor dispatch routes only
    // `.php`, leaving the resolver dead code for the variants).
    let tmp = tempfile::tempdir()?;
    let f = write(
        &tmp.path().join("legacy.php3"),
        "<?php\nclass LegacyThing {}\n",
    )?;
    let result = extract(&[f], Some(tmp.path()));
    assert!(
        result
            .nodes
            .iter()
            .any(|n| s(n, "label").contains("LegacyThing")),
        "a .php3 file must be extracted as PHP: {:?}",
        result
            .nodes
            .iter()
            .map(|n| s(n, "label"))
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn php_two_classes_one_file_keep_distinct_supertypes() -> TestResult {
    // #1923: two classes in ONE file extending different absolute FQNs that share
    // a bare name (`\X\Page`, `\Y\Page`) must NOT collapse to one ambiguous
    // target. The raw supertype refs are keyed by owning class, so each inherits
    // edge resolves to its own external stub (without class-scoping the shared
    // `(inherits, page)` key is marked ambiguous and both fall back to the same
    // `App\Page`).
    let tmp = tempfile::tempdir()?;
    let f = write(
        &tmp.path().join("app/Two.php"),
        "<?php\nnamespace App;\n\
         class A extends \\X\\Page {}\n\
         class B extends \\Y\\Page {}\n",
    )?;
    let result = extract(&[f], Some(tmp.path()));

    let id_of = |label: &str| -> Option<String> {
        result
            .nodes
            .iter()
            .find(|n| s(n, "label") == label && !s(n, "source_file").is_empty())
            .map(|n| s(n, "id"))
    };
    let a_id = id_of("A").ok_or("class A def missing")?;
    let b_id = id_of("B").ok_or("class B def missing")?;
    let target_of = |src: &str| -> Option<String> {
        result
            .edges
            .iter()
            .find(|e| s(e, "relation") == "inherits" && s(e, "source") == src)
            .map(|e| s(e, "target"))
    };
    let a_tgt = target_of(&a_id).ok_or("A inherits edge missing")?;
    let b_tgt = target_of(&b_id).ok_or("B inherits edge missing")?;
    assert_ne!(
        a_tgt, b_tgt,
        "A and B must not collapse onto one shared Page stub (#1923)"
    );
    let a_lbl = s(
        node_by_id(&result.nodes, &a_tgt).ok_or("A target node missing")?,
        "label",
    );
    let b_lbl = s(
        node_by_id(&result.nodes, &b_tgt).ok_or("B target node missing")?,
        "label",
    );
    assert_eq!(a_lbl, "X\\Page", "A must inherit the external X\\Page stub");
    assert_eq!(b_lbl, "Y\\Page", "B must inherit the external Y\\Page stub");
    Ok(())
}
