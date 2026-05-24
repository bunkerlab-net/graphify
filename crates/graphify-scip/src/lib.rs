//! SCIP-style JSON ingestion (simplified subset).
//!
//! Ports `graphify-py/graphify/scip_ingest.py`. Reads a simplified
//! SCIP-style JSON structure (as commonly produced by LLM tools, not the
//! full protobuf) and converts it to graphify's `{nodes, edges}`
//! extraction format. Used by ingest pipelines that need to bridge SCIP
//! tooling into graphify.

use std::collections::HashSet;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use graphify_security::sanitize_metadata;

/// Convert a SCIP-style JSON document into a graphify extraction blob.
///
/// `source_file` and `language` are fallbacks used when a document inside
/// `doc` doesn't carry its own `relative_path` / `language` value. Returns
/// `{"nodes": [], "edges": []}` on any structural mismatch — the function
/// never raises.
#[must_use]
#[allow(clippy::too_many_lines)] // two-pass walker matching the Python layout
pub fn ingest_scip_json(doc: &Value, source_file: &str, language: &str) -> Value {
    let mut nodes: Vec<Value> = Vec::new();
    let mut edges: Vec<Value> = Vec::new();
    let mut seen_node_ids: HashSet<String> = HashSet::new();
    let mut seen_edges: HashSet<(String, String, String, String)> = HashSet::new();

    let Some(map) = doc.as_object() else {
        return json!({"nodes": nodes, "edges": edges});
    };
    let Some(documents) = map.get("documents").and_then(Value::as_array) else {
        return json!({"nodes": nodes, "edges": edges});
    };

    // Pass 1: build symbol → node_id indices.
    let mut per_doc_index: IndexMap<(String, String), String> = IndexMap::new();
    let mut global_index: IndexMap<String, Vec<String>> = IndexMap::new();
    let mut symbol_records: Vec<SymbolRecord> = Vec::new();

    for document in documents {
        let Some(doc_obj) = document.as_object() else {
            continue;
        };
        let doc_path = coerce_str(doc_obj.get("relative_path"), source_file);
        let doc_language = coerce_str(doc_obj.get("language"), language);
        let Some(symbols) = doc_obj.get("symbols").and_then(Value::as_array) else {
            continue;
        };
        for symbol in symbols {
            let Some(sym_obj) = symbol.as_object() else {
                continue;
            };
            let symbol_id = coerce_str(sym_obj.get("symbol"), "");
            if symbol_id.is_empty() {
                continue;
            }
            let node_id = make_scip_node_id(&symbol_id, &doc_path);
            per_doc_index
                .entry((symbol_id.clone(), doc_path.clone()))
                .or_insert_with(|| node_id.clone());
            let candidates = global_index.entry(symbol_id.clone()).or_default();
            if !candidates.contains(&node_id) {
                candidates.push(node_id.clone());
            }
            symbol_records.push(SymbolRecord {
                node_id,
                symbol_id,
                doc_path: doc_path.clone(),
                language: doc_language.clone(),
                raw: sym_obj.clone(),
            });
        }
    }

    // Pass 2: emit nodes + relationships.
    for record in &symbol_records {
        emit_symbol_node(record, &mut nodes, &mut seen_node_ids);
        emit_relationships(
            record,
            &per_doc_index,
            &global_index,
            &mut nodes,
            &mut edges,
            &mut seen_node_ids,
            &mut seen_edges,
        );
    }

    json!({"nodes": nodes, "edges": edges})
}

struct SymbolRecord {
    node_id: String,
    symbol_id: String,
    doc_path: String,
    language: String,
    raw: Map<String, Value>,
}

fn emit_symbol_node(
    record: &SymbolRecord,
    nodes: &mut Vec<Value>,
    seen_node_ids: &mut HashSet<String>,
) {
    if seen_node_ids.contains(&record.node_id) {
        return;
    }
    let kind = coerce_str(record.raw.get("kind"), "unknown");
    let display_name = coerce_str(record.raw.get("display_name"), "");
    let description = record
        .raw
        .get("documentation")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let sourceline = first_occurrence_line(record.raw.get("occurrences"));
    let suffix = record
        .symbol_id
        .split('#')
        .next_back()
        .unwrap_or(&record.symbol_id);
    let label = if !display_name.is_empty() {
        display_name
    } else if !suffix.is_empty() {
        suffix.to_string()
    } else {
        record.symbol_id.clone()
    };
    let source_location = if sourceline > 0 {
        format!("L{sourceline}")
    } else {
        String::new()
    };
    let meta_map: Map<String, Value> = build_scip_metadata(&record.symbol_id, &kind, &description);
    seen_node_ids.insert(record.node_id.clone());
    let _ = record.language; // language is captured for symmetry with Python; not emitted yet
    nodes.push(json!({
        "id": record.node_id,
        "label": label,
        "file_type": scip_kind_to_file_type(&kind),
        "source_file": record.doc_path,
        "source_location": source_location,
        "metadata": Value::Object(sanitize_metadata(Some(&meta_map))),
    }));
}

#[allow(clippy::too_many_arguments)] // mirrors Python signature; refactor would obscure parity
fn emit_relationships(
    record: &SymbolRecord,
    per_doc_index: &IndexMap<(String, String), String>,
    global_index: &IndexMap<String, Vec<String>>,
    nodes: &mut Vec<Value>,
    edges: &mut Vec<Value>,
    seen_node_ids: &mut HashSet<String>,
    seen_edges: &mut HashSet<(String, String, String, String)>,
) {
    let source_node_id = record.node_id.clone();
    let doc_path = record.doc_path.clone();
    let sourceline = first_occurrence_line(record.raw.get("occurrences"));
    let Some(relationships) = record.raw.get("relationships").and_then(Value::as_array) else {
        return;
    };
    for rel in relationships {
        let Some(rel_obj) = rel.as_object() else {
            continue;
        };
        let target_symbol = coerce_str(rel_obj.get("symbol"), "");
        if target_symbol.is_empty() {
            continue;
        }
        let mut target_node_id =
            resolve_relationship_target(&target_symbol, &doc_path, per_doc_index, global_index);
        if target_node_id.is_none() {
            let stub_id = make_scip_node_id(&target_symbol, &doc_path);
            if !seen_node_ids.contains(&stub_id) {
                seen_node_ids.insert(stub_id.clone());
                let suffix = target_symbol
                    .split('#')
                    .next_back()
                    .unwrap_or(&target_symbol);
                let label = if suffix.is_empty() {
                    target_symbol.clone()
                } else {
                    suffix.to_string()
                };
                let stub_meta = build_scip_metadata(&target_symbol, "external", "");
                nodes.push(json!({
                    "id": stub_id,
                    "label": label,
                    "file_type": "code",
                    "source_file": doc_path,
                    "source_location": "",
                    "metadata": Value::Object(sanitize_metadata(Some(&stub_meta))),
                }));
            }
            target_node_id = Some(stub_id);
        }
        let target_node_id = target_node_id.unwrap_or_default();
        let relation = scip_relation_for(rel_obj);
        let source_location = if sourceline > 0 {
            format!("L{sourceline}")
        } else {
            String::new()
        };
        let key = (
            source_node_id.clone(),
            target_node_id.clone(),
            relation.clone(),
            source_location.clone(),
        );
        if seen_edges.contains(&key) {
            continue;
        }
        seen_edges.insert(key);
        let mut edge_meta = Map::new();
        edge_meta.insert(
            "scip_relationship".to_string(),
            Value::Object(rel_obj.clone()),
        );
        edges.push(json!({
            "source": source_node_id,
            "target": target_node_id,
            "relation": relation,
            "confidence": "EXTRACTED",
            "confidence_score": 1.0,
            "source_file": doc_path,
            "source_location": source_location,
            "weight": 1.0,
            "context": "scip",
            "metadata": Value::Object(sanitize_metadata(Some(&edge_meta))),
        }));
    }
}

#[must_use]
fn resolve_relationship_target(
    target_symbol: &str,
    source_doc_path: &str,
    per_doc_index: &IndexMap<(String, String), String>,
    global_index: &IndexMap<String, Vec<String>>,
) -> Option<String> {
    if let Some(id) = per_doc_index.get(&(target_symbol.to_string(), source_doc_path.to_string())) {
        return Some(id.clone());
    }
    let candidates = global_index.get(target_symbol)?;
    if candidates.len() == 1 {
        return Some(candidates[0].clone());
    }
    None
}

#[must_use]
fn is_strictly_true(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Bool(true)))
}

#[must_use]
fn scip_relation_for(rel: &Map<String, Value>) -> String {
    if is_strictly_true(rel.get("is_implementation")) {
        return "scip_impl".to_string();
    }
    if is_strictly_true(rel.get("is_type_definition")) {
        return "scip_typed".to_string();
    }
    if is_strictly_true(rel.get("is_definition")) {
        return "scip_def".to_string();
    }
    "scip_ref".to_string()
}

fn first_occurrence_line(occurrences: Option<&Value>) -> i64 {
    let Some(arr) = occurrences.and_then(Value::as_array) else {
        return 0;
    };
    let Some(first) = arr.first().and_then(Value::as_object) else {
        return 0;
    };
    let Some(rng) = first.get("range").and_then(Value::as_array) else {
        return 0;
    };
    let Some(line) = rng.first() else {
        return 0;
    };
    // Booleans and negatives are rejected so a malformed range cannot produce
    // `LTrue` or `L-5` source locations. Floats are also rejected.
    if let Value::Number(n) = line
        && let Some(i) = n.as_i64()
        && i >= 0
    {
        return i;
    }
    0
}

#[must_use]
fn coerce_str(value: Option<&Value>, default: &str) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        _ => default.to_string(),
    }
}

#[allow(clippy::expect_used)] // literal pattern; cannot fail at runtime
static IDENT_SAFE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-zA-Z0-9_]").expect("static ident-safe regex"));

/// Derive a stable graphify node ID from a SCIP symbol identifier.
///
/// Uses SHA-256 truncated to 12 hex chars for parity with the Python
/// reference's 12-char SHA-1 prefix (which it explicitly notes is an
/// identifier, not a security boundary). Switching to SHA-256 in Rust
/// avoids requiring the deprecated `sha1` crate while preserving the
/// short-suffix collision profile (48 bits of identifier space).
#[must_use]
pub fn make_scip_node_id(symbol: &str, source_file: &str) -> String {
    let raw = format!("{source_file}:{symbol}");
    let digest = Sha256::digest(raw.as_bytes());
    let hex_full = hex::encode(digest);
    let h = &hex_full[..12];
    let suffix_raw = symbol.split('#').next_back().unwrap_or(symbol);
    let suffix = IDENT_SAFE
        .replace_all(suffix_raw, "_")
        .trim_matches('_')
        .to_lowercase();
    if suffix.is_empty() {
        format!("scip_{h}")
    } else {
        format!("scip_{suffix}_{h}")
    }
}

#[must_use]
fn scip_kind_to_file_type(_kind: &str) -> &'static str {
    "code"
}

#[must_use]
fn build_scip_metadata(symbol_id: &str, kind: &str, description: &str) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert(
        "scip_symbol".to_string(),
        Value::String(symbol_id.to_string()),
    );
    map.insert("scip_kind".to_string(), Value::String(kind.to_string()));
    if !description.is_empty() {
        map.insert(
            "scip_description".to_string(),
            Value::String(description.to_string()),
        );
    }
    map
}
