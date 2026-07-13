//! Parity tests for TS/JS receiver-typed cross-file member-call resolution,
//! ported from graphify-py `test_languages.py` (#1316 constructor injection),
//! `test_ts_receiver_member_calls.py` (#1630 new-binding / typed-param), and
//! `test_builtin_global_type_refs.py` (#1726 builtin-global skip).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;
use std::fs;

use graphify_extract::{ExtractOutput, extract};
use indexmap::IndexMap;
use serde_json::Value;

type Obj = IndexMap<String, Value>;

/// Write `(relpath, content)` fixtures into a tempdir (nested dirs kept) and run
/// the corpus `extract()`. The tempdir is returned to keep it alive.
fn corpus(files: &[(&str, &str)]) -> (tempfile::TempDir, ExtractOutput) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut paths = Vec::new();
    for (rel, content) in files {
        let p = tmp.path().join(rel);
        fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        fs::write(&p, content).expect("write");
        paths.push(p);
    }
    let out = extract(&paths, Some(tmp.path()));
    (tmp, out)
}

fn field<'a>(m: &'a Obj, key: &str) -> &'a str {
    m.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn label_of(out: &ExtractOutput, id: &str) -> String {
    out.nodes
        .iter()
        .find(|n| field(n, "id") == id)
        .map_or_else(|| id.to_string(), |n| field(n, "label").to_string())
}

/// `(source_label, target_label)` pairs for `calls` edges.
fn calls(out: &ExtractOutput) -> HashSet<(String, String)> {
    out.edges
        .iter()
        .filter(|e| field(e, "relation") == "calls")
        .map(|e| {
            (
                label_of(out, field(e, "source")),
                label_of(out, field(e, "target")),
            )
        })
        .collect()
}

const SVC: &str = "export class Svc {\n  doThing(): number { return 1; }\n}\n";

// ── #1316: constructor-injection this.field.method() ──────────────────────────

/// `this.repo.findById()` with `constructor(private repo: IUserRepository)`
/// resolves to the interface method across files.
#[test]
fn ts_constructor_injection_calls_edge() {
    let (_t, out) = corpus(&[
        (
            "repo.ts",
            "export interface IUserRepository {\n  findById(id: string): Promise<any>;\n  save(user: any): Promise<void>;\n}\n",
        ),
        (
            "service.ts",
            "import { IUserRepository } from './repo';\n\nexport class UserService {\n  constructor(private repo: IUserRepository) {}\n\n  getUser(id: string) {\n    return this.repo.findById(id);\n  }\n}\n",
        ),
    ]);
    assert!(
        calls(&out)
            .iter()
            .any(|(s, t)| s.contains("getUser") && t.contains("findById")),
        "expected getUser()->findById() calls edge: {:?}",
        calls(&out)
    );
}

/// `this.db.query()` must NOT bind to an unrelated same-file bare `query()`.
#[test]
fn ts_this_field_receiver_not_same_file_collision() {
    let (_t, out) = corpus(&[(
        "collision.ts",
        "function query() { return 'global'; }\n\nexport class Service {\n  constructor(private db: Database) {}\n\n  run() {\n    return this.db.query();\n  }\n}\n",
    )]);
    assert!(
        !calls(&out)
            .iter()
            .any(|(s, t)| s.contains("run") && t.contains("query")),
        "this.db.query() must not resolve to bare query(): {:?}",
        calls(&out)
    );
}

/// Two classes define `query`; the injected field is typed `Database`, so
/// `this.db.query()` binds Database.query ONLY (no name-match fan-out).
#[test]
fn ts_injected_field_resolves_to_typed_class_not_same_named_collision() {
    let (_t, out) = corpus(&[
        (
            "database.ts",
            "export class Database {\n  query(sql: string) { return sql; }\n}\n",
        ),
        (
            "http.ts",
            "export class HttpClient {\n  query(url: string) { return url; }\n}\n",
        ),
        (
            "service.ts",
            "import { Database } from './database';\nexport class Service {\n  constructor(private db: Database) {}\n  run() { return this.db.query('x'); }\n}\n",
        ),
    ]);
    let method_owner: std::collections::HashMap<String, String> = out
        .edges
        .iter()
        .filter(|e| field(e, "relation") == "method")
        .map(|e| {
            (
                field(e, "target").to_string(),
                field(e, "source").to_string(),
            )
        })
        .collect();
    let run_query_targets: Vec<String> = out
        .edges
        .iter()
        .filter(|e| {
            field(e, "relation") == "calls"
                && label_of(&out, field(e, "source")).contains("run")
                && label_of(&out, field(e, "target")).contains("query")
        })
        .map(|e| field(e, "target").to_string())
        .collect();
    assert!(
        !run_query_targets.is_empty(),
        "expected this.db.query() to resolve"
    );
    for tgt in &run_query_targets {
        let owner = method_owner.get(tgt).map(|o| label_of(&out, o));
        assert_eq!(
            owner.as_deref(),
            Some("Database"),
            "must resolve to Database.query"
        );
    }
}

/// An ambiguous injected type (two classes named `Database`) bails — no edge.
#[test]
fn ts_injected_field_ambiguous_type_emits_no_edge() {
    let (_t, out) = corpus(&[
        (
            "a/database.ts",
            "export class Database {\n  query(sql: string) { return sql; }\n}\n",
        ),
        (
            "b/database.ts",
            "export class Database {\n  query(sql: string) { return sql; }\n}\n",
        ),
        (
            "service.ts",
            "export class Service {\n  constructor(private db: Database) {}\n  run() { return this.db.query('x'); }\n}\n",
        ),
    ]);
    assert!(
        !calls(&out)
            .iter()
            .any(|(s, t)| s.contains("run") && t.contains("query")),
        "ambiguous Database type must not produce a this.db.query() edge"
    );
}

// ── #1630: local new-binding + typed-param receivers ──────────────────────────

/// `const s = new Svc(); s.doThing()` resolves across files.
#[test]
fn ts_local_new_binding_receiver() {
    let (_t, out) = corpus(&[
        ("svc.ts", SVC),
        (
            "direct.ts",
            "import { Svc } from \"./svc\";\nconst s = new Svc();\nexport function usesDirect(): number { return s.doThing(); }\n",
        ),
    ]);
    assert!(
        calls(&out)
            .iter()
            .any(|(s, t)| s.contains("usesDirect") && t.contains("doThing"))
    );
}

/// A closure over a typed parameter (`register(svc: Svc): () => svc.doThing()`)
/// resolves — the returned arrow's call attributes to the enclosing function.
#[test]
fn ts_closure_over_typed_param_receiver() {
    let (_t, out) = corpus(&[
        ("svc.ts", SVC),
        (
            "closure.ts",
            "import { Svc } from \"./svc\";\nexport function register(svc: Svc): () => number { return () => svc.doThing(); }\n",
        ),
    ]);
    assert!(
        calls(&out)
            .iter()
            .any(|(s, t)| s.contains("register") && t.contains("doThing"))
    );
}

/// A `new` binding resolves to the right class under a same-method-name collision.
#[test]
fn ts_new_binding_resolves_to_correct_class_under_ambiguity() {
    let (_t, out) = corpus(&[
        ("svc.ts", SVC),
        (
            "cache.ts",
            "export class Cache {\n  doThing(): number { return 2; }\n}\n",
        ),
        (
            "d.ts",
            "import { Svc } from \"./svc\";\nconst s = new Svc();\nexport function f(): number { return s.doThing(); }\n",
        ),
    ]);
    let tgts: Vec<String> = out
        .edges
        .iter()
        .filter(|e| field(e, "relation") == "calls" && field(e, "source").contains("_f"))
        .map(|e| field(e, "target").to_string())
        .collect();
    assert!(!tgts.is_empty(), "expected f()->doThing() edge");
    assert!(
        tgts.iter().all(|t| t.to_lowercase().contains("svc")),
        "must bind Svc: {tgts:?}"
    );
    assert!(!tgts.iter().any(|t| t.to_lowercase().contains("cache")));
}

/// An untyped parameter receiver produces no edge (no guess).
#[test]
fn ts_untyped_param_receiver_emits_no_edge() {
    let (_t, out) = corpus(&[
        ("svc.ts", SVC),
        (
            "n.ts",
            "export function g(x): number { return x.doThing(); }\n",
        ),
    ]);
    assert!(!calls(&out).iter().any(|(_, t)| t.contains("doThing")));
}

/// An array-typed receiver (`xs: Svc[]`) produces no edge.
#[test]
fn ts_array_typed_receiver_emits_no_edge() {
    let (_t, out) = corpus(&[
        ("svc.ts", SVC),
        (
            "a.ts",
            "import { Svc } from \"./svc\";\nexport function h(xs: Svc[]): number { return xs[0].doThing(); }\n",
        ),
    ]);
    assert!(
        !calls(&out)
            .iter()
            .any(|(s, t)| s.contains("h(") && t.contains("doThing"))
    );
}

// ── #1726: builtin-global receiver types must not bind user symbols ───────────

/// `x: Date; x.getTime()` must not bind to a user `class DATE`.
#[test]
fn ts_builtin_date_type_ref_does_not_bind_to_user_date() {
    let (_t, out) = corpus(&[
        (
            "model.ts",
            "export class DATE {\n  value: string = \"\";\n}\n",
        ),
        (
            "a.ts",
            "export function parse(x: Date): number { return x.getTime(); }\n",
        ),
        (
            "b.ts",
            "export function fmt(w: Date): string { return w.toISOString(); }\n",
        ),
    ]);
    let date_ids: HashSet<&str> = out
        .nodes
        .iter()
        .filter(|n| field(n, "label") == "DATE")
        .map(|n| field(n, "id"))
        .collect();
    assert!(!date_ids.is_empty(), "user class DATE must still exist");
    assert!(
        !out.edges
            .iter()
            .any(|e| field(e, "relation") == "references" && date_ids.contains(field(e, "target"))),
        "phantom builtin-Date reference bound to user DATE"
    );
    let d0 = *date_ids.iter().next().unwrap();
    let deg = out
        .edges
        .iter()
        .filter(|e| field(e, "source") == d0 || field(e, "target") == d0)
        .count();
    assert!(deg <= 1, "user DATE should not be a god node; degree={deg}");
}

/// The builtin guard is a no-op for a genuine user type: a user-typed field's
/// member call still resolves cross-file.
#[test]
fn ts_nonbuiltin_receiver_type_still_resolves() {
    let (_t, out) = corpus(&[
        (
            "svc.ts",
            "export class PaymentClient {\n  charge(n: number): boolean { return true; }\n}\n",
        ),
        (
            "order.ts",
            "import { PaymentClient } from \"./svc\";\nexport class Order {\n  constructor(private client: PaymentClient) {}\n  pay(): boolean { return this.client.charge(1); }\n}\n",
        ),
    ]);
    assert!(
        out.edges.iter().any(|e| {
            field(e, "target").to_lowercase().contains("charge")
                && label_of(&out, field(e, "source")).contains("pay")
        }),
        "user member-call must still resolve"
    );
}
