//! Backwards-compatible typed views of node and edge attributes.
//!
//! These structs predate the move to `IndexMap<String, Value>` attribute
//! storage. They remain in the public API so older consumers continue to
//! compile; new code should prefer working directly with the
//! `IndexMap<String, Value>` representation that [`crate::Graph`] exposes.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Typed view of the well-known node attribute keys.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeAttrs {
    /// Node ID.
    pub id: String,
    /// Display label.
    pub label: String,
    /// One of the canonical [`crate::file_type`] values.
    pub file_type: String,
    /// Path to the source file, normalised relative to the corpus root.
    pub source_file: String,
    /// Catch-all for additional attributes.
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

/// Typed view of the well-known edge attribute keys.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EdgeAttrs {
    /// Relation name (e.g. `"calls"`, `"imports"`).
    pub relation: String,
    /// Confidence tier (`"EXTRACTED"`, `"INFERRED"`, `"AMBIGUOUS"`).
    pub confidence: String,
    /// Path to the source file, normalised relative to the corpus root.
    pub source_file: String,
    /// Catch-all for additional attributes.
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}
