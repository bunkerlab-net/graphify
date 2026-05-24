//! Public data types and the `CallflowOptions` configuration struct.
//!
//! Extracted so that callers can import `CallflowOptions` (and the lower-level
//! `Node` / `CfEdge` / `Section` types) without pulling in the heavier HTML
//! rendering or template machinery.

use std::path::PathBuf;

/// A lightweight normalized graph node used across all callflow helpers.
#[derive(Debug, Clone)]
pub struct Node {
    /// Unique node identifier from the source graph.
    pub id: String,
    /// Human-readable display label.
    pub label: String,
    /// Community (cluster) this node belongs to.
    pub community: String,
    /// Path to the source file that defines this node.
    pub source_file: String,
    /// Structural kind of the node (e.g. `"class"`, `"function"`, `"module"`).
    pub node_type: String,
    /// Content kind of the node (e.g. `"code"`, `"document"`, `"rationale"`).
    pub file_type: String,
}

/// A lightweight normalized graph edge.
#[derive(Debug, Clone)]
pub struct CfEdge {
    /// Unique edge identifier.
    pub id: String,
    /// Source node id.
    pub source: String,
    /// Target node id.
    pub target: String,
    /// Relation type (e.g. `"calls"`, `"imports"`, `"uses"`).
    pub relation: String,
    /// Confidence class: one of `"EXTRACTED"`, `"INFERRED"`, or `"AMBIGUOUS"`.
    pub confidence: String,
    /// Numeric confidence in `[0, 1]`; used to filter low-quality INFERRED edges.
    pub confidence_score: f64,
}

/// A section definition (id + name + communities).
#[derive(Debug, Clone)]
pub struct Section {
    /// Stable HTML anchor id for this section.
    pub id: String,
    /// Human-readable section title shown in navigation and headings.
    pub name: String,
    /// Community ids whose nodes belong to this section.
    pub communities: Vec<String>,
}

/// Options for [`crate::callflow::write_callflow_html`].
#[derive(Debug, Clone)]
pub struct CallflowOptions {
    /// Root directory of the project being documented.
    pub project: Option<PathBuf>,
    /// Explicit path to the `graphify-out/` directory. Inferred from `project` when absent.
    pub graphify_out: Option<PathBuf>,
    /// Explicit path to `graph.json`. Inferred from `graphify_out` when absent.
    pub graph: Option<PathBuf>,
    /// Explicit path to `GRAPH_REPORT.md`. Inferred from `graphify_out` when absent.
    pub report: Option<PathBuf>,
    /// Explicit path to the community-labels JSON file. Inferred when absent.
    pub labels: Option<PathBuf>,
    /// Explicit path to a sections JSON file. When absent, sections are derived automatically.
    pub sections: Option<PathBuf>,
    /// Output path for the generated HTML file. Derived from the project name when absent.
    pub output: Option<PathBuf>,
    /// BCP 47 language tag (`"en"`, `"zh-CN"`, or `"auto"` for detection).
    pub lang: String,
    /// Maximum number of architecture sections to render (including the overview).
    pub max_sections: usize,
    /// Mermaid diagram scale factor; clamped to `[0.65, 1.8]`.
    pub diagram_scale: f64,
    /// Maximum nodes rendered per section diagram.
    pub max_diagram_nodes: usize,
    /// Maximum edges rendered per section diagram.
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
