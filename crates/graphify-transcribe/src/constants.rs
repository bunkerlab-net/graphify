//! Public and private constants for the transcription module.

/// Video/audio file extensions recognised by this module.
pub const VIDEO_EXTENSIONS: &[&str] = &[
    ".mp4", ".mov", ".webm", ".mkv", ".avi", ".m4v", ".mp3", ".wav", ".m4a", ".ogg",
];

/// Prefixes that mark a string as a URL rather than a local file path.
pub(crate) const URL_PREFIXES: &[&str] = &["http://", "https://", "www."];

/// Whisper model used when `GRAPHIFY_WHISPER_MODEL` is unset.
pub(crate) const DEFAULT_MODEL: &str = "base";

/// Default directory transcripts are written into.
pub(crate) const TRANSCRIPTS_DIR: &str = "graphify-out/transcripts";

/// Prompt passed to whisper when no other prompt is available.
pub(crate) const FALLBACK_PROMPT: &str = "Use proper punctuation and paragraph breaks.";
