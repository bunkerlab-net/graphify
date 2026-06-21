//! BYOND `DreamMaker` extractors.
//!
//! Ports the DM section of `graphify-py/graphify/extract.py`:
//! - [`extract_dm`] — `.dm`/`.dme` source via tree-sitter (types, procs,
//!   includes, calls). DM identity is path-based (`/datum/object/proc/New()`),
//!   so this uses a bespoke walk rather than the generic class-body walker.
//! - [`extract_dmi`] — `.dmi` icon sheets (PNG with a BYOND metadata text chunk).
//! - [`extract_dmm`] — `.dmm` map files (tile dictionary type references).
//! - [`extract_dmf`] — `.dmf` interface forms (windows + controls).

mod dmf;
mod dmi;
mod dmm;
mod source;

pub use dmf::extract_dmf;
pub use dmi::extract_dmi;
pub use dmm::extract_dmm;
pub use source::extract_dm;
