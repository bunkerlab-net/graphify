//! Video/audio transcription via `whisper-cli` and `yt-dlp` shell-outs.
//!
//! Ports `graphify-py/graphify/transcribe.py`.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha1::{Digest, Sha1};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Video/audio file extensions recognised by this module.
pub const VIDEO_EXTENSIONS: &[&str] = &[
    ".mp4", ".mov", ".webm", ".mkv", ".avi", ".m4v", ".mp3", ".wav", ".m4a", ".ogg",
];

const URL_PREFIXES: &[&str] = &["http://", "https://", "www."];
const DEFAULT_MODEL: &str = "base";
const TRANSCRIPTS_DIR: &str = "graphify-out/transcripts";
const FALLBACK_PROMPT: &str = "Use proper punctuation and paragraph breaks.";

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by transcription and audio-download operations.
#[derive(Debug, Error)]
pub enum TranscribeError {
    /// A required binary (`whisper-cli` or `yt-dlp`) was not found on PATH.
    #[error("Required binary '{binary}' not found on PATH")]
    BinaryMissing { binary: String },

    /// The binary exited with a non-zero status.
    #[error("'{binary}' failed (exit {code}): {stderr}")]
    BinaryFailed {
        binary: String,
        code: i32,
        stderr: String,
    },

    /// URL validation failed (SSRF guard, bad scheme, etc.).
    #[error("URL validation failed: {0}")]
    InvalidUrl(#[from] graphify_security::SecurityError),

    /// A filesystem I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Transcript output was not found after the whisper-cli run.
    #[error("whisper-cli did not produce expected transcript at {path}")]
    OutputMissing { path: PathBuf },
}

// ---------------------------------------------------------------------------
// WhisperRunner trait
// ---------------------------------------------------------------------------

/// Abstraction over the `whisper-cli` invocation.
///
/// The default implementation ([`WhisperCliRunner`]) shells out to the real
/// binary. Tests may inject a [`MockWhisperRunner`] instead.
pub trait WhisperRunner {
    /// Run transcription on `audio`, returning the transcript text.
    ///
    /// # Errors
    ///
    /// Returns `TranscribeError::BinaryMissing` when the binary is absent,
    /// `TranscribeError::BinaryFailed` on non-zero exit.
    fn run(&self, audio: &Path, prompt: &str, model: &str) -> Result<String, TranscribeError>;
}

/// Shells out to `whisper-cli` on PATH.
///
/// Invocation:
/// ```text
/// whisper-cli --model <model> --output-txt --file <audio>
///             [--prompt <prompt>]
/// ```
/// The transcript is written to `<audio_stem>.txt` beside the audio file;
/// we read and return that file.
pub struct WhisperCliRunner;

impl WhisperRunner for WhisperCliRunner {
    fn run(&self, audio: &Path, prompt: &str, model: &str) -> Result<String, TranscribeError> {
        let mut cmd = Command::new("whisper-cli");
        cmd.arg("--model")
            .arg(model)
            .arg("--output-txt")
            .arg("--file")
            .arg(audio);
        if !prompt.is_empty() {
            cmd.arg("--prompt").arg(prompt);
        }

        let output = cmd.output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                TranscribeError::BinaryMissing {
                    binary: "whisper-cli".to_string(),
                }
            } else {
                TranscribeError::Io(e)
            }
        })?;

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(TranscribeError::BinaryFailed {
                binary: "whisper-cli".to_string(),
                code,
                stderr,
            });
        }

        // whisper-cli writes `<stem>.txt` next to the input file.
        let txt_path = audio.with_extension("txt");
        if !txt_path.exists() {
            return Err(TranscribeError::OutputMissing { path: txt_path });
        }
        let text = std::fs::read_to_string(&txt_path)?;
        Ok(text)
    }
}

// ---------------------------------------------------------------------------
// YtDlpRunner trait
// ---------------------------------------------------------------------------

/// Abstraction over the `yt-dlp` invocation.
pub trait YtDlpRunner {
    /// Download audio from `url` into `output_dir`, using `out_template` as
    /// the yt-dlp output template (without extension).
    ///
    /// Returns the path to the downloaded file.
    ///
    /// # Errors
    ///
    /// Returns `TranscribeError::BinaryMissing` when yt-dlp is absent,
    /// `TranscribeError::BinaryFailed` on non-zero exit, or
    /// `TranscribeError::OutputMissing` if no matching file is found.
    fn download(
        &self,
        url: &str,
        out_template: &str,
        output_dir: &Path,
        hash_prefix: &str,
    ) -> Result<PathBuf, TranscribeError>;
}

/// Shells out to `yt-dlp` on PATH.
pub struct YtDlpCliRunner;

impl YtDlpRunner for YtDlpCliRunner {
    fn download(
        &self,
        url: &str,
        out_template: &str,
        output_dir: &Path,
        hash_prefix: &str,
    ) -> Result<PathBuf, TranscribeError> {
        let output = Command::new("yt-dlp")
            .arg("--format")
            .arg("bestaudio[ext=m4a]/bestaudio/best")
            .arg("--output")
            .arg(out_template)
            .arg("--quiet")
            .arg("--no-warnings")
            .arg("--no-playlist")
            .arg(url)
            .output()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    TranscribeError::BinaryMissing {
                        binary: "yt-dlp".to_string(),
                    }
                } else {
                    TranscribeError::Io(e)
                }
            })?;

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(TranscribeError::BinaryFailed {
                binary: "yt-dlp".to_string(),
                code,
                stderr,
            });
        }

        // Find the downloaded file among known extensions.
        for ext in &["m4a", "opus", "mp3", "ogg", "wav", "webm"] {
            let candidate = output_dir.join(format!("yt_{hash_prefix}.{ext}"));
            if candidate.exists() {
                return Ok(candidate);
            }
        }

        Err(TranscribeError::OutputMissing {
            path: output_dir.join(format!("yt_{hash_prefix}.*")),
        })
    }
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// Return `true` if `path` looks like a URL rather than a file path.
#[must_use]
pub fn is_url(path: &str) -> bool {
    URL_PREFIXES.iter().any(|p| path.starts_with(p))
}

/// Build a domain hint for Whisper from god-node labels extracted from the corpus.
///
/// Order of precedence:
/// 1. If `god_nodes` is empty → return the fallback prompt.
/// 2. If `GRAPHIFY_WHISPER_PROMPT` env var is set → return its value.
/// 3. Build a topic string from up to 5 node labels.
/// 4. If no valid labels exist → return the fallback prompt.
#[must_use]
pub fn build_whisper_prompt(god_nodes: &[Value]) -> String {
    if god_nodes.is_empty() {
        return FALLBACK_PROMPT.to_string();
    }

    if let Ok(override_prompt) = std::env::var("GRAPHIFY_WHISPER_PROMPT")
        && !override_prompt.is_empty()
    {
        return override_prompt;
    }

    let labels: Vec<&str> = god_nodes
        .iter()
        .take(10)
        .filter_map(|n| n.get("label").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .take(5)
        .collect();

    if labels.is_empty() {
        return FALLBACK_PROMPT.to_string();
    }

    let topics = labels.join(", ");
    format!("Technical discussion about {topics}. Use proper punctuation and paragraph breaks.")
}

// ---------------------------------------------------------------------------
// Download audio
// ---------------------------------------------------------------------------

/// Download audio-only stream from a URL using `yt-dlp`.
///
/// Uses a stable filename derived from the SHA-1 hash of the URL
/// (`yt_<first12hexchars>.<ext>`). Returns immediately if the file already
/// exists (any of `.m4a`, `.opus`, `.mp3`, `.ogg`, `.wav`, `.webm`).
///
/// # Errors
///
/// Propagates URL-validation failures, missing-binary errors, and I/O errors.
pub fn download_audio(url: &str, output_dir: &Path) -> Result<PathBuf, TranscribeError> {
    download_audio_with(url, output_dir, &YtDlpCliRunner)
}

/// Inner implementation, injectable runner for tests.
///
/// # Errors
///
/// Returns [`TranscribeError`] on URL validation, binary, or I/O failure.
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

    // Check cache first.
    for ext in &["m4a", "opus", "mp3", "ogg", "wav", "webm"] {
        let candidate = output_dir.join(format!("yt_{hash_prefix}.{ext}"));
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    runner.download(url, &out_template, output_dir, &hash_prefix)
}

/// SHA-1 hash of `url`, first 12 hex characters.
fn url_hash_prefix(url: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(url.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)[..12].to_string()
}

// ---------------------------------------------------------------------------
// Transcribe
// ---------------------------------------------------------------------------

/// Transcribe a video/audio file or URL to a `.txt` transcript.
///
/// - If `video_path` is a URL, audio is downloaded first via `yt-dlp`.
/// - Returns cached transcript immediately unless `force` is `true`.
/// - `initial_prompt` overrides the default fallback prompt passed to whisper.
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

/// Inner implementation with injectable runners for testing.
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

    let stem = audio_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
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

// ---------------------------------------------------------------------------
// Transcribe all
// ---------------------------------------------------------------------------

/// Transcribe a list of video/audio files or URLs; failures are logged and skipped.
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
                // Mirror Python: warn and continue.
                eprintln!("  warning: could not transcribe {vf}: {e}");
            }
        }
    }
    results
}
