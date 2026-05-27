//! Core data types: `Node`, `Edge`, `ExtractResult`, `RawCall`.
//!
//! These mirror the Python dict shapes used throughout `extract.py`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A graph node emitted by any extractor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Stable, deterministic node identifier (e.g. `"module::ClassName"`).
    pub id: String,
    /// Human-readable display name (e.g. `"ClassName"` or `"function_name"`).
    pub label: String,
    /// Semantic category of the node (e.g. `"class"`, `"function"`, `"file"`).
    pub file_type: String,
    /// Absolute or repo-relative path of the file this node was extracted from.
    pub source_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_location: Option<String>,
    /// Optional extractor-specific metadata (e.g. MCP config nodes carry
    /// `{"mcp_kind": "mcp_server"}`). Omitted from output when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<IndexMap<String, Value>>,
}

/// A graph edge emitted by any extractor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    /// Node ID of the edge origin.
    pub source: String,
    /// Node ID of the edge destination.
    pub target: String,
    /// Semantic relationship label (e.g. `"calls"`, `"imports"`, `"inherits"`).
    pub relation: String,
    /// Qualitative confidence tier: `"high"`, `"medium"`, or `"low"`.
    pub confidence: String,
    /// Absolute or repo-relative path of the file this edge was extracted from.
    pub source_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_location: Option<String>,
    /// Numeric edge weight in `[0.0, 1.0]`; used for ranking during graph analysis.
    pub weight: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional surrounding source snippet providing call-site context.
    pub context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional numeric confidence in `[0.0, 1.0]` complementing the string tier.
    pub confidence_score: Option<f64>,
}

/// An unresolved call saved for cross-file resolution.
#[derive(Debug, Clone)]
pub struct RawCall {
    /// Node ID of the calling function or method.
    pub caller_nid: String,
    /// Raw callee text as it appears in the source (not yet resolved to a node ID).
    pub callee: String,
    /// `true` if the call is a method call on an object (e.g. `obj.method()`).
    pub is_member_call: bool,
    /// Absolute or repo-relative path of the file containing this call.
    pub source_file: String,
    /// Source location string (e.g. `"file.py:42"`) for traceability.
    pub source_location: String,
}

/// Result of extracting a single file.
#[derive(Debug, Default, Clone)]
pub struct FileResult {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub raw_calls: Vec<RawCall>,
    pub error: Option<String>,
}

impl FileResult {
    /// Construct a `FileResult` carrying only an error message, with all other fields empty.
    #[must_use]
    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            error: Some(msg.into()),
            ..Default::default()
        }
    }
}

/// Final output of the multi-file `extract()` call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractOutput {
    /// Deduplicated graph nodes, each serialised as a JSON object.
    pub nodes: Vec<IndexMap<String, Value>>,
    /// Graph edges after cross-file import resolution, each serialised as a JSON object.
    pub edges: Vec<IndexMap<String, Value>>,
    /// Estimated LLM input token count (reserved for future LLM-assisted extraction).
    pub input_tokens: u64,
    /// Estimated LLM output token count (reserved for future LLM-assisted extraction).
    pub output_tokens: u64,
}
