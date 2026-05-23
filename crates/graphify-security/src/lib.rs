//! Security helpers — URL validation, safe fetch, path guards, label sanitisation.
//!
//! Ports `graphify-py/graphify/security.py`.

mod error;
mod fetch;
mod ip;
mod label;
mod path_guard;
#[doc(hidden)]
pub mod test_support;
mod url_guard;

pub use error::SecurityError;
pub use fetch::{MAX_FETCH_BYTES, MAX_TEXT_BYTES, safe_fetch, safe_fetch_text};
pub use label::sanitize_label;
pub use path_guard::validate_graph_path;
pub use url_guard::validate_url;
