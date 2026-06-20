//! JS/TS assignment-form extraction (#09da529).
//!
//! Ports the new `test_extract_js_*` / `test_extract_ts_*` cases from
//! `graphify-py/tests/test_extract.py`: `this.X = fn` in constructor bodies,
//! `CommonJS` `exports`/`module.exports` assignments, `Foo.prototype.bar = fn`,
//! `const f = function(){}` function expressions, class arrow fields, and the
//! #1077 guard that an arbitrary `obj.x = fn` is NOT captured.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;
use std::error::Error;
use std::path::Path;

use graphify_extract::{FileResult, extract_js};
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn Error>>;

fn write(path: &Path, text: &str) -> TestResult {
    std::fs::write(path, text)?;
    Ok(())
}

fn labels(result: &FileResult) -> Vec<&str> {
    result.nodes.iter().map(|n| n.label.as_str()).collect()
}

fn label_of<'a>(result: &'a FileResult, id: &str) -> Option<&'a str> {
    result
        .nodes
        .iter()
        .find(|n| n.id == id)
        .map(|n| n.label.as_str())
}

/// `(source_id, target_label)` pairs for `method` edges.
fn method_edges(result: &FileResult) -> HashSet<(String, String)> {
    result
        .edges
        .iter()
        .filter(|e| e.relation == "method")
        .map(|e| {
            (
                e.source.clone(),
                label_of(result, &e.target).unwrap_or("").to_string(),
            )
        })
        .collect()
}

#[test]
fn this_assigned_methods() -> TestResult {
    let tmp = tempdir()?;
    let f = tmp.path().join("dao.js");
    write(
        &f,
        "function UserDAO(db) {\n\
         \x20 this.addUser = (name) => { return name; };\n\
         \x20 this.getUser = function(id) { return id; };\n\
         }\n",
    )?;
    let result = extract_js(&f);
    let ls = labels(&result);
    assert!(ls.contains(&"UserDAO()"), "labels: {ls:?}");
    assert!(ls.contains(&".addUser()"), "labels: {ls:?}");
    assert!(ls.contains(&".getUser()"), "labels: {ls:?}");

    // The methods are owned by UserDAO via a `method` edge.
    let owner = result
        .nodes
        .iter()
        .find(|n| n.label == "UserDAO()")
        .unwrap()
        .id
        .clone();
    let edges = method_edges(&result);
    assert!(edges.contains(&(owner.clone(), ".addUser()".to_string())));
    assert!(edges.contains(&(owner, ".getUser()".to_string())));
    Ok(())
}

#[test]
fn commonjs_exports_assignment() -> TestResult {
    let tmp = tempdir()?;
    let f = tmp.path().join("mod.js");
    write(
        &f,
        "exports.alpha = (x) => x;\n\
         module.exports.beta = function(y) { return y; };\n",
    )?;
    let result = extract_js(&f);
    let ls = labels(&result);
    assert!(ls.contains(&"alpha()"), "labels: {ls:?}");
    assert!(ls.contains(&"beta()"), "labels: {ls:?}");
    Ok(())
}

#[test]
fn prototype_method_assignment() -> TestResult {
    let tmp = tempdir()?;
    let f = tmp.path().join("proto.js");
    write(
        &f,
        "function Foo() {}\n\
         Foo.prototype.bar = function() { return 1; };\n",
    )?;
    let result = extract_js(&f);
    let ls = labels(&result);
    assert!(ls.contains(&"Foo()"), "labels: {ls:?}");
    assert!(ls.contains(&".bar()"), "labels: {ls:?}");
    Ok(())
}

#[test]
fn const_function_expression() -> TestResult {
    let tmp = tempdir()?;
    let f = tmp.path().join("fnexpr.js");
    write(&f, "const handler = function(req, res) { return res; };\n")?;
    let result = extract_js(&f);
    assert!(labels(&result).contains(&"handler()"));
    Ok(())
}

#[test]
fn ts_class_arrow_field() -> TestResult {
    let tmp = tempdir()?;
    let f = tmp.path().join("comp.ts");
    write(
        &f,
        "class Widget {\n\
         \x20 onClick = (e) => { return e; };\n\
         \x20 render() { return null; }\n\
         }\n",
    )?;
    let result = extract_js(&f);
    let ls = labels(&result);
    assert!(ls.contains(&"Widget"), "labels: {ls:?}");
    assert!(ls.contains(&".onClick()"), "arrow field; labels: {ls:?}");
    assert!(ls.contains(&".render()"), "plain method; labels: {ls:?}");
    Ok(())
}

#[test]
fn arbitrary_member_assignment_not_captured() -> TestResult {
    // #1077: an arbitrary `obj.x = fn` (obj is neither this/exports/
    // module.exports/<X>.prototype) must NOT produce a node.
    let tmp = tempdir()?;
    let f = tmp.path().join("noise.js");
    write(&f, "const obj = {};\nobj.whatever = () => 1;\n")?;
    let result = extract_js(&f);
    let ls = labels(&result);
    assert!(!ls.contains(&"whatever()"), "labels: {ls:?}");
    assert!(!ls.contains(&".whatever()"), "labels: {ls:?}");
    Ok(())
}
