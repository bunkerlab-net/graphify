//! Per-language inheritance-edge emitters.
//!
//! Each `emit_*_inheritance` function is called from the structural `walk`
//! pass when a class node is encountered for the corresponding language.
//! They inspect language-specific child nodes (e.g. `base_list`, `superclass`,
//! `base_class_clause`) and push `inherits` / `extends` / `implements` edges.
//!
//! One submodule per language; this file holds the shared `emit_base_node`.

use std::collections::HashSet;

use crate::types::Node as GNode;

mod cpp;
mod csharp;
mod java;
mod kotlin;
mod php;
mod scala;
mod swift;
mod ts;

pub(crate) use cpp::*;
pub(crate) use csharp::*;
pub(crate) use java::*;
pub(crate) use kotlin::*;
pub(crate) use php::*;
pub(crate) use scala::*;
pub(crate) use swift::*;
pub(crate) use ts::*;

/// Ensure a base-class node exists and return its NID.
pub(crate) fn emit_base_node(
    base: &str,
    _line: u32,
    stem: &str,
    _str_path: &str,
    nodes: &mut Vec<GNode>,
    seen_ids: &mut HashSet<String>,
) -> String {
    use crate::ids::{make_id, make_id1};

    let nid1 = make_id(&[stem, base]);
    if seen_ids.contains(&nid1) {
        return nid1;
    }
    let nid2 = make_id1(base);
    if !seen_ids.contains(&nid2) {
        nodes.push(GNode {
            id: nid2.clone(),
            label: base.to_string(),
            file_type: "code".to_string(),
            source_file: String::new(),
            source_location: None,
            metadata: None,
        });
        seen_ids.insert(nid2.clone());
    }
    nid2
}
