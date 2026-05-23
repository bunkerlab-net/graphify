//! Schema validation for graphify extraction JSON.
//!
//! Ports `graphify-py/graphify/validate.py`. See the module docstring
//! there.

mod error;
mod schema;
mod validate;

pub use error::ValidationError;
pub use schema::{REQUIRED_EDGE_FIELDS, REQUIRED_NODE_FIELDS, VALID_CONFIDENCES, VALID_FILE_TYPES};
pub use validate::{assert_valid, validate_extraction};
