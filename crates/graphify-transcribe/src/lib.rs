//! Video/audio transcription via `whisper-cli` and `yt-dlp` shell-outs.
//!
//! Ports `graphify-py/graphify/transcribe.py`.

mod constants;
mod download;
mod error;
mod transcribe_fn;
mod util;
mod whisper;
mod ytdlp;

pub use constants::VIDEO_EXTENSIONS;
pub use download::{download_audio, download_audio_with};
pub use error::TranscribeError;
pub use transcribe_fn::{transcribe, transcribe_all, transcribe_with};
pub use util::{build_whisper_prompt, is_url};
pub use whisper::{WhisperCliRunner, WhisperRunner};
pub use ytdlp::{YtDlpCliRunner, YtDlpRunner};
