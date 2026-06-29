//! Parity tests for type-aware Ruby call-graph resolution (#1499), ported from
//! `graphify-py/tests/test_ruby_resolution.py`.
//!
//! Member calls capture their receiver (extraction); `var = ClassName.new` local
//! bindings give the receiver a type (extraction); the cross-file resolver turns
//! `var.method` into a precise edge BY TYPE, not by globally-unique name — so it
//! survives name collisions and never emits a false positive when the type is
//! unknown (resolution). Every resolved edge must be EXTRACTED confidence.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use graphify_extract::{ExtractOutput, FileResult, RawCall, extract, extract_ruby};
use serde_json::Value;
use tempfile::tempdir;

const HELPER_RB: &str = "def transform(data)\n  data.upcase\nend\n\n\
class Processor\n  def run(items)\n    items.map { |i| transform(i) }\n  end\nend\n";

const MAIN_RB: &str = "require_relative \"helper\"\n\n\
def handle(values)\n  transform(values)\nend\n\n\
def process_all(items)\n  p = Processor.new\n  p.run(items)\nend\n";

const WORKER_RB: &str = "class Worker\n  def run(jobs)\n    jobs.each { |j| j }\n  end\nend\n";

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).expect("write fixture");
    p
}

fn find_raw_call<'a>(r: &'a FileResult, callee: &str) -> Option<&'a RawCall> {
    r.raw_calls.iter().find(|rc| rc.callee == callee)
}

fn label_map(graph: &ExtractOutput) -> HashMap<&str, &str> {
    graph
        .nodes
        .iter()
        .filter_map(|n| Some((n.get("id")?.as_str()?, n.get("label")?.as_str()?)))
        .collect()
}

/// Return `(target_id, confidence)` for the `calls` edge whose source/target
/// labels contain the given substrings, or `None`. Mirrors Python `_has_call_edge`.
fn call_edge<'a>(
    graph: &'a ExtractOutput,
    labels: &HashMap<&str, &str>,
    src_sub: &str,
    tgt_sub: &str,
) -> Option<(&'a str, &'a str)> {
    graph.edges.iter().find_map(|e| {
        if e.get("relation").and_then(Value::as_str) != Some("calls") {
            return None;
        }
        let s = e.get("source").and_then(Value::as_str)?;
        let t = e.get("target").and_then(Value::as_str)?;
        if labels.get(s).copied().unwrap_or("").contains(src_sub)
            && labels.get(t).copied().unwrap_or("").contains(tgt_sub)
        {
            Some((t, e.get("confidence").and_then(Value::as_str).unwrap_or("")))
        } else {
            None
        }
    })
}

// ── extraction level ────────────────────────────────────────────────────────

#[test]
fn member_call_captures_receiver() {
    let tmp = tempdir().unwrap();
    let main = write(tmp.path(), "main.rb", MAIN_RB);
    let r = extract_ruby(&main);
    let rc = find_raw_call(&r, "run").expect("p.run should produce a raw_call with callee 'run'");
    assert!(rc.is_member_call);
    assert_eq!(rc.receiver.as_deref(), Some("p"));
}

#[test]
fn local_binding_gives_receiver_a_type() {
    let tmp = tempdir().unwrap();
    let main = write(tmp.path(), "main.rb", MAIN_RB);
    let r = extract_ruby(&main);
    let rc = find_raw_call(&r, "run").unwrap();
    // `p = Processor.new` in the same method => p has type Processor.
    assert_eq!(rc.receiver_type.as_deref(), Some("Processor"));
}

#[test]
fn ambiguous_binding_yields_no_type() {
    let tmp = tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.rb",
        "def process_all(items)\n  p = Processor.new\n  p = Worker.new\n  p.run(items)\nend\n",
    );
    let r = extract_ruby(&main);
    let rc = find_raw_call(&r, "run").unwrap();
    // reassigned to a different class => not certain => no type attached.
    assert_eq!(rc.receiver_type, None);
}

// ── resolution level ──────────────────────────────────────────────────────────

#[test]
fn resolves_member_call_by_type() {
    let tmp = tempdir().unwrap();
    write(tmp.path(), "helper.rb", HELPER_RB);
    let main = write(tmp.path(), "main.rb", MAIN_RB);
    let graph = extract(&[main, tmp.path().join("helper.rb")], Some(tmp.path()));
    let labels = label_map(&graph);
    let edge = call_edge(&graph, &labels, "process_all", "run");
    assert!(
        edge.is_some(),
        "process_all should resolve a call to Processor#run"
    );
    assert_eq!(edge.unwrap().1, "EXTRACTED");
}

#[test]
fn resolution_is_type_based_not_name_luck() {
    // The differentiator: adding an unrelated Worker#run must NOT break the edge.
    // A name-match resolver drops this (two `run` defs => ambiguous); a type-based
    // resolver keeps resolving p.run -> Processor#run, never Worker#run.
    let tmp = tempdir().unwrap();
    write(tmp.path(), "helper.rb", HELPER_RB);
    write(tmp.path(), "worker.rb", WORKER_RB);
    let main = write(tmp.path(), "main.rb", MAIN_RB);
    let graph = extract(
        &[
            main,
            tmp.path().join("helper.rb"),
            tmp.path().join("worker.rb"),
        ],
        Some(tmp.path()),
    );
    let labels = label_map(&graph);
    let edge = call_edge(&graph, &labels, "process_all", "run");
    assert!(edge.is_some(), "edge must survive the name collision");
    let (tgt_id, conf) = edge.unwrap();
    assert_eq!(conf, "EXTRACTED");
    // And it must be the RIGHT run: the target node id is prefixed by its owning
    // class (helper_processor_run), so it must mention processor, never worker.
    assert!(
        tgt_id.to_lowercase().contains("processor"),
        "expected Processor#run, got {tgt_id}"
    );
    assert!(!tgt_id.to_lowercase().contains("worker"));
}

#[test]
fn no_false_positive_when_type_unknown() {
    // A member call on a receiver with no known type must NOT be resolved.
    let tmp = tempdir().unwrap();
    write(tmp.path(), "helper.rb", HELPER_RB);
    let main = write(
        tmp.path(),
        "main.rb",
        "require_relative \"helper\"\n\ndef process_all(thing)\n  thing.run(1)\nend\n",
    );
    let graph = extract(&[main, tmp.path().join("helper.rb")], Some(tmp.path()));
    let labels = label_map(&graph);
    // `thing` is a parameter of unknown type => no precise target => no edge.
    assert!(call_edge(&graph, &labels, "process_all", "run").is_none());
}

#[test]
fn class_new_creates_instantiation_edge() {
    // `p = Processor.new` should link the caller to the Processor class.
    let tmp = tempdir().unwrap();
    write(tmp.path(), "helper.rb", HELPER_RB);
    let main = write(tmp.path(), "main.rb", MAIN_RB);
    let graph = extract(&[main, tmp.path().join("helper.rb")], Some(tmp.path()));
    let labels = label_map(&graph);
    let edge = call_edge(&graph, &labels, "process_all", "Processor");
    assert!(
        edge.is_some(),
        "Processor.new should resolve a call to the Processor class"
    );
    assert_eq!(edge.unwrap().1, "EXTRACTED");
}

#[test]
fn reassignment_to_untyped_value_clears_type() {
    // The 100%-confidence contract: a variable reassigned to anything other than
    // a single `Constant.new` (here a plain method call) becomes ambiguous, so no
    // type is carried and the member call is never resolved by type.
    let tmp = tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.rb",
        "def process_all(items)\n  p = Processor.new\n  p = items.first\n  p.run(items)\nend\n",
    );
    let result = extract_ruby(&main);
    let rc = find_raw_call(&result, "run").unwrap();
    assert_eq!(
        rc.receiver_type, None,
        "reassign to an untyped value poisons the type"
    );
}
