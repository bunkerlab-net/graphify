//! Core data types: `Node`, `Edge`, `ExtractResult`, `RawCall`.
//!
//! These mirror the Python dict shapes used throughout `extract.py`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A graph node emitted by any extractor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub file_type: String,
    pub source_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_location: Option<String>,
}

/// A graph edge emitted by any extractor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub confidence: String,
    pub source_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_location: Option<String>,
    pub weight: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_score: Option<f64>,
}

/// An unresolved call saved for cross-file resolution.
#[derive(Debug, Clone)]
pub struct RawCall {
    pub caller_nid: String,
    pub callee: String,
    pub is_member_call: bool,
    pub source_file: String,
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
    pub nodes: Vec<IndexMap<String, Value>>,
    pub edges: Vec<IndexMap<String, Value>>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}
