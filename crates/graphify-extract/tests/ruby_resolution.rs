//! Parity tests for type-aware Ruby call-graph resolution (#1499), ported from
//! `graphify-py/tests/test_ruby_resolution.py`.
//!
//! Member calls capture their receiver (extraction); `var = ClassName.new` local
//! bindings give the receiver a type (extraction); the cross-file resolver turns
//! `var.method` into a precise edge BY TYPE, not by globally-unique name — so it
//! survives name collisions and never emits a false positive when the type is
//! unknown (resolution). Every resolved edge must be EXTRACTED confidence.

use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use graphify_extract::{ExtractOutput, FileResult, RawCall, extract, extract_ruby};
use serde_json::Value;
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn Error>>;

const HELPER_RB: &str = "def transform(data)\n  data.upcase\nend\n\n\
class Processor\n  def run(items)\n    items.map { |i| transform(i) }\n  end\nend\n";

const MAIN_RB: &str = "require_relative \"helper\"\n\n\
def handle(values)\n  transform(values)\nend\n\n\
def process_all(items)\n  p = Processor.new\n  p.run(items)\nend\n";

const WORKER_RB: &str = "class Worker\n  def run(jobs)\n    jobs.each { |j| j }\n  end\nend\n";

fn write(dir: &Path, name: &str, body: &str) -> std::io::Result<PathBuf> {
    let p = dir.join(name);
    std::fs::write(&p, body)?;
    Ok(p)
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
fn member_call_captures_receiver() -> TestResult {
    let tmp = tempdir()?;
    let main = write(tmp.path(), "main.rb", MAIN_RB)?;
    let r = extract_ruby(&main);
    let rc = find_raw_call(&r, "run").ok_or("p.run should produce a raw_call with callee 'run'")?;
    assert!(rc.is_member_call);
    assert_eq!(rc.receiver.as_deref(), Some("p"));
    Ok(())
}

#[test]
fn local_binding_gives_receiver_a_type() -> TestResult {
    let tmp = tempdir()?;
    let main = write(tmp.path(), "main.rb", MAIN_RB)?;
    let r = extract_ruby(&main);
    let rc = find_raw_call(&r, "run").ok_or("missing run raw_call")?;
    // `p = Processor.new` in the same method => p has type Processor.
    assert_eq!(rc.receiver_type.as_deref(), Some("Processor"));
    Ok(())
}

#[test]
fn ambiguous_binding_yields_no_type() -> TestResult {
    let tmp = tempdir()?;
    let main = write(
        tmp.path(),
        "main.rb",
        "def process_all(items)\n  p = Processor.new\n  p = Worker.new\n  p.run(items)\nend\n",
    )?;
    let r = extract_ruby(&main);
    let rc = find_raw_call(&r, "run").ok_or("missing run raw_call")?;
    // reassigned to a different class => not certain => no type attached.
    assert_eq!(rc.receiver_type, None);
    Ok(())
}

// ── resolution level ──────────────────────────────────────────────────────────

#[test]
fn resolves_member_call_by_type() -> TestResult {
    let tmp = tempdir()?;
    write(tmp.path(), "helper.rb", HELPER_RB)?;
    let main = write(tmp.path(), "main.rb", MAIN_RB)?;
    let graph = extract(&[main, tmp.path().join("helper.rb")], Some(tmp.path()));
    let labels = label_map(&graph);
    let (_, conf) = call_edge(&graph, &labels, "process_all", "run")
        .ok_or("process_all should resolve a call to Processor#run")?;
    assert_eq!(conf, "EXTRACTED");
    Ok(())
}

#[test]
fn resolution_is_type_based_not_name_luck() -> TestResult {
    // The differentiator: adding an unrelated Worker#run must NOT break the edge.
    // A name-match resolver drops this (two `run` defs => ambiguous); a type-based
    // resolver keeps resolving p.run -> Processor#run, never Worker#run.
    let tmp = tempdir()?;
    write(tmp.path(), "helper.rb", HELPER_RB)?;
    write(tmp.path(), "worker.rb", WORKER_RB)?;
    let main = write(tmp.path(), "main.rb", MAIN_RB)?;
    let graph = extract(
        &[
            main,
            tmp.path().join("helper.rb"),
            tmp.path().join("worker.rb"),
        ],
        Some(tmp.path()),
    );
    let labels = label_map(&graph);
    let (tgt_id, conf) = call_edge(&graph, &labels, "process_all", "run")
        .ok_or("edge must survive the collision")?;
    assert_eq!(conf, "EXTRACTED");
    // And it must be the RIGHT run: the target node id is prefixed by its owning
    // class (helper_processor_run), so it must mention processor, never worker.
    assert!(
        tgt_id.to_lowercase().contains("processor"),
        "expected Processor#run, got {tgt_id}"
    );
    assert!(!tgt_id.to_lowercase().contains("worker"));
    Ok(())
}

#[test]
fn no_false_positive_when_type_unknown() -> TestResult {
    // A member call on a receiver with no known type must NOT be resolved.
    let tmp = tempdir()?;
    write(tmp.path(), "helper.rb", HELPER_RB)?;
    let main = write(
        tmp.path(),
        "main.rb",
        "require_relative \"helper\"\n\ndef process_all(thing)\n  thing.run(1)\nend\n",
    )?;
    let graph = extract(&[main, tmp.path().join("helper.rb")], Some(tmp.path()));
    let labels = label_map(&graph);
    // `thing` is a parameter of unknown type => no precise target => no edge.
    assert!(call_edge(&graph, &labels, "process_all", "run").is_none());
    Ok(())
}

#[test]
fn class_new_creates_instantiation_edge() -> TestResult {
    // `p = Processor.new` should link the caller to the Processor class.
    let tmp = tempdir()?;
    write(tmp.path(), "helper.rb", HELPER_RB)?;
    let main = write(tmp.path(), "main.rb", MAIN_RB)?;
    let graph = extract(&[main, tmp.path().join("helper.rb")], Some(tmp.path()));
    let labels = label_map(&graph);
    let (_, conf) = call_edge(&graph, &labels, "process_all", "Processor")
        .ok_or("Processor.new should resolve a call to the Processor class")?;
    assert_eq!(conf, "EXTRACTED");
    Ok(())
}

#[test]
fn class_new_resolves_for_method_less_class() -> TestResult {
    // A method-less class still resolves `X.new`: the Ruby class index is built
    // from `contains` nodes, not only `method` edges, so `class Config; end`
    // (which emits no `method` edge) is still found.
    let tmp = tempdir()?;
    write(tmp.path(), "config.rb", "class Config\nend\n")?;
    let main = write(
        tmp.path(),
        "main.rb",
        "require_relative \"config\"\n\ndef boot\n  c = Config.new\n  c\nend\n",
    )?;
    let graph = extract(&[main, tmp.path().join("config.rb")], Some(tmp.path()));
    let labels = label_map(&graph);
    let (_, conf) = call_edge(&graph, &labels, "boot", "Config")
        .ok_or("Config.new should resolve to the method-less Config class")?;
    assert_eq!(conf, "EXTRACTED");
    Ok(())
}

#[test]
fn reassignment_to_untyped_value_clears_type() -> TestResult {
    // The 100%-confidence contract: a variable reassigned to anything other than
    // a single `Constant.new` (here a plain method call) becomes ambiguous, so no
    // type is carried and the member call is never resolved by type.
    let tmp = tempdir()?;
    let main = write(
        tmp.path(),
        "main.rb",
        "def process_all(items)\n  p = Processor.new\n  p = items.first\n  p.run(items)\nend\n",
    )?;
    let result = extract_ruby(&main);
    let rc = find_raw_call(&result, "run").ok_or("missing run raw_call")?;
    assert_eq!(
        rc.receiver_type, None,
        "reassign to an untyped value poisons the type"
    );
    Ok(())
}
// ── #1640 node extraction + #1634 constant-receiver resolution ──────────────

/// Distinct node labels of a single-file extraction.
fn fr_labels(r: &FileResult) -> std::collections::HashSet<&str> {
    r.nodes.iter().map(|n| n.label.as_str()).collect()
}

/// `(source_label, target_label)` pairs for edges of the given `relation`.
fn fr_rel_pairs<'a>(
    r: &'a FileResult,
    relation: &str,
) -> std::collections::HashSet<(&'a str, &'a str)> {
    let by_id: HashMap<&str, &str> = r
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    r.edges
        .iter()
        .filter(|e| e.relation == relation)
        .map(|e| {
            (
                *by_id.get(e.source.as_str()).unwrap_or(&""),
                *by_id.get(e.target.as_str()).unwrap_or(&""),
            )
        })
        .collect()
}

#[test]
fn plain_module_gets_a_node_with_methods() -> TestResult {
    // #1640 shape 1: `module Foo` must get a node and own its methods.
    let tmp = tempdir()?;
    let f = write(
        tmp.path(),
        "tax.rb",
        "module TaxCalculator\n  module_function\n  def rate_for(order)\n    0.2\n  end\nend\n",
    )?;
    let r = extract_ruby(&f);
    assert!(fr_labels(&r).contains("TaxCalculator"));
    assert!(fr_rel_pairs(&r, "method").contains(&("TaxCalculator", ".rate_for()")));
    Ok(())
}

#[test]
fn nested_modules_each_get_a_node() -> TestResult {
    // #1640 shape 1, nested.
    let tmp = tempdir()?;
    let f = write(
        tmp.path(),
        "n.rb",
        "module Billing\n  module Rounding\n    def round(x)\n      x.round(2)\n    end\n  end\nend\n",
    )?;
    let r = extract_ruby(&f);
    let labels = fr_labels(&r);
    assert!(labels.contains("Billing") && labels.contains("Rounding"));
    assert!(fr_rel_pairs(&r, "method").contains(&("Rounding", ".round()")));
    Ok(())
}

#[test]
fn struct_new_constant_creates_class_with_methods() -> TestResult {
    // #1640 shape 2: `Foo = Struct.new(...) do ... end`.
    let tmp = tempdir()?;
    let f = write(
        tmp.path(),
        "invoice.rb",
        "Invoice = Struct.new(:total, :tax) do\n  def grand_total\n    total + tax\n  end\nend\n",
    )?;
    let r = extract_ruby(&f);
    assert!(fr_labels(&r).contains("Invoice"));
    assert!(fr_rel_pairs(&r, "method").contains(&("Invoice", ".grand_total()")));
    Ok(())
}

#[test]
fn class_new_constant_creates_class_and_inherits() -> TestResult {
    // #1640 shape 3: `Foo = Class.new(Super)` — node + inherits edge.
    let tmp = tempdir()?;
    let f = write(
        tmp.path(),
        "err.rb",
        "ApiError = Class.new(StandardError)\n",
    )?;
    let r = extract_ruby(&f);
    assert!(fr_labels(&r).contains("ApiError"));
    assert!(fr_rel_pairs(&r, "inherits").contains(&("ApiError", "StandardError")));
    Ok(())
}

#[test]
fn data_define_constant_creates_class() -> TestResult {
    let tmp = tempdir()?;
    let f = write(tmp.path(), "res.rb", "Result = Data.define(:ok, :value)\n")?;
    let r = extract_ruby(&f);
    assert!(fr_labels(&r).contains("Result"));
    Ok(())
}

#[test]
fn constant_receiver_singleton_call_resolves() -> TestResult {
    // #1634: `Processor.call` (def self.call) resolves to the singleton method.
    let tmp = tempdir()?;
    write(
        tmp.path(),
        "processor.rb",
        "class Processor\n  def self.call; end\nend\n",
    )?;
    let runner = write(
        tmp.path(),
        "runner.rb",
        "class Runner\n  def run\n    Processor.call\n  end\nend\n",
    )?;
    let graph = extract(&[runner, tmp.path().join("processor.rb")], Some(tmp.path()));
    let labels = label_map(&graph);
    assert!(call_edge(&graph, &labels, "run", "call").is_some());
    Ok(())
}

#[test]
fn constant_receiver_module_function_call_resolves() -> TestResult {
    // #1634 + #1640: `TaxCalculator.rate_for` resolves across files to a
    // module_function — needs both the module node (#1640) and the resolver (#1634).
    let tmp = tempdir()?;
    write(
        tmp.path(),
        "tax.rb",
        "module TaxCalculator\n  module_function\n  def rate_for(o)\n    0.2\n  end\nend\n",
    )?;
    let pp = write(
        tmp.path(),
        "pp.rb",
        "class PaymentProcessor\n  def process(order)\n    TaxCalculator.rate_for(order)\n  end\nend\n",
    )?;
    let graph = extract(&[pp, tmp.path().join("tax.rb")], Some(tmp.path()));
    let labels = label_map(&graph);
    assert!(call_edge(&graph, &labels, "process", "rate_for").is_some());
    Ok(())
}

#[test]
fn constant_receiver_unknown_class_method_falls_back_to_class() -> TestResult {
    // #1634: `Model.where` (no `where` def) still links to the class node for
    // blast-radius rather than dropping the edge.
    let tmp = tempdir()?;
    write(
        tmp.path(),
        "model.rb",
        "class Model\n  def self.create; end\nend\n",
    )?;
    let caller = write(
        tmp.path(),
        "svc.rb",
        "class Svc\n  def run\n    Model.where(id: 1)\n  end\nend\n",
    )?;
    let graph = extract(&[caller, tmp.path().join("model.rb")], Some(tmp.path()));
    let labels = label_map(&graph);
    // No `where` method node exists, so the edge lands on the class node itself.
    assert!(call_edge(&graph, &labels, "run", "Model").is_some());
    Ok(())
}

#[test]
fn ambiguous_constant_receiver_emits_no_edge() -> TestResult {
    // Two classes named `Processor` => ambiguous receiver => bail (no wrong edge).
    let tmp = tempdir()?;
    write(
        tmp.path(),
        "a.rb",
        "module A\n  class Processor\n    def self.call; end\n  end\nend\n",
    )?;
    write(
        tmp.path(),
        "b.rb",
        "module B\n  class Processor\n    def self.call; end\n  end\nend\n",
    )?;
    let caller = write(
        tmp.path(),
        "c.rb",
        "class Runner\n  def run\n    Processor.call\n  end\nend\n",
    )?;
    let graph = extract(
        &[caller, tmp.path().join("a.rb"), tmp.path().join("b.rb")],
        Some(tmp.path()),
    );
    let labels = label_map(&graph);
    assert!(call_edge(&graph, &labels, "run", "call").is_none());
    Ok(())
}

#[test]
fn constant_receiver_never_binds_across_language_families() -> TestResult {
    // The Ruby class index is `.rb`-scoped: a Ruby `Model.where` must not bind to
    // a same-named Java `Model` (#1634 / cross-language guard). Only a Java `Model`
    // exists here, so the call resolves to nothing rather than a phantom edge.
    let tmp = tempdir()?;
    write(
        tmp.path(),
        "Model.java",
        "public class Model {\n    public static void where() {}\n}\n",
    )?;
    let caller = write(
        tmp.path(),
        "svc.rb",
        "class Svc\n  def run\n    Model.where\n  end\nend\n",
    )?;
    let graph = extract(&[caller, tmp.path().join("Model.java")], Some(tmp.path()));
    let labels = label_map(&graph);
    // A regressed all-language index would bind either to the Java `Model` class
    // (target label `Model`) OR to its `.where()` method (target label `.where()`),
    // so reject both.
    assert!(
        call_edge(&graph, &labels, "run", "Model").is_none(),
        "Ruby call must not bind to a Java class of the same name"
    );
    assert!(
        call_edge(&graph, &labels, "run", "where").is_none(),
        "Ruby call must not bind to a Java method of the same name"
    );
    Ok(())
}
#[test]
fn namespaced_constant_receiver_resolves_by_last_constant() -> TestResult {
    // `Billing::Processor.call` — a namespaced receiver resolves by its last
    // constant (`Processor`), exercising the `scope_resolution` receiver capture.
    let tmp = tempdir()?;
    write(
        tmp.path(),
        "processor.rb",
        "module Billing\n  class Processor\n    def self.call; end\n  end\nend\n",
    )?;
    let runner = write(
        tmp.path(),
        "runner.rb",
        "class Runner\n  def run\n    Billing::Processor.call\n  end\nend\n",
    )?;
    let graph = extract(&[runner, tmp.path().join("processor.rb")], Some(tmp.path()));
    let labels = label_map(&graph);
    assert!(
        call_edge(&graph, &labels, "run", "call").is_some(),
        "a namespaced receiver must resolve by its last constant"
    );
    Ok(())
}
