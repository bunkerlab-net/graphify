//! Git hook integration — install/uninstall/status graphify post-commit and
//! post-checkout hooks, plus per-platform install/uninstall functions.
//!
//! Ports `graphify-py/graphify/hooks.py` and the platform install functions
//! from `graphify-py/graphify/__main__.py`.

mod constants;
mod error;
mod git;
mod install;
pub mod platform;
mod status;
mod uninstall;

pub use constants::{
    CHECKOUT_MARKER, CHECKOUT_MARKER_END, CHECKOUT_SCRIPT, HOOK_MARKER, HOOK_MARKER_END,
    HOOK_SCRIPT, PYTHON_DETECT,
};
pub use error::HooksError;
pub use git::{hooks_dir, hooks_dir_with};
pub use install::install;
pub use status::status;
pub use uninstall::uninstall;
