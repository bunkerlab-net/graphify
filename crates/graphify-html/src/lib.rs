//! HTML generators for graphify knowledge graphs.
//!
//! Ports `graphify-py/graphify/tree_html.py` and
//! `graphify-py/graphify/callflow_html.py` into a single crate with two
//! public sub-modules:
//!
//! * [`tree`] — D3 v7 collapsible-tree view of the file hierarchy.
//! * [`callflow`] — Mermaid architecture / call-flow documentation page.

pub mod callflow;
mod error;
pub mod tree;

pub use error::HtmlError;
