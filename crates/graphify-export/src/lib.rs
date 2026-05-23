//! Graph export to JSON / HTML / SVG / Obsidian vault / Canvas /
//! `GraphML` / Cypher.
//!
//! Ports `graphify-py/graphify/export.py`.

pub mod canvas;
pub mod cypher;
mod error;
pub mod graphml;
pub mod html;
pub mod json;
pub mod neo4j;
pub mod obsidian;
pub mod svg;
mod util;

pub use canvas::to_canvas;
pub use cypher::{cypher_escape, cypher_escape_identifier, cypher_label, to_cypher};
pub use error::ExportError;
pub use graphml::to_graphml;
pub use html::to_html;
pub use json::{attach_hyperedges, backup_if_protected, prune_dangling_edges, to_json};
pub use neo4j::{Neo4jError, push_to_neo4j, push_to_neo4j_blocking};
pub use obsidian::to_obsidian;
pub use svg::to_svg;
pub use util::{
    BACKUP_ARTIFACTS, COMMUNITY_COLORS, MAX_NODES_FOR_VIZ, confidence_score, node_community_map,
    obsidian_tag, strip_diacritics, viz_node_limit, yaml_str,
};

// Backward-compatible alias.
pub use html::to_html as generate_html;
