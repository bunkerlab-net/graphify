//! URL / PDF / Office document ingestion into corpus.
//!
//! Ports `graphify-py/graphify/ingest.py`.

mod error;
mod fetchers;
mod ingest_fn;
mod memory;
mod regexes;
mod text;

pub use error::IngestError;
pub use ingest_fn::ingest;
pub use memory::save_query_result;
pub use text::{detect_url_type, html_to_markdown, safe_filename, yaml_str};
