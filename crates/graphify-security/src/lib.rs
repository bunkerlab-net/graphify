//! Security helpers for graphify: SSRF-protective URL and IP validation,
//! safe HTTP fetch with redirect re-validation and size caps, path-traversal
//! protection for `graphify-out/` files, and label sanitisation for text
//! embedded in HTML or JSON.
//!
//! # Overview
//!
//! - **URL / IP validation** ([`validate_url`]): enforces an http/https
//!   scheme allowlist, rejects known cloud-metadata endpoints, and blocks
//!   private, loopback, link-local, CGN, NAT64-embedded, and other reserved
//!   IP ranges to prevent SSRF attacks.
//! - **Safe fetch** ([`safe_fetch`], [`safe_fetch_text`]): wraps `ureq` with
//!   SSRF checks on every redirect hop and a hard byte-cap on the response
//!   body.
//! - **Path guard** ([`validate_graph_path`]): resolves a path and asserts it
//!   stays inside the `graphify-out/` base directory, preventing directory
//!   traversal reads.
//! - **Label sanitisation** ([`sanitize_label`]): strips C0/C1 control
//!   characters and JavaScript line terminators, then truncates to 256 chars,
//!   making free-form text safe for JSON-in-`<script>` embedding.
//!
//! Ports `graphify-py/graphify/security.py`.

mod error;
mod fetch;
mod graph_size;
mod ip;
mod label;
mod metadata;
mod path_guard;
pub mod paths;
#[doc(hidden)]
pub mod test_support;
mod url_guard;

pub use error::SecurityError;
pub use fetch::{MAX_FETCH_BYTES, MAX_TEXT_BYTES, safe_fetch, safe_fetch_text};
pub use graph_size::{
    MAX_GRAPH_FILE_BYTES, check_graph_file_size_cap, check_graph_file_size_cap_with,
    max_graph_file_bytes,
};
pub use label::sanitize_label;
pub use metadata::{
    METADATA_MAX_LIST_ITEMS, METADATA_MAX_VALUE_LEN, sanitize_metadata, sanitize_metadata_map,
    sanitize_metadata_string, sanitize_metadata_value,
};
pub use path_guard::validate_graph_path;
pub use paths::{DEFAULT_GRAPHIFY_OUT, default_graph_json, graphify_out, graphify_out_name};
pub use url_guard::validate_url;
