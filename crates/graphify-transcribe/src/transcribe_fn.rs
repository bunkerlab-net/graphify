//! Top-level [`transcribe`] / [`transcribe_all`] drivers.

use std::path::{Path, PathBuf};

use crate::constants::{DEFAULT_MODEL, FALLBACK_PROMPT, TRANSCRIPTS_DIR};
use crate::download::download_audio_with;
use crate::error::TranscribeError;
use crate::util::is_url;
use crate::whisper::{WhisperCliRunner, WhisperRunner};
use crate::ytdlp::{YtDlpCliRunner, YtDlpRunner};

/// Transcribe a video/audio file or URL to a `.txt` transcript.
///
/// - If `video_path` is a URL, audio is downloaded first via `yt-dlp`.
/// - Returns the cached transcript immediately unless `force` is `true`.
/// - `initial_prompt` overrides the default fallback prompt passed to
///   whisper.
/// - The whisper model is taken from `GRAPHIFY_WHISPER_MODEL` if set,
///   else defaults to `"base"`.
///
/// # Errors
///
/// Returns [`TranscribeError`] on download, binary, or I/O failure.
pub fn transcribe(
    video_path: impl AsRef<Path>,
    output_dir: Option<&Path>,
    initial_prompt: Option<&str>,
    force: bool,
) -> Result<PathBuf, TranscribeError> {
    transcribe_with(
        video_path,
        output_dir,
        initial_prompt,
        force,
        &WhisperCliRunner,
        &YtDlpCliRunner,
    )
}

/// Like [`transcribe`] but accepts injected runners for testing.
///
/// # Errors
///
/// Returns [`TranscribeError`] on download, binary, or I/O failure.
pub fn transcribe_with(
    video_path: impl AsRef<Path>,
    output_dir: Option<&Path>,
    initial_prompt: Option<&str>,
    force: bool,
    whisper: &dyn WhisperRunner,
    ytdlp: &dyn YtDlpRunner,
) -> Result<PathBuf, TranscribeError> {
    let out_dir = output_dir.map_or_else(|| PathBuf::from(TRANSCRIPTS_DIR), Path::to_path_buf);
    std::fs::create_dir_all(&out_dir)?;

    let path_str = video_path.as_ref().to_string_lossy().into_owned();
    let audio_path = if is_url(&path_str) {
        let downloads_dir = out_dir.join("downloads");
        download_audio_with(&path_str, &downloads_dir, ytdlp)?
    } else {
        video_path.as_ref().to_path_buf()
    };

    // Fall back to `transcript` when the audio file has no stem (e.g.
    // a hidden file like `.opus`) so we don't write `.txt` with an
    // empty stem.
    let stem = audio_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "transcript".to_string());
    let transcript_path = out_dir.join(format!("{stem}.txt"));

    if transcript_path.exists() && !force {
        return Ok(transcript_path);
    }

    let model_name =
        std::env::var("GRAPHIFY_WHISPER_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    let prompt = initial_prompt.unwrap_or(FALLBACK_PROMPT);

    let text = whisper.run(&audio_path, prompt, &model_name)?;

    std::fs::write(&transcript_path, &text)?;
    Ok(transcript_path)
}

/// Transcribe a list of video/audio files or URLs; failures are logged
/// and skipped (matches the Python reference).
///
/// Returns a list of paths to the produced transcript `.txt` files.
#[must_use]
pub fn transcribe_all(
    video_files: &[&str],
    output_dir: Option<&Path>,
    initial_prompt: Option<&str>,
) -> Vec<String> {
    if video_files.is_empty() {
        return Vec::new();
    }

    let mut results = Vec::new();
    for vf in video_files {
        match transcribe(vf, output_dir, initial_prompt, false) {
            Ok(p) => results.push(p.to_string_lossy().into_owned()),
            Err(e) => {
                eprintln!("  warning: could not transcribe {vf}: {e}");
            }
        }
    }
    results
}
