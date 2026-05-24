//! Parity tests for file classification.
//!
//! Mirrors `graphify-py/tests/test_detect.py` — classification tests.
#![allow(clippy::expect_used)]

use graphify_detect::{FileType, classify_file};
use std::path::Path;
use tempfile::tempdir;

#[test]
fn classify_python() {
    assert_eq!(classify_file(Path::new("foo.py")), Some(FileType::Code));
}

#[test]
fn classify_typescript() {
    assert_eq!(classify_file(Path::new("bar.ts")), Some(FileType::Code));
}

#[test]
fn classify_markdown() {
    assert_eq!(
        classify_file(Path::new("README.md")),
        Some(FileType::Document)
    );
}

#[test]
fn classify_pdf() {
    assert_eq!(classify_file(Path::new("paper.pdf")), Some(FileType::Paper));
}

#[test]
fn classify_pdf_in_xcassets_skipped() {
    let p = Path::new("MyApp/Images.xcassets/icon.imageset/icon.pdf");
    assert_eq!(classify_file(p), None);
}

#[test]
fn classify_pdf_in_xcassets_root_skipped() {
    let p = Path::new("Pods/HXPHPicker/Assets.xcassets/photo.pdf");
    assert_eq!(classify_file(p), None);
}

#[test]
fn classify_unknown_returns_none() {
    assert_eq!(classify_file(Path::new("archive.zip")), None);
}

#[test]
fn classify_image_png() {
    assert_eq!(
        classify_file(Path::new("screenshot.png")),
        Some(FileType::Image)
    );
}

#[test]
fn classify_image_jpg() {
    assert_eq!(
        classify_file(Path::new("design.jpg")),
        Some(FileType::Image)
    );
}

#[test]
fn classify_image_webp() {
    assert_eq!(
        classify_file(Path::new("diagram.webp")),
        Some(FileType::Image)
    );
}

#[test]
fn classify_video_mp4() {
    assert_eq!(
        classify_file(Path::new("lecture.mp4")),
        Some(FileType::Video)
    );
}

#[test]
fn classify_video_mp3() {
    assert_eq!(
        classify_file(Path::new("podcast.mp3")),
        Some(FileType::Video)
    );
}

#[test]
fn classify_video_mov() {
    assert_eq!(classify_file(Path::new("talk.mov")), Some(FileType::Video));
}

#[test]
fn classify_video_wav() {
    assert_eq!(
        classify_file(Path::new("recording.wav")),
        Some(FileType::Video)
    );
}

#[test]
fn classify_video_webm() {
    assert_eq!(
        classify_file(Path::new("webinar.webm")),
        Some(FileType::Video)
    );
}

#[test]
fn classify_video_m4a() {
    assert_eq!(classify_file(Path::new("audio.m4a")), Some(FileType::Video));
}

#[test]
fn classify_google_workspace_gdoc() {
    assert_eq!(
        classify_file(Path::new("notes.gdoc")),
        Some(FileType::Document)
    );
}

#[test]
fn classify_google_workspace_gsheet() {
    assert_eq!(
        classify_file(Path::new("budget.gsheet")),
        Some(FileType::Document)
    );
}

#[test]
fn classify_google_workspace_gslides() {
    assert_eq!(
        classify_file(Path::new("deck.gslides")),
        Some(FileType::Document)
    );
}

#[test]
fn classify_md_paper_by_signals() {
    let tmp = tempdir().expect("tempdir");
    let paper = tmp.path().join("paper.md");
    std::fs::write(
        &paper,
        "# Abstract\n\nWe propose a new method. See [1] and [23].\n\
         This work was published in the Journal of AI. ArXiv preprint.\n\
         See Equation 3 for details. \\cite{vaswani2017}.\n",
    )
    .expect("test invariant");
    assert_eq!(classify_file(&paper), Some(FileType::Paper));
}

#[test]
fn classify_md_doc_without_signals() {
    let tmp = tempdir().expect("tempdir");
    let doc = tmp.path().join("notes.md");
    std::fs::write(
        &doc,
        "# My Notes\n\nHere are some notes about the project.\n",
    )
    .expect("test invariant");
    assert_eq!(classify_file(&doc), Some(FileType::Document));
}
