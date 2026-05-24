//! Schema validation for graphify extraction-stage JSON output.
//!
//! The extraction pipeline produces a JSON object with two top-level arrays:
//! `nodes` and `edges` (the `links` alias from older `NetworkX` output is also
//! accepted for `edges`).  This crate checks that the object conforms to the
//! graphify schema — required fields are present, `file_type` and `confidence`
//! values are from the accepted sets, and every edge endpoint refers to a
//! declared node id.
//!
//! # Entry points
//!
//! - [`validate_extraction`] — returns a `Vec<String>` of error messages;
//!   empty means valid.
//! - [`assert_valid`] — returns `Ok(())` or a [`ValidationError`] that
//!   aggregates all messages for display.
//!
//! # Schema constants
//!
//! [`VALID_FILE_TYPES`], [`VALID_CONFIDENCES`], [`REQUIRED_NODE_FIELDS`], and
//! [`REQUIRED_EDGE_FIELDS`] are exposed so callers can generate user-facing
//! documentation or build their own validators without duplicating the allowed
//! value lists.
//!
//! Ports `graphify-py/graphify/validate.py`.

mod error;
mod schema;
mod validate;

pub use error::ValidationError;
pub use schema::{REQUIRED_EDGE_FIELDS, REQUIRED_NODE_FIELDS, VALID_CONFIDENCES, VALID_FILE_TYPES};
pub use validate::{assert_valid, validate_extraction};
