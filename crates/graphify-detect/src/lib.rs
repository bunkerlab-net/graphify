//! File discovery + filtering for graphify.
//!
//! Ports `graphify-py/graphify/detect.py`. Provides:
//! - Extension-based and content-based file classification
//! - Gitignore-aware directory walking
//! - Sensitive-file and noise-directory filtering
//! - Manifest persistence for incremental detection
//!
//! # Side effects
//!
//! [`save_manifest`] writes to `graphify-out/manifest.json` under the
//! scan root by default. This path is parameterised by the
//! `manifest_path` argument so callers can override it.

pub mod error;
pub mod extensions;
pub mod ignore;
mod incremental;
pub mod manifest;
pub mod office;
pub mod sensitive;
pub mod shebang;
pub mod walk;

pub use error::DetectError;
pub use extensions::{
    CODE_EXTENSIONS, DOC_EXTENSIONS, FileType, GOOGLE_WORKSPACE_EXTENSIONS, IMAGE_EXTENSIONS,
    PACKAGE_MANIFEST_NAMES, PAPER_EXTENSIONS, classify_file, is_package_manifest_path,
};
pub use ignore::{
    IgnoreEvalCache, could_contain_included_path, find_vcs_root, is_ignored, is_ignored_with_cache,
    is_included, load_graphifyignore, load_graphifyinclude, parse_gitignore_line,
};
pub use incremental::{
    IncrementalDetectResult, Manifest, detect_incremental, detect_incremental_with_cache_root,
    load_manifest, save_manifest,
};
pub use manifest::{
    IncrementalOptions, MANIFEST_PATH, ManifestEntry, detect_incremental_with_manifest,
    load_manifest_from_path, load_manifest_from_path_with_root, md5_file, save_manifest_to_path,
    save_manifest_to_path_with_root,
};
pub use sensitive::{SKIP_DIRS, SKIP_FILES, is_noise_dir, is_sensitive};
pub use shebang::{env_command_args, shebang_interpreter};
pub use walk::{
    DetectResult, FILE_TYPE_KINDS, auto_follow_symlinks, collect_files, detect,
    detect_with_cache_root,
};
