//! Ingests external URLs, PDFs, and web content into graphify-ready text
//! fragments.
//!
//! The top-level entry point is [`ingest`], which takes a URL, classifies it
//! (tweet, arXiv paper, PDF, image, `YouTube`, or generic webpage), fetches
//! the content via the security-hardened HTTP client in `graphify-security`,
//! and writes a Markdown file with YAML frontmatter into the caller-supplied
//! target directory.
//!
//! [`save_query_result`] persists a Q&A result into the memory directory so
//! the graph extractor can pick it up on the next `--update` run.
//!
//! Text-shaping utilities ([`yaml_str`], [`safe_filename`],
//! [`detect_url_type`], [`html_to_markdown`]) are re-exported for use by
//! other crates in the workspace that need the same normalisation logic.
//!
//! Ports `graphify-py/graphify/ingest.py`.

mod error;
mod fetchers;
mod ingest_fn;
mod memory;
mod regexes;
mod text;

pub use error::IngestError;
pub use ingest_fn::{ingest, ingest_with};
pub use memory::{OUTCOMES, save_query_result};
pub use text::{detect_url_type, html_to_markdown, safe_filename, yaml_str};
