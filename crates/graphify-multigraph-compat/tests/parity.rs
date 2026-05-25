//! Parity tests against `graphify-py/tests/test_multigraph_compat.py`.

use graphify_multigraph_compat::{probe_multigraph_capabilities, require_multigraph_capabilities};

#[test]
fn probe_passes_on_current_runtime() {
    let result = probe_multigraph_capabilities();
    assert!(result.ok(), "probe failed: {}", result.error_message());
}

#[test]
fn probe_runs_all_six_checks() {
    let result = probe_multigraph_capabilities();
    let names: Vec<&str> = result.checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names.len(), 6, "expected exactly 6 probes, got {names:?}");
    assert!(names.contains(&"keyed_parallel_edges"));
    assert!(names.contains(&"node_link_edges_links_round_trip"));
    assert!(names.contains(&"duplicate_key_overwrite_semantics"));
    assert!(names.contains(&"reserved_key_attr_rejected"));
    assert!(names.contains(&"remove_edges_from_two_tuple_semantics"));
    assert!(names.contains(&"to_undirected_preserves_multigraph_type"));
}

#[test]
fn probe_is_cached() {
    // Two calls return the same `&'static` reference — verifies the
    // OnceLock cache is wired up.
    let a = std::ptr::from_ref(probe_multigraph_capabilities());
    let b = std::ptr::from_ref(probe_multigraph_capabilities());
    assert_eq!(a, b);
}

#[test]
fn require_succeeds_when_probe_passes() {
    let result = require_multigraph_capabilities();
    assert!(result.is_ok());
}
