//! Parity port of `graphify-py/tests/test_office_limits.py`.
//!
//! Resource-cap guards for parsing untrusted office/PDF files (F2). `.docx`/
//! `.xlsx` are zip+XML containers; a few-KB zip-bomb can decompress to
//! gigabytes and OOM-kill the process during a corpus scan. These tests verify
//! the pre-parse screen rejects bombs before the parser ever decompresses them.

// The file-top `expect_used`/`unwrap_used` suppression is the sanctioned
// project convention for integration-test files (see AGENTS.md "Strict lints":
// `#![allow(...)]` at the top is OK for `tests/*.rs`). A panic in a test fixture
// is a test failure with a useful message, which is the desired behaviour here,
// so converting every helper to `?`-returning would be churn against that
// convention rather than a correctness gain. `cast_possible_truncation` covers
// the deterministic `i.wrapping_mul(...) as u8` fill in the streaming-bomb test.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation
)]

use std::io::Write as _;
use std::path::Path;

use graphify_detect::office::{
    OFFICE_MAX_RAW_BYTES, docx_to_markdown, extract_pdf_text, extract_pdf_text_with,
    file_within_size_cap, xlsx_extract_structure, xlsx_to_markdown, zip_within_caps,
    zip_within_caps_with,
};
use zip::write::SimpleFileOptions;

/// Write a single-member deflate zip with `payload` stored at `name`.
fn write_zip(path: &Path, name: &str, payload: &[u8]) {
    write_multi_zip(path, &[(name, payload)]);
}

/// Write a multi-member deflate zip from `(name, payload)` entries, so a fixture
/// can be a structurally valid `.docx`/`.xlsx` (`Content_Types` + rels + the body
/// part that carries the bomb payload) rather than a single stray member.
fn write_multi_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).expect("create zip");
    let mut zf = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, payload) in entries {
        zf.start_file(*name, opts).expect("start_file");
        zf.write_all(payload).expect("write payload");
    }
    zf.finish().expect("finish zip");
}

/// Build a small, valid single-page PDF whose text content is `text`, using the
/// same `lopdf` engine `extract_pdf_text` reads with, so the parity test can
/// prove the *size cap* (not a parse error) is what produces an empty string.
// `page_id`/`pages_id` are the natural PDF object names; the proximity is
// inherent to the PDF object model, not a readability hazard.
#[allow(clippy::similar_names)]
fn build_text_pdf(path: &Path, text: &str) {
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 24.into()]),
            Operation::new("Td", vec![100.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(
        dictionary! {},
        content.encode().expect("encode"),
    ));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
        "Resources" => resources_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    doc.save(path).expect("save pdf");
}

#[test]
fn file_within_size_cap_basic() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("a.bin");
    std::fs::write(&f, vec![b'x'; 1024]).expect("write");
    assert!(file_within_size_cap(&f, OFFICE_MAX_RAW_BYTES)); // within default cap
    assert!(!file_within_size_cap(&f, 512)); // over an explicit small cap
    assert!(!file_within_size_cap(
        &tmp.path().join("missing"),
        OFFICE_MAX_RAW_BYTES
    ));
}

#[test]
fn zip_ratio_bomb_rejected() {
    // A tiny file that expands far past the ratio threshold is rejected.
    let tmp = tempfile::tempdir().expect("tempdir");
    let bomb = tmp.path().join("bomb.xlsx");
    write_zip(
        &bomb,
        "xl/worksheets/sheet1.xml",
        &vec![b'0'; 5 * 1024 * 1024],
    );
    assert!(
        std::fs::metadata(&bomb).expect("stat").len() < 100 * 1024,
        "5 MiB of zeros should compress to well under 100 KiB"
    );
    assert!(!zip_within_caps(&bomb));
}

#[test]
fn legit_zip_passes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ok = tmp.path().join("ok.docx");
    write_zip(
        &ok,
        "word/document.xml",
        b"<xml>hello world</xml>".repeat(20).as_slice(),
    );
    assert!(zip_within_caps(&ok));
}

#[test]
fn non_zip_rejected() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let notzip = tmp.path().join("fake.xlsx");
    std::fs::write(&notzip, b"this is not a zip file").expect("write");
    assert!(!zip_within_caps(&notzip));
}

#[test]
fn converters_return_empty_for_bomb() {
    // The live converters bail out (return "") on a bomb *before parsing*. The
    // fixtures are structurally valid Office archives (Content_Types + rels +
    // the body part) with the 5 MiB bomb payload in a real internal part, so the
    // empty result is attributable to the pre-parse zip-bomb screen rather than a
    // missing-part error: if the `zip_within_caps` guard were removed the parser
    // would actually open the bomb part instead of bailing on a structural gap.
    let tmp = tempfile::tempdir().expect("tempdir");
    let blob = vec![b'0'; 5 * 1024 * 1024];

    let docx = tmp.path().join("bomb.docx");
    write_multi_zip(
        &docx,
        &[
            ("[Content_Types].xml", b"<Types/>"),
            ("_rels/.rels", b"<Relationships/>"),
            ("word/document.xml", blob.as_slice()),
        ],
    );
    assert_eq!(docx_to_markdown(&docx), "");

    let xlsx = tmp.path().join("bomb.xlsx");
    write_multi_zip(
        &xlsx,
        &[
            ("[Content_Types].xml", b"<Types/>"),
            ("_rels/.rels", b"<Relationships/>"),
            ("xl/workbook.xml", b"<workbook/>"),
            ("xl/sharedStrings.xml", blob.as_slice()),
        ],
    );
    assert_eq!(xlsx_to_markdown(&xlsx), "");
}

#[test]
fn structure_extraction_returns_empty_for_bomb() {
    // `xlsx_extract_structure` is a public XLSX parsing path, so the zip-bomb
    // screen must reject a bomb before calamine opens it — same guard as
    // `xlsx_to_markdown`. Rust hardens this path even though graphify-py leaves
    // it unguarded (detect.py F-035 only flags it for a future audit).
    let tmp = tempfile::tempdir().expect("tempdir");
    let bomb = tmp.path().join("bomb.xlsx");
    write_multi_zip(
        &bomb,
        &[
            ("[Content_Types].xml", b"<Types/>"),
            ("_rels/.rels", b"<Relationships/>"),
            ("xl/workbook.xml", b"<workbook/>"),
            (
                "xl/sharedStrings.xml",
                vec![b'0'; 5 * 1024 * 1024].as_slice(),
            ),
        ],
    );
    assert!(xlsx_extract_structure(&bomb).sheets.is_empty());
}

#[test]
fn legit_multi_member_passes_streaming() {
    // A normal multi-member office zip passes the streaming-ceiling pass.
    let tmp = tempfile::tempdir().expect("tempdir");
    let ok = tmp.path().join("ok.xlsx");
    let file = std::fs::File::create(&ok).expect("create");
    let mut zf = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zf.start_file("[Content_Types].xml", opts).expect("start");
    zf.write_all(b"<types/>").expect("write");
    zf.start_file("xl/workbook.xml", opts).expect("start");
    zf.write_all(b"<workbook/>".repeat(100).as_slice())
        .expect("write");
    zf.start_file("xl/worksheets/sheet1.xml", opts)
        .expect("start");
    zf.write_all(b"<sheetData>rows</sheetData>".repeat(500).as_slice())
        .expect("write");
    zf.finish().expect("finish");
    assert!(zip_within_caps(&ok));
}

#[test]
fn streaming_ceiling_rejects_oversized_actual() {
    // With a low decompressed cap, content whose actual bytes exceed it is
    // rejected. This exercises the authoritative bounded-decompression pass:
    // the function reads real decompressed bytes (not the attacker-declared
    // central-directory sizes) and stops once the ceiling is crossed.
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("big.xlsx");
    // ~512 KiB of incompressible data: low ratio (passes the ratio pre-filter),
    // but real decompressed size far exceeds the 64 KiB ceiling.
    let mut data = vec![0u8; 512 * 1024];
    for (i, b) in data.iter_mut().enumerate() {
        // Deterministic pseudo-random so the bytes don't compress.
        *b = (i.wrapping_mul(2_654_435_761) >> 13) as u8;
    }
    write_zip(&f, "xl/x.xml", &data);
    // 64 KiB decompressed cap via the parameterized variant (Python monkeypatches
    // the module constant; the Rust seam is an explicit cap argument).
    assert!(!zip_within_caps_with(
        &f,
        OFFICE_MAX_RAW_BYTES,
        64 * 1024,
        graphify_detect::office::OFFICE_MAX_COMPRESSION_RATIO,
    ));
}

#[test]
fn pdf_over_cap_returns_empty() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // A genuinely valid PDF that extracts to non-empty text under the default
    // cap — the positive control proving the parser *would* yield text.
    let pdf = tmp.path().join("doc.pdf");
    build_text_pdf(&pdf, "Hello Graphify");
    assert!(
        !extract_pdf_text(&pdf).is_empty(),
        "valid PDF under the cap must extract text"
    );

    // Same valid, parseable file under a tiny cap → "". The only thing that
    // changed is the cap, so the empty result is provably the size screen, not a
    // parse error (mirrors Python monkeypatching `_OFFICE_MAX_RAW_BYTES`).
    assert_eq!(extract_pdf_text_with(&pdf, 16), "");

    // And via the production const path: a file larger than the raw cap is
    // skipped before the parser opens it. A sparse file makes the over-cap size
    // instant; the file_within_size_cap assertion pins the guard predicate.
    let big = tmp.path().join("big.pdf");
    let f = std::fs::File::create(&big).expect("create");
    f.set_len(OFFICE_MAX_RAW_BYTES + 4096).expect("set_len");
    drop(f);
    assert!(!file_within_size_cap(&big, OFFICE_MAX_RAW_BYTES));
    assert_eq!(extract_pdf_text(&big), "");
}
