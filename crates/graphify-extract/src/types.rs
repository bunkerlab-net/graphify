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
    /// Referencing file for a SOURCELESS cross-file stub (#1462): the file whose
    /// reference created this placeholder. Disambiguates same-label stubs from
    /// different files during id-collision splitting, while `source_file` stays
    /// empty so a real project definition can still be rewired onto it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_file: Option<String>,
    /// Optional node category serialised as `"type"` (e.g. a C# `namespace` node,
    /// #1562). Distinct from `file_type` (always `"code"` here). Omitted when None.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub node_type: Option<String>,
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
    #[serde(default, skip_serializing_if = "is_false")]
    /// `true` when the edge points at an unresolved / external target (e.g. a
    /// BYOND `#include` of a file outside the corpus). Serialised only when set,
    /// mirroring graphify-py, which adds the key solely for unresolved includes.
    pub external: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    /// `true` for a deferred `import(...)` dependency (a JS/TS dynamic import):
    /// a real edge kept in the graph, but excluded from file-cycle detection so
    /// a static→dynamic back-import is not reported as a phantom cycle (#1241).
    pub deferred: bool,
    /// Optional extractor-specific edge metadata (e.g. a C# `using` import edge
    /// carries `{"using_kind","target_fqn","alias","scope_kind","scope_id"}`; a
    /// qualified type reference carries `{"qualified","ref_qualifier","ref_token"}`).
    /// Omitted from output when absent (#1562).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<IndexMap<String, Value>>,
}

/// serde `skip_serializing_if` predicate: omit a `bool` field when it is `false`.
#[allow(clippy::trivially_copy_pass_by_ref)] // signature dictated by serde's skip_serializing_if
fn is_false(b: &bool) -> bool {
    !*b
}

/// Which bespoke resolver claims a [`RawCall`]. A `.h` header routes to either
/// the C++ or the Objective-C extractor by content, and both cross-file
/// member-call resolvers activate on `.h`, so the source-file suffix alone can't
/// separate them; the extractor stamps this instead. `None` for every other
/// language, whose resolver gates on the file suffix. Mirrors graphify-py's
/// per-`raw_call` `"lang"` tag (#1547/#1556).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawCallLang {
    /// Emitted by the C++ generic walk (`extract_cpp`).
    Cpp,
    /// Emitted by the Objective-C extractor (`extract_objc`).
    ObjC,
}

/// An unresolved call saved for cross-file resolution.
#[derive(Debug, Clone, Default)]
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
    /// For Swift member calls (`recv.method()`), the depth-1 receiver name used
    /// by cross-file member-call resolution (#1356). `None` for other languages
    /// and non-member calls.
    pub receiver: Option<String>,
    /// For Ruby member calls (`var.method()`), the receiver's inferred type from
    /// local `var = ClassName.new` bindings, when unambiguously known. Lets the
    /// cross-file pass resolve the call by the receiver's *type* rather than by
    /// globally-unique method name (#1499). `None` for other languages, non-member
    /// calls, and receivers whose type is unknown or ambiguous.
    pub receiver_type: Option<String>,
    /// Extractor that produced this call, used to claim `.h` member calls
    /// unambiguously across the C++/ObjC resolvers. `None` for all other
    /// languages (which gate on the file suffix). See [`RawCallLang`].
    pub lang: Option<RawCallLang>,
    /// `true` for an INDIRECT-dispatch reference: a callable named BY NAME (passed
    /// as a call argument, listed in a dispatch table, bound/returned) rather than
    /// invoked. Deferred to the cross-file resolver when the name is defined in
    /// another file; resolves to a distinct INFERRED `indirect_call` edge, never a
    /// `calls` edge, and only when the target is a real callable. `false` for a
    /// normal call. Mirrors graphify-py's `rc["indirect"]` flag (#1565/#1566).
    pub indirect: bool,
    /// Dispatch context for an `indirect` `raw_call`: `"argument"`, `"collection"`,
    /// `"assignment"`, `"return"`, or `"getattr"`. Carried through to the emitted
    /// `indirect_call` edge's `context`. `None` for a normal call.
    pub context: Option<String>,
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
