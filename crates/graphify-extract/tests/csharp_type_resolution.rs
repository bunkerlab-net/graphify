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
