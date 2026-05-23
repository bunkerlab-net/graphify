//! Audio download entry points.

use std::path::{Path, PathBuf};

use crate::error::TranscribeError;
use crate::util::url_hash_prefix;
use crate::ytdlp::{YtDlpCliRunner, YtDlpRunner};

/// Download the audio-only stream from a URL using `yt-dlp`.
///
/// Uses a stable filename derived from the SHA-1 hash of the URL
/// (`yt_<first12hexchars>.<ext>`). Returns immediately if the file
/// already exists (any of `.m4a`, `.opus`, `.mp3`, `.ogg`, `.wav`,
/// `.webm`).
///
/// # Errors
///
/// Propagates URL-validation failures, missing-binary errors, and I/O
/// errors.
pub fn download_audio(url: &str, output_dir: &Path) -> Result<PathBuf, TranscribeError> {
    download_audio_with(url, output_dir, &YtDlpCliRunner)
}

/// Like [`download_audio`] but accepts an injected [`YtDlpRunner`] so
/// tests can stub the download.
///
/// # Errors
///
/// Returns [`TranscribeError`] on URL validation, binary, or I/O
/// failure.
pub fn download_audio_with(
    url: &str,
    output_dir: &Path,
    runner: &dyn YtDlpRunner,
) -> Result<PathBuf, TranscribeError> {
    graphify_security::validate_url(url)?;

    std::fs::create_dir_all(output_dir)?;

    let hash_prefix = url_hash_prefix(url);
    let out_template = output_dir
        .join(format!("yt_{hash_prefix}.%(ext)s"))
        .to_string_lossy()
        .into_owned();

    for ext in &["m4a", "opus", "mp3", "ogg", "wav", "webm"] {
        let candidate = output_dir.join(format!("yt_{hash_prefix}.{ext}"));
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    runner.download(url, &out_template, output_dir, &hash_prefix)
}
