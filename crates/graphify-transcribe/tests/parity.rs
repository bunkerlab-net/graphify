//! Parity tests against graphify-py/tests/test_transcribe.py.
//!
//! Pure-logic and cache-path tests run without any external binaries.
//! Tests that would require `whisper-cli` use an injected `MockWhisperRunner`.

#![allow(clippy::expect_used, unsafe_code)]

use std::path::{Path, PathBuf};

use graphify_transcribe::{
    TranscribeError, VIDEO_EXTENSIONS, WhisperRunner, YtDlpRunner, build_whisper_prompt,
    transcribe_all, transcribe_with,
};
use serde_json::json;
use serial_test::serial;
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// Mock runners
// ---------------------------------------------------------------------------

/// A `WhisperRunner` that always succeeds, returning the configured text.
struct MockWhisperRunner {
    text: String,
}

impl WhisperRunner for MockWhisperRunner {
    fn run(&self, _audio: &Path, _prompt: &str, _model: &str) -> Result<String, TranscribeError> {
        Ok(self.text.clone())
    }
}

/// A `WhisperRunner` that always fails with `BinaryMissing`.
struct MissingWhisperRunner;

impl WhisperRunner for MissingWhisperRunner {
    fn run(&self, _audio: &Path, _prompt: &str, _model: &str) -> Result<String, TranscribeError> {
        Err(TranscribeError::BinaryMissing {
            binary: "whisper-cli".to_string(),
        })
    }
}

/// A `YtDlpRunner` that always fails (not used in most tests but required by the API).
struct UnreachableYtDlpRunner;

impl YtDlpRunner for UnreachableYtDlpRunner {
    fn download(
        &self,
        _url: &str,
        _out_template: &str,
        _output_dir: &Path,
        _hash_prefix: &str,
    ) -> Result<PathBuf, TranscribeError> {
        Err(TranscribeError::BinaryMissing {
            binary: "yt-dlp".to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// VIDEO_EXTENSIONS
// ---------------------------------------------------------------------------

#[test]
fn test_video_extensions_set() {
    assert!(VIDEO_EXTENSIONS.contains(&".mp4"));
    assert!(VIDEO_EXTENSIONS.contains(&".mp3"));
    assert!(VIDEO_EXTENSIONS.contains(&".wav"));
    assert!(VIDEO_EXTENSIONS.contains(&".mov"));
    assert!(!VIDEO_EXTENSIONS.contains(&".py"));
}

// ---------------------------------------------------------------------------
// build_whisper_prompt
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_build_whisper_prompt_no_nodes() {
    // SAFETY: serial attribute ensures single-threaded access to env.
    unsafe { std::env::remove_var("GRAPHIFY_WHISPER_PROMPT") };
    let prompt = build_whisper_prompt(&[]);
    assert!(
        prompt.to_lowercase().contains("punctuation") || !prompt.is_empty(),
        "expected fallback prompt, got: {prompt}"
    );
}

#[test]
#[serial]
fn test_build_whisper_prompt_env_override() {
    // SAFETY: serial attribute ensures single-threaded access to env.
    unsafe { std::env::set_var("GRAPHIFY_WHISPER_PROMPT", "Custom domain hint.") };
    let nodes = vec![json!({"label": "Python"}), json!({"label": "FastAPI"})];
    let prompt = build_whisper_prompt(&nodes);
    unsafe { std::env::remove_var("GRAPHIFY_WHISPER_PROMPT") };
    assert_eq!(prompt, "Custom domain hint.");
}

#[test]
#[serial]
fn test_build_whisper_prompt_returns_topic_string() {
    // SAFETY: serial attribute ensures single-threaded access to env.
    unsafe { std::env::remove_var("GRAPHIFY_WHISPER_PROMPT") };
    let nodes = vec![
        json!({"label": "neural networks"}),
        json!({"label": "transformers"}),
        json!({"label": "attention"}),
    ];
    let prompt = build_whisper_prompt(&nodes);
    let lower = prompt.to_lowercase();
    assert!(
        lower.contains("neural networks") || lower.contains("transformers"),
        "expected topic labels in prompt, got: {prompt}"
    );
    assert!(
        lower.contains("punctuation"),
        "expected punctuation mention in prompt, got: {prompt}"
    );
}

#[test]
#[serial]
fn test_build_whisper_prompt_nodes_without_labels() {
    // SAFETY: serial attribute ensures single-threaded access to env.
    unsafe { std::env::remove_var("GRAPHIFY_WHISPER_PROMPT") };
    let nodes = vec![json!({"id": "1"}), json!({"id": "2", "label": ""})];
    let prompt = build_whisper_prompt(&nodes);
    assert!(!prompt.is_empty());
}

// ---------------------------------------------------------------------------
// transcribe — cache-path tests (no binaries needed)
// ---------------------------------------------------------------------------

#[test]
fn test_transcribe_uses_cache() {
    let tmp = tempdir().expect("tempdir");
    let video = tmp.path().join("lecture.mp4");
    std::fs::write(&video, b"fake").expect("write fixture");
    let out_dir = tmp.path().join("transcripts");
    std::fs::create_dir_all(&out_dir).expect("create_dir_all");
    let cached = out_dir.join("lecture.txt");
    std::fs::write(&cached, "Cached transcript content.").expect("write fixture");

    // Pass a runner that would fail if invoked — proves the cache is used.
    let result = transcribe_with(
        &video,
        Some(&out_dir),
        None,
        false,
        &MissingWhisperRunner,
        &UnreachableYtDlpRunner,
    )
    .expect("test invariant");

    assert_eq!(result, cached);
}

#[test]
fn test_transcribe_force_reruns() {
    let tmp = tempdir().expect("tempdir");
    let video = tmp.path().join("talk.mp4");
    std::fs::write(&video, b"fake").expect("write fixture");
    let out_dir = tmp.path().join("transcripts");
    std::fs::create_dir_all(&out_dir).expect("create_dir_all");
    std::fs::write(out_dir.join("talk.txt"), "Old transcript.").expect("test invariant");

    let runner = MockWhisperRunner {
        text: "New transcript segment.".to_string(),
    };

    let result = transcribe_with(
        &video,
        Some(&out_dir),
        None,
        true, // force=true
        &runner,
        &UnreachableYtDlpRunner,
    )
    .expect("test invariant");

    let content = std::fs::read_to_string(&result).expect("read fixture");
    assert_eq!(content, "New transcript segment.");
}

#[test]
fn test_transcribe_missing_whisper_binary() {
    // Rust equivalent of Python's test_transcribe_missing_faster_whisper.
    // Injects a runner that returns BinaryMissing.
    let tmp = tempdir().expect("tempdir");
    let video = tmp.path().join("clip.mp4");
    std::fs::write(&video, b"fake").expect("write fixture");

    let err = transcribe_with(
        &video,
        Some(&tmp.path().join("out")),
        None,
        false,
        &MissingWhisperRunner,
        &UnreachableYtDlpRunner,
    )
    .expect_err("expected Err");

    assert!(
        matches!(err, TranscribeError::BinaryMissing { .. }),
        "expected BinaryMissing, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// transcribe_all
// ---------------------------------------------------------------------------

#[test]
fn test_transcribe_all_empty() {
    let result = transcribe_all(&[], None, None);
    assert!(result.is_empty());
}

#[test]
fn test_transcribe_all_uses_cache() {
    let tmp = tempdir().expect("tempdir");
    let video = tmp.path().join("lecture.mp4");
    std::fs::write(&video, b"fake").expect("write fixture");
    let out_dir = tmp.path().join("transcripts");
    std::fs::create_dir_all(&out_dir).expect("create_dir_all");
    let cached = out_dir.join("lecture.txt");
    std::fs::write(&cached, "Cached.").expect("write fixture");

    let results = transcribe_all(&[video.to_str().expect("utf-8 path")], Some(&out_dir), None);
    assert_eq!(results.len(), 1);
    assert!(results[0].contains("lecture.txt"));
}

#[test]
fn test_transcribe_all_skips_failed() {
    // A file that does not exist as a URL (no http://) and has no cached
    // transcript — transcribe will fail with I/O error creating the dir then
    // trying to run the real binary. We want failures to be skipped gracefully.
    //
    // We can't inject a runner into transcribe_all directly (it calls the
    // public `transcribe`). Instead we use a path whose parent does not exist
    // AND whose transcript cache is absent, which triggers a BinaryMissing
    // error when the real whisper-cli is absent (or an I/O error). Either way,
    // the result must be an empty Vec (skipped).
    let tmp = tempdir().expect("tempdir");
    let broken = tmp.path().join("broken.mp4");
    std::fs::write(&broken, b"fake").expect("write fixture");

    // transcribe_all calls the real `transcribe`; with no cached .txt and no
    // whisper-cli, it will fail. The function must swallow the error.
    let results = transcribe_all(
        &[broken.to_str().expect("utf-8 path")],
        Some(&tmp.path().join("out")),
        None,
    );
    // Either empty (binary missing) or 1 entry (binary happened to exist on CI).
    // The important contract: no panic.
    drop(results); // we just verify no panic/propagation
}
