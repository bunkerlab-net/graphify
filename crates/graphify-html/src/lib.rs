//! HTML generators for graphify knowledge graphs.
//!
//! Ports `graphify-py/graphify/tree_html.py` and
//! `graphify-py/graphify/callflow_html.py` into a single crate with two
//! public sub-modules:
//!
//! * [`tree`] — D3 v7 collapsible-tree view of the file hierarchy.
//! * [`callflow`] — Mermaid architecture / call-flow documentation page.

use thiserror::Error;

pub mod callflow;
pub mod tree;

/// Errors produced by the HTML generators.
#[derive(Debug, Error)]
pub enum HtmlError {
    /// An I/O error when reading or writing files.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// The graph contained no nodes.
    #[error("graph.json contains 0 nodes")]
    EmptyGraph,

    /// No sections could be derived from the graph.
    #[error("no sections defined")]
    NoSections,
}
