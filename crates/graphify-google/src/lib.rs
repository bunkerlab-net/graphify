//! Google Workspace shortcut export support.
//!
//! Google Drive for desktop stores native Docs, Sheets, and Slides as small
//! JSON shortcut files (`.gdoc`, `.gsheet`, `.gslides`). Those files are
//! pointers, not document content. This module exports them to Markdown
//! sidecars via the `gws` CLI so Graphify can extract their actual
//! contents.
//!
//! Ports `graphify-py/graphify/google_workspace.py`.

mod convert;
mod error;
mod gws;
mod shortcut;

pub use convert::convert_google_workspace_file;
pub use error::GoogleError;
pub use gws::run_gws_export;
pub use shortcut::{
    GOOGLE_WORKSPACE_EXTENSIONS, ShortcutMetadata, google_workspace_enabled, read_google_shortcut,
};
