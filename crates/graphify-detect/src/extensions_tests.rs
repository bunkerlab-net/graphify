//! Unit tests for [`crate::extensions`].
//!
//! Lives in a sibling file (rather than an inline `mod tests`) per the
//! project convention that test code is colocated by name but separated from
//! production code for readability.

#![allow(clippy::expect_used)] // test-only — `.expect("...")` panics are the failure

use super::*;
use std::path::Path;

/// `.py` files are classified as `FileType::Code` regardless of casing.
#[test]
fn classify_python() {
    assert_eq!(classify_file(Path::new("foo.py")), Some(FileType::Code));
}

/// `.ts` (TypeScript) is a first-class code extension.
#[test]
fn classify_typescript() {
    assert_eq!(classify_file(Path::new("bar.ts")), Some(FileType::Code));
}

/// Markdown files are tagged as documents (driven through the LLM extractor).
#[test]
fn classify_markdown() {
    assert_eq!(
        classify_file(Path::new("README.md")),
        Some(FileType::Document)
    );
}

/// Standalone PDFs land in the paper bucket.
#[test]
fn classify_pdf() {
    assert_eq!(classify_file(Path::new("paper.pdf")), Some(FileType::Paper));
}

/// PDFs *inside* Xcode `.xcassets` bundles are build artefacts; they must be
/// excluded so we don't ingest icon vector files as papers.
#[test]
fn classify_pdf_in_xcassets_skipped() {
    let p = Path::new("MyApp/Images.xcassets/icon.imageset/icon.pdf");
    assert_eq!(classify_file(p), None);
}

/// The xcassets exclusion applies regardless of nesting depth.
#[test]
fn classify_pdf_in_xcassets_root_skipped() {
    let p = Path::new("Pods/HXPHPicker/Assets.xcassets/photo.pdf");
    assert_eq!(classify_file(p), None);
}

/// Unknown extensions return `None`, signalling "skip this file."
#[test]
fn classify_unknown_returns_none() {
    assert_eq!(classify_file(Path::new("archive.zip")), None);
}

/// All raster image formats we support map to `FileType::Image`.
#[test]
fn classify_images() {
    assert_eq!(
        classify_file(Path::new("screenshot.png")),
        Some(FileType::Image)
    );
    assert_eq!(
        classify_file(Path::new("design.jpg")),
        Some(FileType::Image)
    );
    assert_eq!(
        classify_file(Path::new("diagram.webp")),
        Some(FileType::Image)
    );
}

/// Audio and video extensions share the same `FileType::Video` bucket because
/// they both go through the transcribe pipeline.
#[test]
fn classify_video_extensions() {
    assert_eq!(
        classify_file(Path::new("lecture.mp4")),
        Some(FileType::Video)
    );
    assert_eq!(
        classify_file(Path::new("podcast.mp3")),
        Some(FileType::Video)
    );
    assert_eq!(classify_file(Path::new("talk.mov")), Some(FileType::Video));
    assert_eq!(
        classify_file(Path::new("recording.wav")),
        Some(FileType::Video)
    );
    assert_eq!(
        classify_file(Path::new("webinar.webm")),
        Some(FileType::Video)
    );
    assert_eq!(classify_file(Path::new("audio.m4a")), Some(FileType::Video));
}

/// Google-Workspace shortcut extensions (`.gdoc`, `.gsheet`, `.gslides`) map
/// to `FileType::Document` so the converter knows to fetch their content.
#[test]
fn classify_google_workspace() {
    assert_eq!(
        classify_file(Path::new("notes.gdoc")),
        Some(FileType::Document)
    );
    assert_eq!(
        classify_file(Path::new("budget.gsheet")),
        Some(FileType::Document)
    );
    assert_eq!(
        classify_file(Path::new("deck.gslides")),
        Some(FileType::Document)
    );
}
