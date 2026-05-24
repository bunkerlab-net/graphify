//! Wiki/Obsidian vault export.
//!
//! Ports `graphify-py/graphify/wiki.py`. Generates `.md` files per community
//! and per god node, plus an `index.md` entry point.

mod error;
mod generate;
mod render;
mod types;
mod util;

pub use error::WikiError;
pub use generate::to_wiki;
pub use types::GodNodeData;
