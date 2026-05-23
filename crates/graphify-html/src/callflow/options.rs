//! Public data types and the `CallflowOptions` configuration struct.
//!
//! Extracted so that callers can import `CallflowOptions` (and the lower-level
//! `Node` / `CfEdge` / `Section` types) without pulling in the heavier HTML
//! rendering or template machinery.

use std::path::PathBuf;

/// A lightweight normalized graph node used across all callflow helpers.
#[derive(Debug, Clone)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub community: String,
    pub source_file: String,
    pub node_type: String,
    pub file_type: String,
}

/// A lightweight normalized graph edge.
#[derive(Debug, Clone)]
pub struct CfEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub relation: String,
    pub confidence: String,
    pub confidence_score: f64,
}

/// A section definition (id + name + communities).
#[derive(Debug, Clone)]
pub struct Section {
    pub id: String,
    pub name: String,
    pub communities: Vec<String>,
}

/// Options for [`crate::callflow::write_callflow_html`].
#[derive(Debug, Clone)]
pub struct CallflowOptions {
    pub project: Option<PathBuf>,
    pub graphify_out: Option<PathBuf>,
    pub graph: Option<PathBuf>,
    pub report: Option<PathBuf>,
    pub labels: Option<PathBuf>,
    pub sections: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub lang: String,
    pub max_sections: usize,
    pub diagram_scale: f64,
    pub max_diagram_nodes: usize,
    pub max_diagram_edges: usize,
}

impl Default for CallflowOptions {
    /// Returns a `CallflowOptions` with sensible defaults: auto language detection,
    /// up to 15 sections, and diagram scale of 1.0.
    fn default() -> Self {
        Self {
            project: None,
            graphify_out: None,
            graph: None,
            report: None,
            labels: None,
            sections: None,
            output: None,
            lang: "auto".to_owned(),
            max_sections: 15,
            diagram_scale: 1.0,
            max_diagram_nodes: 18,
            max_diagram_edges: 24,
        }
    }
}
