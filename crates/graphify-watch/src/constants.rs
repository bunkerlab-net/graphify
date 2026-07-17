//! File-extension constants used by the watcher.

use std::sync::LazyLock;

use graphify_detect::{CODE_EXTENSIONS, DOC_EXTENSIONS, IMAGE_EXTENSIONS, PAPER_EXTENSIONS};

/// All extensions the watcher pays attention to (code + doc + paper + image).
///
/// Corresponds to `_WATCHED_EXTENSIONS` in the Python reference, which is
/// literally `CODE_EXTENSIONS | DOC_EXTENSIONS | PAPER_EXTENSIONS |
/// IMAGE_EXTENSIONS` (watch.py). Composed here from the same four authoritative
/// `graphify_detect` slices so a new extension registered in detect propagates
/// automatically instead of drifting from a hand-maintained copy. Elements are
/// bare extensions **without** a leading dot.
pub static WATCHED_EXTENSIONS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    CODE_EXTENSIONS
        .iter()
        .chain(DOC_EXTENSIONS)
        .chain(PAPER_EXTENSIONS)
        .chain(IMAGE_EXTENSIONS)
        .copied()
        .collect()
});
