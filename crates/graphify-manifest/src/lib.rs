//! Backwards-compat re-export of manifest helpers from [`graphify_detect`].
//!
//! Ports `graphify-py/graphify/manifest.py`, which simply re-exports
//! `save_manifest`, `load_manifest`, and `detect_incremental` from `detect.py`.

pub use graphify_detect::{
    IncrementalDetectResult, detect_incremental, load_manifest, save_manifest,
};
