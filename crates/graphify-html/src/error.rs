//! Error type for the HTML generators.

use thiserror::Error;

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
