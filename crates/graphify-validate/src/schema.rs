//! Constants describing the extraction schema (accepted values and
//! required fields).

/// File type values accepted by the extraction schema.
pub const VALID_FILE_TYPES: &[&str] =
    &["code", "document", "paper", "image", "rationale", "concept"];

/// Confidence values accepted by the extraction schema.
pub const VALID_CONFIDENCES: &[&str] = &["EXTRACTED", "INFERRED", "AMBIGUOUS"];

/// Required fields on each node object.
pub const REQUIRED_NODE_FIELDS: &[&str] = &["id", "label", "file_type", "source_file"];

/// Required fields on each edge object.
pub const REQUIRED_EDGE_FIELDS: &[&str] =
    &["source", "target", "relation", "confidence", "source_file"];
