//! Parity tests for C# cross-file type-reference resolution (#1466), ported from
//! `graphify-py/tests/test_csharp_type_resolution.py`.

use std::path::{Path, PathBuf};

use graphify_extract::{ExtractOutput, extract};
use indexmap::IndexMap;
use serde_json::Value;

type Obj = IndexMap<String, Value>;
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn write_file(root: &Path, rel: &str, text: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().ok_or("write_file: rel has no parent")?)?;
    std::fs::write(&p, text)?;
    Ok(p)
}

fn str_field<'a>(n: &'a Obj, key: &str) -> &'a str {
    n.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn node_by_id<'a>(res: &'a ExtractOutput, nid: &str) -> Option<&'a Obj> {
    res.nodes
        .iter()
        .find(|n| n.get("id").and_then(Value::as_str) == Some(nid))
}

/// Nodes that are the target of an edge with `relation` and carry `label`.
fn targets<'a>(res: &'a ExtractOutput, relation: &str, label: &str) -> Vec<&'a Obj> {
    res.edges
        .iter()
        .filter(|e| e.get("relation").and_then(Value::as_str) == Some(relation))
        .filter_map(|e| e.get("target").and_then(Value::as_str))
        .filter_map(|tgt| node_by_id(res, tgt))
        .filter(|n| str_field(n, "label") == label)
        .collect()
}

/// Source-backed definition nodes carrying `label`.
fn defs<'a>(res: &'a ExtractOutput, label: &str) -> Vec<&'a Obj> {
    res.nodes
        .iter()
        .filter(|n| str_field(n, "label") == label && !str_field(n, "source_file").is_empty())
        .collect()
}

#[test]
fn csharp_cross_file_inherits_resolves_to_real_def() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let core = write_file(
        tmp.path(),
        "core.cs",
        "namespace Game.Core { public class Damage { public int Calc() { return 1; } } }\n",
    )?;
    let combat = write_file(
        tmp.path(),
        "combat.cs",
        "using Game.Core;\nnamespace Game.Combat { public class Weapon : Damage {} }\n",
    )?;
    let res = extract(&[core, combat], Some(tmp.path()));

    let damage = targets(&res, "inherits", "Damage");
    assert!(!damage.is_empty(), "expected an inherits edge to Damage");
    assert!(
        damage
            .iter()
            .all(|d| !str_field(d, "source_file").is_empty()),
        "Weapon : Damage must resolve to the real Damage def, not a shadow stub"
    );
    Ok(())
}

#[test]
fn csharp_collision_disambiguated_by_using() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let core = write_file(
        tmp.path(),
        "core.cs",
        "namespace Game.Core { public class WeaponData { public int Number; } }\n",
    )?;
    let ui = write_file(
        tmp.path(),
        "ui.cs",
        "namespace Game.UI { public class WeaponData { public int Width; } }\n",
    )?;
    let combat = write_file(
        tmp.path(),
        "combat.cs",
        "using Game.Core;\nnamespace Game.Combat { public class Holder { public WeaponData data; } }\n",
    )?;
    let res = extract(&[core, ui, combat], Some(tmp.path()));

    let shadow: Vec<_> = res
        .nodes
        .iter()
        .filter(|n| str_field(n, "label") == "WeaponData" && str_field(n, "source_file").is_empty())
        .collect();
    assert!(shadow.is_empty(), "orphan WeaponData shadow node(s) remain");

    let resolved: Vec<_> = targets(&res, "references", "WeaponData")
        .into_iter()
        .filter(|w| !str_field(w, "source_file").is_empty())
        .collect();
    assert!(
        !resolved.is_empty(),
        "WeaponData reference should resolve to a real def"
    );
    assert!(
        resolved
            .iter()
            .all(|w| str_field(w, "source_file").contains("core.cs")),
        "must disambiguate to Game.Core.WeaponData via `using Game.Core;`, not Game.UI"
    );
    Ok(())
}

#[test]
fn csharp_global_using_and_global_namespace() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let gadget = write_file(tmp.path(), "gadget.cs", "public class Gadget {}\n")?;
    let user = write_file(
        tmp.path(),
        "user.cs",
        "global using System;\npublic class Widget : Gadget {}\n",
    )?;
    let res = extract(&[gadget, user], Some(tmp.path()));

    let g = targets(&res, "inherits", "Gadget");
    assert!(!g.is_empty(), "expected an inherits edge to Gadget");
    assert!(
        g.iter().all(|x| !str_field(x, "source_file").is_empty()),
        "Widget : Gadget (both global namespace) must resolve; `global using` must not break parsing"
    );
    Ok(())
}

#[test]
fn csharp_cross_namespace_enum_reference_resolves_to_real_def() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let core = write_file(
        tmp.path(),
        "core.cs",
        "namespace Game.Core { public enum Element { Fire, Ice } public class Damage {} }\n",
    )?;
    let combat = write_file(
        tmp.path(),
        "combat.cs",
        "using Game.Core;\nnamespace Game.Combat { public class Spell { Element element; Damage dmg; } }\n",
    )?;
    let res = extract(&[core, combat], Some(tmp.path()));

    let defs_found = defs(&res, "Element");
    assert!(
        !defs_found.is_empty(),
        "enum Element should be a real type def node"
    );
    assert!(
        defs_found
            .iter()
            .all(|n| str_field(n, "source_file").contains("core.cs"))
    );

    let resolved: Vec<_> = targets(&res, "references", "Element")
        .into_iter()
        .filter(|n| !str_field(n, "source_file").is_empty())
        .collect();
    assert!(
        !resolved.is_empty(),
        "Element field reference should resolve to the enum def"
    );
    assert!(
        resolved
            .iter()
            .all(|n| str_field(n, "source_file").contains("core.cs"))
    );
    Ok(())
}

#[test]
fn csharp_cross_namespace_struct_and_record_references_resolve() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let core = write_file(
        tmp.path(),
        "core.cs",
        "namespace Game.Core { public struct Coord { public int X; } public record Player(string Name); }\n",
    )?;
    let combat = write_file(
        tmp.path(),
        "combat.cs",
        "using Game.Core;\nnamespace Game.Combat { public class Spell { Coord coord; Player player; } }\n",
    )?;
    let res = extract(&[core, combat], Some(tmp.path()));

    for label in ["Coord", "Player"] {
        assert!(
            !defs(&res, label).is_empty(),
            "{label} should be a real type def node"
        );
        let resolved: Vec<_> = targets(&res, "references", label)
            .into_iter()
            .filter(|n| !str_field(n, "source_file").is_empty())
            .collect();
        assert!(
            !resolved.is_empty(),
            "{label} field reference should resolve to the real def"
        );
        assert!(
            resolved
                .iter()
                .all(|n| str_field(n, "source_file").contains("core.cs"))
        );
    }
    Ok(())
}

#[test]
fn csharp_ambiguous_using_does_not_resolve() -> TestResult {
    // WeaponData is defined in BOTH Game.Core and Game.UI, and the referrer opens
    // BOTH namespaces. With two candidates the resolver must REFUSE (accept only a
    // unique hit) and leave the reference dangling, not fabricate a wrong edge.
    let tmp = tempfile::tempdir()?;
    let core = write_file(
        tmp.path(),
        "core.cs",
        "namespace Game.Core { public class WeaponData { public int Number; } }\n",
    )?;
    let ui = write_file(
        tmp.path(),
        "ui.cs",
        "namespace Game.UI { public class WeaponData { public int Width; } }\n",
    )?;
    let holder = write_file(
        tmp.path(),
        "holder.cs",
        "using Game.Core;\nusing Game.UI;\nnamespace Game.Combat { public class Holder { public WeaponData data; } }\n",
    )?;
    let res = extract(&[core, ui, holder], Some(tmp.path()));

    let wd_refs = targets(&res, "references", "WeaponData");
    assert!(!wd_refs.is_empty(), "expected a WeaponData reference edge");
    let resolved: Vec<_> = wd_refs
        .iter()
        .filter(|n| !str_field(n, "source_file").is_empty())
        .collect();
    assert!(
        resolved.is_empty(),
        "ambiguous WeaponData (Game.Core vs Game.UI, both opened) must NOT resolve to either def"
    );
    Ok(())
}

#[test]
fn csharp_using_alias_resolves_to_aliased_type() -> TestResult {
    // `using Dmg = Game.Core.Damage;` is a single-type alias; a base type written
    // as `Dmg` must resolve to the real Game.Core.Damage def via the alias map.
    let tmp = tempfile::tempdir()?;
    let core = write_file(
        tmp.path(),
        "core.cs",
        "namespace Game.Core { public class Damage {} }\n",
    )?;
    let combat = write_file(
        tmp.path(),
        "combat.cs",
        "using Dmg = Game.Core.Damage;\nnamespace Game.Combat { public class Weapon : Dmg {} }\n",
    )?;
    let res = extract(&[core, combat], Some(tmp.path()));

    let damage = targets(&res, "inherits", "Damage");
    assert!(
        !damage.is_empty(),
        "Weapon : Dmg must resolve via the alias to Damage"
    );
    assert!(
        damage
            .iter()
            .all(|d| str_field(d, "source_file").contains("core.cs")),
        "the alias `Dmg` must resolve to the real Game.Core.Damage def, not a shadow stub"
    );
    Ok(())
}

/// b9d8067 (#1562): C# type-definition nodes carry the enclosing namespace (and a
/// lexical `scope_chain`) as metadata — block, nested, and file-scoped forms.
#[test]
fn csharp_declaration_nodes_carry_enclosing_namespace() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let block = write_file(
        tmp.path(),
        "block.cs",
        "namespace Game.Core { public class Damage {} }\n",
    )?;
    let nested = write_file(
        tmp.path(),
        "nested.cs",
        "namespace Outer { namespace Inner { public class NestedDamage {} } }\n",
    )?;
    let file_scoped = write_file(
        tmp.path(),
        "file_scoped.cs",
        "namespace FileScoped.Core;\npublic class FileScopedDamage {}\n",
    )?;
    let result = extract(&[block, nested, file_scoped], Some(tmp.path()));
    let ns_of = |label: &str| -> Option<String> {
        defs(&result, label)
            .first()
            .and_then(|n| n.get("metadata"))
            .and_then(|m| m.get("namespace"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    assert_eq!(ns_of("Damage").as_deref(), Some("Game.Core"));
    assert_eq!(ns_of("NestedDamage").as_deref(), Some("Outer.Inner"));
    assert_eq!(
        ns_of("FileScopedDamage").as_deref(),
        Some("FileScoped.Core")
    );
    let damage = defs(&result, "Damage");
    let scope_chain = damage
        .first()
        .and_then(|n| n.get("metadata"))
        .and_then(|m| m.get("scope_chain"))
        .and_then(Value::as_array);
    assert!(
        scope_chain.is_some_and(|a| !a.is_empty()),
        "lexical scope_chain must be stamped: {damage:?}"
    );
    Ok(())
}

/// b9d8067 (#1562): namespace `N` is one canonical node shared across files
/// (digest id), nested namespaces are discriminated, all ids `csharp_namespace:`.
#[test]
fn csharp_namespace_nodes_canonical_and_discriminated() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let a = write_file(tmp.path(), "a.cs", "namespace N { class A {} }\n")?;
    let b = write_file(tmp.path(), "b.cs", "namespace N { class B {} }\n")?;
    let nested = write_file(
        tmp.path(),
        "n.cs",
        "namespace Outer { namespace Inner { class C {} } }\n",
    )?;
    let result = extract(&[a, b, nested], Some(tmp.path()));
    let ns: Vec<&Obj> = result
        .nodes
        .iter()
        .filter(|n| n.get("type").and_then(Value::as_str) == Some("namespace"))
        .collect();
    let mut by_label: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for n in &ns {
        *by_label.entry(str_field(n, "label")).or_default() += 1;
    }
    assert_eq!(
        by_label.get("N").copied(),
        Some(1),
        "namespace N must be one canonical node across files: {by_label:?}"
    );
    assert!(
        by_label.contains_key("Outer.Inner"),
        "nested namespace present: {by_label:?}"
    );
    assert!(
        ns.iter()
            .all(|n| str_field(n, "id").starts_with("csharp_namespace:")),
        "all namespace ids must be digest-prefixed"
    );
    // The canonical survivor must be deterministic regardless of input order:
    // reversing the file list keeps the same `N` node (same id + source_file).
    let ns_source = |res: &ExtractOutput| -> Option<String> {
        res.nodes
            .iter()
            .find(|n| str_field(n, "label") == "N")
            .map(|n| format!("{}|{}", str_field(n, "id"), str_field(n, "source_file")))
    };
    let a2 = write_file(tmp.path(), "a.cs", "namespace N { class A {} }\n")?;
    let b2 = write_file(tmp.path(), "b.cs", "namespace N { class B {} }\n")?;
    let n2 = write_file(
        tmp.path(),
        "n.cs",
        "namespace Outer { namespace Inner { class C {} } }\n",
    )?;
    let reversed = extract(&[n2, b2, a2], Some(tmp.path()));
    assert_eq!(
        ns_source(&result),
        ns_source(&reversed),
        "canonical namespace survivor must be input-order-independent"
    );
    Ok(())
}

/// b9d8067 (#1562): C# `using` import edges carry `using_kind` / `target_fqn` /
/// `alias` metadata for namespace, `static`, `global using`, and alias forms.
#[test]
fn csharp_import_edges_carry_using_kind() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let f = write_file(
        tmp.path(),
        "a.cs",
        "using Game.Core;\nusing static System.Math;\nglobal using System;\nusing X = Game.Core.Damage;\nclass Z {}\n",
    )?;
    let result = extract(&[f], Some(tmp.path()));
    let imports: std::collections::HashSet<(String, String, Option<String>)> = result
        .edges
        .iter()
        .filter(|e| e.get("relation").and_then(Value::as_str) == Some("imports"))
        .filter_map(|e| {
            e.get("metadata").and_then(Value::as_object).map(|m| {
                (
                    m.get("using_kind")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    m.get("target_fqn")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    m.get("alias").and_then(Value::as_str).map(str::to_string),
                )
            })
        })
        .collect();
    assert!(
        imports.contains(&("namespace".into(), "Game.Core".into(), None)),
        "{imports:?}"
    );
    assert!(
        imports.contains(&("namespace".into(), "System".into(), None)),
        "{imports:?}"
    );
    assert!(
        imports.contains(&("static".into(), "System.Math".into(), None)),
        "{imports:?}"
    );
    assert!(
        imports.contains(&("alias".into(), "Game.Core.Damage".into(), Some("X".into()))),
        "{imports:?}"
    );
    Ok(())
}

/// b9d8067 (#1562): a `using` inside a namespace block carries `scope_kind`
/// = "namespace" + a lexical `scope_id`, and targets `make_id1(target_fqn)`.
/// Exercises the scope plumbing the resolver consumes (file-scope is covered by
/// `csharp_import_edges_carry_using_kind`).
#[test]
fn csharp_namespace_scoped_using_carries_scope() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let f = write_file(
        tmp.path(),
        "a.cs",
        "namespace N {\n    using Game.Core;\n    class Z {}\n}\n",
    )?;
    let result = extract(&[f], Some(tmp.path()));
    let edge = result
        .edges
        .iter()
        .find(|e| {
            e.get("relation").and_then(Value::as_str) == Some("imports")
                && e.get("metadata")
                    .and_then(Value::as_object)
                    .and_then(|m| m.get("target_fqn"))
                    .and_then(Value::as_str)
                    == Some("Game.Core")
        })
        .ok_or("expected a Game.Core using import edge")?;
    let m = edge
        .get("metadata")
        .and_then(Value::as_object)
        .ok_or("import edge must carry metadata")?;
    assert_eq!(
        m.get("scope_kind").and_then(Value::as_str),
        Some("namespace")
    );
    assert!(
        m.get("scope_id")
            .and_then(Value::as_str)
            .is_some_and(|s| s.starts_with('s')),
        "namespace-scoped using must carry a scope_id: {m:?}"
    );
    assert_eq!(
        str_field(edge, "target"),
        graphify_extract::make_id(&["Game.Core"])
    );
    Ok(())
}

/// b9d8067 (#1562): a qualified base reference (`B.T`) carries `metadata.qualified`.
#[test]
fn csharp_qualified_base_ref_is_flagged() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let f = write_file(
        tmp.path(),
        "a.cs",
        "namespace N { class T {} class Use : B.T {} }\n",
    )?;
    let result = extract(&[f], Some(tmp.path()));
    assert!(
        result.edges.iter().any(|e| e
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|m| m.get("qualified"))
            .and_then(Value::as_bool)
            == Some(true)),
        "the qualified base ref B.T must carry metadata.qualified"
    );
    Ok(())
}

/// b9d8067 (#1562): a C# `inherits` edge carries `metadata.ref_token` (the
/// referenced simple name), used by the resolver to bind the base type.
#[test]
fn csharp_type_ref_edges_carry_ref_token() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let core = write_file(tmp.path(), "core.cs", "namespace N { class Base {} }\n")?;
    let use_f = write_file(
        tmp.path(),
        "use.cs",
        "using N;\nnamespace M { class Use : Base {} }\n",
    )?;
    let result = extract(&[core, use_f], Some(tmp.path()));
    let inh: Vec<&Obj> = result
        .edges
        .iter()
        .filter(|e| {
            e.get("relation").and_then(Value::as_str) == Some("inherits")
                && str_field(e, "source").to_lowercase().contains("use")
        })
        .collect();
    assert!(!inh.is_empty(), "expected the Use : Base inherits edge");
    assert!(
        inh.iter().any(|e| e
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|m| m.get("ref_token"))
            .and_then(Value::as_str)
            == Some("Base")),
        "the inherits edge must carry metadata.ref_token == 'Base'"
    );
    Ok(())
}

/// b9d8067 (#1562): a use of an in-scope type parameter (`class Box<T> { T value; }`)
/// must NOT produce a reference to a same-named real type.
#[test]
fn csharp_type_parameter_emits_no_reference() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let f = write_file(
        tmp.path(),
        "a.cs",
        "namespace N { class T {} class Box<T> { T value; } }\n",
    )?;
    let result = extract(&[f], Some(tmp.path()));
    let real_t: std::collections::HashSet<&str> = result
        .nodes
        .iter()
        .filter(|n| str_field(n, "label") == "T" && !str_field(n, "source_file").is_empty())
        .filter_map(|n| n.get("id").and_then(Value::as_str))
        .collect();
    let box_to_t = result.edges.iter().any(|e| {
        matches!(
            e.get("relation").and_then(Value::as_str),
            Some("references" | "inherits" | "implements")
        ) && e
            .get("target")
            .and_then(Value::as_str)
            .is_some_and(|t| real_t.contains(t))
            && str_field(e, "source").to_lowercase().contains("box")
    });
    assert!(
        !box_to_t,
        "type parameter T must not produce a ref to the real N.T"
    );
    Ok(())
}

// ── b9d8067 (#1562/#1552): metadata-driven resolver ports ─────────────────────

/// First node with `label`.
fn find_node<'a>(res: &'a ExtractOutput, label: &str) -> Option<&'a Obj> {
    res.nodes.iter().find(|n| str_field(n, "label") == label)
}

/// First node with `label` whose `metadata.namespace` == `ns`.
fn node_in_ns<'a>(res: &'a ExtractOutput, label: &str, ns: &str) -> Option<&'a Obj> {
    res.nodes.iter().find(|n| {
        str_field(n, "label") == label
            && n.get("metadata")
                .and_then(|m| m.get("namespace"))
                .and_then(Value::as_str)
                == Some(ns)
    })
}

/// First source-backed node with `label`.
fn def1<'a>(res: &'a ExtractOutput, label: &str) -> Option<&'a Obj> {
    res.nodes
        .iter()
        .find(|n| str_field(n, "label") == label && !str_field(n, "source_file").is_empty())
}

/// `(source, target)` id pairs for edges with `relation`.
fn rel_pairs(res: &ExtractOutput, relation: &str) -> std::collections::HashSet<(String, String)> {
    res.edges
        .iter()
        .filter(|e| str_field(e, "relation") == relation)
        .map(|e| {
            (
                str_field(e, "source").to_string(),
                str_field(e, "target").to_string(),
            )
        })
        .collect()
}

/// A node's boolean metadata flag (missing → false).
fn meta_bool(n: &Obj, key: &str) -> bool {
    n.get("metadata")
        .and_then(|m| m.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// b9d8067 (#1552): internal `using N;` / alias imports re-point to the real
/// namespace / type node; external / static / unknown-prefix imports stay put.
#[test]
fn csharp_import_edges_resolve_internal_namespace_and_alias() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let core = write_file(
        tmp.path(),
        "core.cs",
        "namespace Game.Core { public class Damage {} }\n",
    )?;
    let user = write_file(
        tmp.path(),
        "u.cs",
        "using Game.Core;\nusing UnityEngine;\nusing Dmg = Game.Core.Damage;\nusing DMath = System.Math;\nusing static Game.Core.Damage;\nclass Z {}\n",
    )?;
    let res = extract(&[core, user], Some(tmp.path()));
    let imports: Vec<(&str, &str, Option<&Obj>)> = res
        .edges
        .iter()
        .filter(|e| str_field(e, "relation") == "imports")
        .filter_map(|e| {
            let m = e.get("metadata").and_then(Value::as_object)?;
            let kind = m.get("using_kind").and_then(Value::as_str)?;
            let fqn = m
                .get("target_fqn")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Some((kind, fqn, node_by_id(&res, str_field(e, "target"))))
        })
        .collect();
    assert!(
        imports.iter().any(|(k, f, t)| *k == "namespace"
            && *f == "Game.Core"
            && opt_field(*t, "type") == Some("namespace")),
        "internal namespace import must resolve to the namespace node: {imports:?}"
    );
    assert!(
        imports.iter().any(|(k, f, t)| *k == "namespace"
            && *f == "UnityEngine"
            && opt_field(*t, "type").is_none()),
        "external namespace import must stay unresolved: {imports:?}"
    );
    assert!(
        imports.iter().any(|(k, f, t)| *k == "alias"
            && *f == "Game.Core.Damage"
            && opt_field(*t, "label") == Some("Damage")),
        "internal alias import must resolve to the aliased type: {imports:?}"
    );
    assert!(
        imports.iter().any(|(k, f, t)| *k == "alias"
            && *f == "System.Math"
            && opt_field(*t, "label").is_none()),
        "external alias import must stay unresolved: {imports:?}"
    );
    assert!(
        imports.iter().any(|(k, f, t)| *k == "static"
            && *f == "Game.Core.Damage"
            && opt_field(*t, "label").is_none()),
        "static import must stay unresolved: {imports:?}"
    );
    assert!(
        !res.nodes
            .iter()
            .any(|n| str_field(n, "source_file").is_empty()
                && matches!(str_field(n, "label"), "Game.Core" | "Game.Core.Damage")),
        "no sourceless Game.Core / Game.Core.Damage shadow node should remain"
    );
    Ok(())
}

/// b9d8067 (#1562): `ns_collision` metadata is gone — same-named types in two
/// namespaces of one file are distinct nodes with no collision flag.
#[test]
fn csharp_one_file_same_name_no_collision_flag() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let dup = write_file(
        tmp.path(),
        "dup.cs",
        "namespace A { class T {} } namespace B { class T {} }\n",
    )?;
    let res = extract(&[dup], Some(tmp.path()));
    let tnodes: Vec<&Obj> = res
        .nodes
        .iter()
        .filter(|n| str_field(n, "label") == "T" && !str_field(n, "source_file").is_empty())
        .collect();
    let ids: std::collections::HashSet<&str> = tnodes.iter().map(|n| str_field(n, "id")).collect();
    assert_eq!(
        ids.len(),
        2,
        "A.T and B.T must be distinct nodes: {tnodes:?}"
    );
    assert!(
        !tnodes.iter().any(|n| meta_bool(n, "ns_collision")),
        "ns_collision must no longer be stamped"
    );
    Ok(())
}

/// b9d8067 (#1562): a nested type carries `is_nested_type` metadata.
#[test]
fn csharp_nested_type_carries_metadata() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let f = write_file(
        tmp.path(),
        "a.cs",
        "namespace N { class Outer { class Inner {} } }\n",
    )?;
    let res = extract(&[f], Some(tmp.path()));
    let inner = find_node(&res, "Inner").ok_or("Inner node missing")?;
    assert!(
        meta_bool(inner, "is_nested_type"),
        "Inner must carry is_nested_type: {inner:?}"
    );
    Ok(())
}

/// b9d8067 (#1562): a bare ref in namespace B must NOT bind C.T (B never opens C),
/// even though T is globally unique.
#[test]
fn csharp_cross_namespace_ref_not_misbound() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let f = write_file(
        tmp.path(),
        "x.cs",
        "namespace B { class Use : T {} } namespace C { class T {} }\n",
    )?;
    let res = extract(&[f], Some(tmp.path()));
    let resolved: Vec<_> = targets(&res, "inherits", "T")
        .into_iter()
        .filter(|t| !str_field(t, "source_file").is_empty())
        .collect();
    assert!(
        resolved.is_empty(),
        "Use:T in B must not bind C.T: {resolved:?}"
    );
    Ok(())
}

/// b9d8067 (#1562): same file, T in B and `Use : T` in C — must NOT bind B.T
/// (the eager same-file binding case).
#[test]
fn csharp_same_file_cross_namespace_ref_not_misbound() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let f = write_file(
        tmp.path(),
        "x.cs",
        "namespace B { class T {} } namespace C { class Use : T {} }\n",
    )?;
    let res = extract(&[f], Some(tmp.path()));
    let resolved: Vec<_> = targets(&res, "inherits", "T")
        .into_iter()
        .filter(|t| !str_field(t, "source_file").is_empty())
        .collect();
    assert!(
        resolved.is_empty(),
        "same-file Use:T in C must not bind B.T: {resolved:?}"
    );
    Ok(())
}

/// b9d8067 (#1562): `class Use : Game` where `Game` is a namespace must NOT bind
/// the namespace node.
#[test]
fn csharp_inherits_does_not_bind_namespace_node() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let f = write_file(
        tmp.path(),
        "y.cs",
        "namespace Game { class Damage {} class Use : Game {} }\n",
    )?;
    let res = extract(&[f], Some(tmp.path()));
    let nsids: std::collections::HashSet<&str> = res
        .nodes
        .iter()
        .filter(|n| str_field(n, "type") == "namespace")
        .map(|n| str_field(n, "id"))
        .collect();
    assert!(
        !res.edges
            .iter()
            .any(|e| str_field(e, "relation") == "inherits"
                && nsids.contains(str_field(e, "target"))),
        "inherits must not target a namespace node"
    );
    Ok(())
}

/// b9d8067 (#1562): `B.T` where `B` is neither a known namespace nor an alias
/// must NOT bind A.T (sound dangle).
#[test]
fn csharp_qualified_ref_unknown_qualifier_dangles() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let f = write_file(
        tmp.path(),
        "a.cs",
        "namespace A { class T {} class Use : B.T {} }\n",
    )?;
    let res = extract(&[f], Some(tmp.path()));
    let resolved: Vec<_> = targets(&res, "inherits", "T")
        .into_iter()
        .filter(|t| !str_field(t, "source_file").is_empty())
        .collect();
    assert!(
        resolved.is_empty(),
        "unknown-qualifier B.T must not bind A.T: {resolved:?}"
    );
    Ok(())
}

/// b9d8067 (#1562): `M.Use : N.T` resolves via the known namespace `N`.
#[test]
fn csharp_qualified_ref_known_namespace_resolves() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let a = write_file(tmp.path(), "n.cs", "namespace N { class T {} }\n")?;
    let b = write_file(tmp.path(), "m.cs", "namespace M { class Use : N.T {} }\n")?;
    let res = extract(&[a, b], Some(tmp.path()));
    let n_t = def1(&res, "T").ok_or("N.T def missing")?;
    let use_n = find_node(&res, "Use").ok_or("Use node missing")?;
    let inh = rel_pairs(&res, "inherits");
    assert!(
        inh.contains(&(
            str_field(use_n, "id").to_string(),
            str_field(n_t, "id").to_string()
        )),
        "M.Use : N.T must bind N.T"
    );
    Ok(())
}

/// b9d8067 (#1562): `N.Box<int>` resolves to the real `N.Box` def with no junk
/// generic-label node.
#[test]
fn csharp_qualified_generic_resolves_to_real_def() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let f = write_file(
        tmp.path(),
        "g.cs",
        "namespace N { class Box<TI> {} class Use { N.Box<int> b; } }\n",
    )?;
    let res = extract(&[f], Some(tmp.path()));
    let boxn = def1(&res, "Box").ok_or("N.Box def missing")?;
    let use_n = find_node(&res, "Use").ok_or("Use node missing")?;
    let ref_pairs = rel_pairs(&res, "references");
    assert!(
        ref_pairs.contains(&(
            str_field(use_n, "id").to_string(),
            str_field(boxn, "id").to_string()
        )),
        "N.Box<int> field must resolve to the real N.Box def"
    );
    assert!(
        !res.nodes
            .iter()
            .any(|n| str_field(n, "label").contains('<')),
        "no node should carry a junk generic label"
    );
    Ok(())
}

/// b9d8067 (#1562): `using B = X.Y;` then `B.T` resolves the type `T` in `X.Y`.
#[test]
fn csharp_qualified_alias_namespace_resolves() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let a = write_file(tmp.path(), "n.cs", "namespace X.Y { class T {} }\n")?;
    let b = write_file(
        tmp.path(),
        "m.cs",
        "using B = X.Y;\nnamespace M { class Use : B.T {} }\n",
    )?;
    let res = extract(&[a, b], Some(tmp.path()));
    let t = def1(&res, "T").ok_or("X.Y.T def missing")?;
    let use_n = find_node(&res, "Use").ok_or("Use node missing")?;
    let inh = rel_pairs(&res, "inherits");
    assert!(
        inh.contains(&(
            str_field(use_n, "id").to_string(),
            str_field(t, "id").to_string()
        )),
        "B.T with `using B = X.Y;` must resolve to X.Y.T"
    );
    Ok(())
}

/// b9d8067 (#1562): an out-of-scope alias `B` falls through to the real namespace
/// `B` (declared elsewhere, used where the alias is not in scope).
#[test]
fn csharp_qualified_out_of_scope_alias_falls_through_to_namespace() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let a = write_file(tmp.path(), "b.cs", "namespace B { class T {} }\n")?;
    let c = write_file(
        tmp.path(),
        "m.cs",
        "namespace A { using B = X.Y; }\nnamespace M { class Use : B.T {} }\n",
    )?;
    let res = extract(&[a, c], Some(tmp.path()));
    let b_t = def1(&res, "T").ok_or("B.T def missing")?;
    let use_n = find_node(&res, "Use").ok_or("Use node missing")?;
    let inh = rel_pairs(&res, "inherits");
    assert!(
        inh.contains(&(
            str_field(use_n, "id").to_string(),
            str_field(b_t, "id").to_string()
        )),
        "out-of-scope alias B must fall through to namespace B"
    );
    Ok(())
}

/// b9d8067 (#1562): an in-scope alias `B = X.Y` shadows the same-named namespace
/// `B`; a later out-of-scope alias must not overwrite it.
#[test]
fn csharp_qualified_in_scope_alias_shadows_namespace() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let a = write_file(tmp.path(), "xy.cs", "namespace X.Y { class T {} }\n")?;
    let b = write_file(tmp.path(), "b.cs", "namespace B { class T {} }\n")?;
    let c = write_file(
        tmp.path(),
        "use.cs",
        "namespace A { using B = X.Y; class Good : B.T {} }\nnamespace C { using B = Z.Q; }\n",
    )?;
    let res = extract(&[a, b, c], Some(tmp.path()));
    let xy_t = node_in_ns(&res, "T", "X.Y").ok_or("X.Y.T missing")?;
    let b_t = node_in_ns(&res, "T", "B").ok_or("B.T missing")?;
    let good = find_node(&res, "Good").ok_or("Good node missing")?;
    let inh = rel_pairs(&res, "inherits");
    assert!(
        inh.contains(&(
            str_field(good, "id").to_string(),
            str_field(xy_t, "id").to_string()
        )),
        "in-scope alias B=X.Y must resolve B.T to X.Y.T"
    );
    assert!(
        !inh.contains(&(
            str_field(good, "id").to_string(),
            str_field(b_t, "id").to_string()
        )),
        "must NOT bind namespace B's T"
    );
    Ok(())
}

/// b9d8067 (#1562): T in both A and B of one file; `Use : T` in B binds B.T (its
/// own namespace), not A.T.
#[test]
fn csharp_one_file_same_name_binds_own_namespace() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let f = write_file(
        tmp.path(),
        "c.cs",
        "namespace A { class T {} } namespace B { class T {} class Use : T {} }\n",
    )?;
    let res = extract(&[f], Some(tmp.path()));
    let b_t = node_in_ns(&res, "T", "B").ok_or("B.T missing")?;
    let a_t = node_in_ns(&res, "T", "A").ok_or("A.T missing")?;
    let use_n = find_node(&res, "Use").ok_or("Use node missing")?;
    let inh = rel_pairs(&res, "inherits");
    assert!(
        inh.contains(&(
            str_field(use_n, "id").to_string(),
            str_field(b_t, "id").to_string()
        )),
        "Use:T in B must bind B.T"
    );
    assert!(
        !inh.contains(&(
            str_field(use_n, "id").to_string(),
            str_field(a_t, "id").to_string()
        )),
        "Use:T must NOT bind A.T"
    );
    Ok(())
}

/// b9d8067 (#1562): a nested type is not importable via `using N;` as a bare member.
#[test]
fn csharp_nested_type_not_importable_via_using() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let a = write_file(
        tmp.path(),
        "a.cs",
        "namespace N { class Outer { class Inner {} } }\n",
    )?;
    let b = write_file(
        tmp.path(),
        "b.cs",
        "using N;\nnamespace M { class Use { Inner x; } }\n",
    )?;
    let res = extract(&[a, b], Some(tmp.path()));
    let resolved: Vec<_> = targets(&res, "references", "Inner")
        .into_iter()
        .filter(|t| !str_field(t, "source_file").is_empty())
        .collect();
    assert!(
        resolved.is_empty(),
        "nested Inner must not resolve via `using N;`: {resolved:?}"
    );
    Ok(())
}

/// b9d8067 (#1562): a generic alias `using Bx = N.Box<int>;` resolves to the real
/// `N.Box` def.
#[test]
fn csharp_generic_alias_resolves_to_base_type() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let core = write_file(tmp.path(), "core.cs", "namespace N { class Box {} }\n")?;
    let use_f = write_file(
        tmp.path(),
        "use.cs",
        "using Bx = N.Box<int>;\nclass Use : Bx {}\n",
    )?;
    let res = extract(&[core, use_f], Some(tmp.path()));
    let resolved: Vec<_> = targets(&res, "inherits", "Box")
        .into_iter()
        .filter(|t| !str_field(t, "source_file").is_empty())
        .collect();
    assert!(
        !resolved.is_empty(),
        "generic alias `using Bx = N.Box<int>;` must resolve to the real Box def"
    );
    Ok(())
}

/// b9d8067 (#1562): a C# type reference must never target a `.cs` file-labeled node.
#[test]
fn csharp_type_ref_never_targets_a_file_label() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let core = write_file(tmp.path(), "core.cs", "namespace N { class Box {} }\n")?;
    let b = write_file(tmp.path(), "b.cs", "using B = N.Box;\nclass Use : B {}\n")?;
    let res = extract(&[core, b], Some(tmp.path()));
    let bad = res.edges.iter().any(|e| {
        matches!(
            str_field(e, "relation"),
            "inherits" | "implements" | "references"
        ) && node_by_id(&res, str_field(e, "target"))
            .is_some_and(|n| str_field(n, "label").strip_suffix(".cs").is_some())
    });
    assert!(
        !bad,
        "a C# type ref must not target a .cs file-labeled node"
    );
    Ok(())
}

/// b9d8067 (#1562): an alias whose name equals the file stem still resolves via
/// the ref token (previously corrupted the target label).
#[test]
fn csharp_alias_matching_file_stem_resolves_via_token() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let core = write_file(tmp.path(), "core.cs", "namespace N { class Box {} }\n")?;
    let b = write_file(tmp.path(), "b.cs", "using B = N.Box;\nclass Use : B {}\n")?;
    let res = extract(&[core, b], Some(tmp.path()));
    let resolved: Vec<_> = targets(&res, "inherits", "Box")
        .into_iter()
        .filter(|t| !str_field(t, "source_file").is_empty())
        .collect();
    assert!(
        !resolved.is_empty(),
        "Use : B (alias B == file stem) must resolve to the real Box def"
    );
    Ok(())
}

/// b9d8067 (#1562): same-named types in different namespaces have distinct,
/// namespace-carrying ids.
#[test]
fn csharp_same_name_diff_namespace_have_distinct_ids() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let f = write_file(
        tmp.path(),
        "x.cs",
        "namespace A { class T {} } namespace B { class T {} }\n",
    )?;
    let res = extract(&[f], Some(tmp.path()));
    let ids: std::collections::HashSet<&str> = res
        .nodes
        .iter()
        .filter(|n| str_field(n, "label") == "T" && !str_field(n, "source_file").is_empty())
        .map(|n| str_field(n, "id"))
        .collect();
    assert_eq!(ids.len(), 2, "A.T and B.T must be distinct nodes: {ids:?}");
    Ok(())
}

/// b9d8067 (#1562): a global-scope (no namespace) C# type keeps the bare
/// stem+name id and carries no `namespace` metadata.
#[test]
fn csharp_global_scope_id_unchanged() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let f = write_file(tmp.path(), "g.cs", "class Glob {}\n")?;
    let res = extract(&[f], Some(tmp.path()));
    let glob = find_node(&res, "Glob").ok_or("Glob node missing")?;
    assert_eq!(
        str_field(glob, "id"),
        graphify_extract::make_id(&["g", "Glob"])
    );
    assert!(
        glob.get("metadata")
            .and_then(|m| m.get("namespace"))
            .is_none(),
        "global-scope type must not carry namespace metadata: {glob:?}"
    );
    Ok(())
}

/// b9d8067 (#1562): a namespaced C# type id carries the namespace segment.
#[test]
fn csharp_namespaced_id_carries_namespace_segment() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let f = write_file(
        tmp.path(),
        "n.cs",
        "namespace Game.Core { class Order {} }\n",
    )?;
    let res = extract(&[f], Some(tmp.path()));
    let order = find_node(&res, "Order").ok_or("Order node missing")?;
    let id = str_field(order, "id");
    assert!(
        id.ends_with("order") && id.contains("game_core"),
        "namespaced id must carry the namespace segment: {id}"
    );
    assert_eq!(
        order
            .get("metadata")
            .and_then(|m| m.get("namespace"))
            .and_then(Value::as_str),
        Some("Game.Core")
    );
    Ok(())
}

/// b9d8067 (#1562): two namespaces in one file each resolve their own `T`.
#[test]
fn csharp_two_namespaces_each_resolve_own_type() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let f = write_file(
        tmp.path(),
        "two.cs",
        "namespace A { class T {} class UseA : T {} } namespace B { class T {} class UseB : T {} }\n",
    )?;
    let res = extract(&[f], Some(tmp.path()));
    let a_t = node_in_ns(&res, "T", "A").ok_or("A.T missing")?;
    let b_t = node_in_ns(&res, "T", "B").ok_or("B.T missing")?;
    let use_a = node_in_ns(&res, "UseA", "A").ok_or("UseA missing")?;
    let use_b = node_in_ns(&res, "UseB", "B").ok_or("UseB missing")?;
    let inh = rel_pairs(&res, "inherits");
    assert!(
        inh.contains(&(
            str_field(use_a, "id").to_string(),
            str_field(a_t, "id").to_string()
        )) && inh.contains(&(
            str_field(use_b, "id").to_string(),
            str_field(b_t, "id").to_string()
        )),
        "each Use must bind its own namespace's T"
    );
    assert!(
        !inh.contains(&(
            str_field(use_a, "id").to_string(),
            str_field(b_t, "id").to_string()
        )) && !inh.contains(&(
            str_field(use_b, "id").to_string(),
            str_field(a_t, "id").to_string()
        )),
        "no cross-namespace binding"
    );
    Ok(())
}

/// b9d8067 (#1562): a file-level `using N;` reaches every namespace block.
#[test]
fn csharp_file_level_using_applies_across_blocks() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let a = write_file(tmp.path(), "n.cs", "namespace N { class T {} }\n")?;
    let b = write_file(
        tmp.path(),
        "u.cs",
        "using N;\nnamespace A { class X : T {} } namespace B { class Y : T {} }\n",
    )?;
    let res = extract(&[a, b], Some(tmp.path()));
    let resolved: Vec<_> = targets(&res, "inherits", "T")
        .into_iter()
        .filter(|t| !str_field(t, "source_file").is_empty())
        .collect();
    assert!(
        resolved.len() >= 2,
        "file-level using N must reach both A.X and B.Y: {resolved:?}"
    );
    Ok(())
}

/// b9d8067 (#1562): a namespace-scoped `using N;` binds only within its block,
/// not a sibling block of the same namespace.
#[test]
fn csharp_namespace_scoped_using_isolated_to_sibling_block() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let a = write_file(tmp.path(), "n.cs", "namespace N { class T {} }\n")?;
    let b = write_file(
        tmp.path(),
        "u.cs",
        "namespace A { using N; class Good : T {} }\nnamespace A { class Bad : T {} }\n",
    )?;
    let res = extract(&[a, b], Some(tmp.path()));
    let good = find_node(&res, "Good").ok_or("Good node missing")?;
    let bad = find_node(&res, "Bad").ok_or("Bad node missing")?;
    let n_t = def1(&res, "T").ok_or("N.T def missing")?;
    let inh = rel_pairs(&res, "inherits");
    assert!(
        inh.contains(&(
            str_field(good, "id").to_string(),
            str_field(n_t, "id").to_string()
        )),
        "Good (same block as using N) must bind N.T"
    );
    assert!(
        !inh.contains(&(
            str_field(bad, "id").to_string(),
            str_field(n_t, "id").to_string()
        )),
        "Bad (sibling block, no using) must NOT bind N.T"
    );
    Ok(())
}

/// b9d8067 (#1562): a `using N;` in an outer block flows into a nested block.
#[test]
fn csharp_using_flows_into_nested_block() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let a = write_file(tmp.path(), "n.cs", "namespace N { class T {} }\n")?;
    let b = write_file(
        tmp.path(),
        "u.cs",
        "namespace A { using N; namespace B { class Inner : T {} } }\n",
    )?;
    let res = extract(&[a, b], Some(tmp.path()));
    let resolved: Vec<_> = targets(&res, "inherits", "T")
        .into_iter()
        .filter(|t| !str_field(t, "source_file").is_empty())
        .collect();
    assert!(
        !resolved.is_empty(),
        "using N in outer block A must flow into nested block B"
    );
    Ok(())
}

/// b9d8067 (#1562): an alias `using` binds only within its declaring block.
#[test]
fn csharp_alias_using_scoped_to_its_block() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let a = write_file(tmp.path(), "n.cs", "namespace N { class T {} }\n")?;
    let b = write_file(
        tmp.path(),
        "u.cs",
        "namespace A { using AliasT = N.T; class Good : AliasT {} }\nnamespace A { class Bad : AliasT {} }\n",
    )?;
    let res = extract(&[a, b], Some(tmp.path()));
    let good = find_node(&res, "Good").ok_or("Good node missing")?;
    let bad = find_node(&res, "Bad").ok_or("Bad node missing")?;
    let n_t = def1(&res, "T").ok_or("N.T def missing")?;
    let inh = rel_pairs(&res, "inherits");
    assert!(
        inh.contains(&(
            str_field(good, "id").to_string(),
            str_field(n_t, "id").to_string()
        )),
        "Good must bind N.T via the in-block alias"
    );
    assert!(
        !inh.contains(&(
            str_field(bad, "id").to_string(),
            str_field(n_t, "id").to_string()
        )),
        "Bad (sibling block) must NOT see the alias"
    );
    Ok(())
}

/// A candidate node's string field (`None` when the node or field is absent).
fn opt_field<'a>(t: Option<&'a Obj>, key: &str) -> Option<&'a str> {
    t.and_then(|n| n.get(key).and_then(Value::as_str))
}
