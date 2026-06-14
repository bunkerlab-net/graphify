//! Graph export to multiple output formats.
//!
//! This crate provides serialisers that turn a [`graphify_build::Graph`] into
//! human-readable or tool-consumable representations:
//!
//! - **JSON** (`to_json`) — node-link format, the primary interchange format.
//! - **HTML** (`to_html`) — interactive vis.js visualisation with community
//!   filtering and a search sidebar.
//! - **SVG** (`to_svg`) — static spring-layout visualisation.
//! - **`GraphML`** (`to_graphml`) — XML format understood by Gephi and yEd.
//! - **Cypher** (`to_cypher`) — Neo4j import script.
//! - **Neo4j push** (`push_to_neo4j`) — direct Bolt-protocol upsert.
//! - **Obsidian vault** (`to_obsidian`) — one Markdown note per node plus
//!   community overview notes.
//! - **Obsidian Canvas** (`to_canvas`) — `.canvas` JSON file with a grid
//!   layout of community groups.
//!
//! Utility helpers (colour palettes, YAML escaping, diacritic stripping, …)
//! are exposed via [`util`] and re-exported at the crate root.
//!
//! Ports `graphify-py/graphify/export.py`.

pub mod canvas;
pub mod cypher;
mod error;
pub mod falkordb;
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
pub use falkordb::{FalkorConn, parse_falkordb_uri};
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

/// Backward-compatible alias for [`to_html`].
pub use html::to_html as generate_html;
