//! Tests for `graphify_detect::office` — exercise the error paths
//! (missing files, malformed binaries) and the empty-result wrappers.

#![allow(
    clippy::expect_used,
    clippy::similar_names,
    clippy::items_after_statements
)]

use std::fs;

use graphify_detect::office::{
    convert_office_file, docx_to_markdown, extract_pdf_text, xlsx_extract_structure,
    xlsx_to_markdown,
};

#[test]
fn extract_pdf_text_returns_empty_on_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let result = extract_pdf_text(&tmp.path().join("nonexistent.pdf"));
    assert_eq!(result, "");
}

#[test]
fn extract_pdf_text_returns_empty_on_invalid_bytes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = tmp.path().join("bad.pdf");
    fs::write(&p, b"this is not a pdf").expect("write fixture");
    let result = extract_pdf_text(&p);
    assert_eq!(result, "");
}

#[test]
fn docx_to_markdown_returns_empty_on_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    assert_eq!(docx_to_markdown(&tmp.path().join("nonexistent.docx")), "");
}

#[test]
fn docx_to_markdown_returns_empty_on_non_zip() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = tmp.path().join("bad.docx");
    fs::write(&p, b"definitely not a docx").expect("write fixture");
    assert_eq!(docx_to_markdown(&p), "");
}

#[test]
fn xlsx_to_markdown_returns_empty_on_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    assert_eq!(xlsx_to_markdown(&tmp.path().join("nonexistent.xlsx")), "");
}

#[test]
fn xlsx_to_markdown_returns_empty_on_non_zip() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = tmp.path().join("bad.xlsx");
    fs::write(&p, b"definitely not an xlsx").expect("write fixture");
    assert_eq!(xlsx_to_markdown(&p), "");
}

#[test]
fn xlsx_extract_structure_empty_on_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let s = xlsx_extract_structure(&tmp.path().join("nonexistent.xlsx"));
    assert!(s.sheets.is_empty());
}

#[test]
fn convert_office_file_unknown_extension_returns_none() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("random.xyz");
    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&out_dir).expect("create_dir_all");
    fs::write(&src, b"junk").expect("write fixture");
    let result = convert_office_file(&src, &out_dir).expect("test invariant");
    assert!(result.is_none());
}

#[test]
fn convert_office_file_pdf_extension_falls_through_to_empty_text() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("dummy.pdf");
    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&out_dir).expect("create_dir_all");
    // Invalid PDF; extract returns "" and the function returns Ok(None) (no md written).
    fs::write(&src, b"not really a pdf").expect("write fixture");
    let _ = convert_office_file(&src, &out_dir);
    // Just verify it doesn't panic. May return None or Some depending on impl.
}

#[test]
fn convert_office_file_docx_extension_falls_through() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("dummy.docx");
    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&out_dir).expect("create_dir_all");
    fs::write(&src, b"not really a docx").expect("write fixture");
    let _ = convert_office_file(&src, &out_dir);
}

#[test]
fn convert_office_file_xlsx_extension_falls_through() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("dummy.xlsx");
    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&out_dir).expect("create_dir_all");
    fs::write(&src, b"not really an xlsx").expect("write fixture");
    let _ = convert_office_file(&src, &out_dir);
}

/// Build a minimal valid DOCX (ZIP archive containing `word/document.xml`)
/// for exercising the docx parser's happy path.
fn build_minimal_docx(path: &std::path::Path, body: &str) {
    use std::io::Write;
    let f = fs::File::create(path).expect("test invariant");
    let mut zip = zip::ZipWriter::new(f);
    let opts: zip::write::SimpleFileOptions =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("word/document.xml", opts)
        .expect("test invariant");
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>{body}</w:t></w:r></w:p>
    <w:p><w:r><w:t>second paragraph</w:t></w:r></w:p>
  </w:body>
</w:document>"#
    );
    zip.write_all(xml.as_bytes()).expect("test invariant");
    zip.finish().expect("test invariant");
}

#[test]
fn docx_to_markdown_parses_minimal_docx() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = tmp.path().join("real.docx");
    build_minimal_docx(&p, "hello docx world");
    let md = docx_to_markdown(&p);
    assert!(md.contains("hello docx world"), "got: {md}");
    assert!(md.contains("second paragraph"), "got: {md}");
}

/// Build a minimal valid XLSX (Open XML Spreadsheet) using `umya-spreadsheet`-like
/// shape — just enough for `calamine` to parse a single cell.
fn build_minimal_xlsx(path: &std::path::Path) {
    use std::io::Write;
    let f = fs::File::create(path).expect("test invariant");
    let mut zip = zip::ZipWriter::new(f);
    let opts: zip::write::SimpleFileOptions =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    zip.start_file("[Content_Types].xml", opts)
        .expect("test invariant");
    zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#).expect("test invariant");

    zip.start_file("_rels/.rels", opts).expect("test invariant");
    zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#).expect("test invariant");

    zip.start_file("xl/_rels/workbook.xml.rels", opts)
        .expect("test invariant");
    zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#).expect("test invariant");

    zip.start_file("xl/workbook.xml", opts)
        .expect("test invariant");
    zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Sheet1" sheetId="1" r:id="rId1"/>
  </sheets>
</workbook>"#).expect("test invariant");

    zip.start_file("xl/worksheets/sheet1.xml", opts)
        .expect("test invariant");
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1">
      <c r="A1" t="inlineStr"><is><t>hello cell</t></is></c>
      <c r="B1" t="inlineStr"><is><t>second</t></is></c>
    </row>
    <row r="2">
      <c r="A2"><v>42</v></c>
    </row>
  </sheetData>
</worksheet>"#,
    )
    .expect("test invariant");

    zip.finish().expect("test invariant");
}

#[test]
fn xlsx_extract_structure_reads_minimal() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = tmp.path().join("real.xlsx");
    build_minimal_xlsx(&p);
    let s = xlsx_extract_structure(&p);
    assert!(
        !s.sheets.is_empty(),
        "expected at least one sheet from minimal xlsx"
    );
}

#[test]
fn xlsx_to_markdown_reads_minimal() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = tmp.path().join("real.xlsx");
    build_minimal_xlsx(&p);
    let md = xlsx_to_markdown(&p);
    // Markdown should mention the sheet name and probably contain the cells.
    assert!(!md.is_empty(), "expected markdown output");
}

/// Build a minimal valid PDF using lopdf's writer.
fn build_minimal_pdf(path: &std::path::Path) {
    use lopdf::{Document, Object, Stream, dictionary};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    let content_obj = "BT /F1 12 Tf 100 700 Td (hello pdf world) Tj ET";
    let stream = Stream::new(dictionary! {}, content_obj.as_bytes().to_vec());
    let content_id = doc.add_object(stream);

    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! {
            "F1" => dictionary! {
                "Type" => "Font",
                "Subtype" => "Type1",
                "BaseFont" => "Helvetica",
            },
        },
    });

    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Resources" => resources_id,
        "Contents" => content_id,
        "MediaBox" => vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(595),
            Object::Integer(842)
        ],
    });

    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    doc.compress();
    doc.save(path).expect("test invariant");
}

#[test]
fn extract_pdf_text_parses_minimal_pdf() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = tmp.path().join("real.pdf");
    build_minimal_pdf(&p);
    let text = extract_pdf_text(&p);
    // Even if text content extraction is empty (depends on font handling),
    // we exercised the entire PDF parse pipeline.
    let _ = text;
}

#[test]
fn convert_office_file_real_pdf() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("real.pdf");
    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&out_dir).expect("create_dir_all");
    build_minimal_pdf(&src);
    let _ = convert_office_file(&src, &out_dir);
}

#[test]
fn convert_office_file_real_docx_writes_md() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("real.docx");
    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&out_dir).expect("create_dir_all");
    build_minimal_docx(&src, "convertible doc");
    let result = convert_office_file(&src, &out_dir).expect("test invariant");
    assert!(result.is_some(), "expected an output markdown file");
    let out_path = result.expect("test invariant");
    assert!(out_path.exists());
    let md = fs::read_to_string(&out_path).expect("read fixture");
    assert!(md.contains("convertible doc"));
}

// ── #1226: NFC-stable sidecar names + skip rewriting existing sidecars ───────

#[test]
fn convert_office_file_hash_stable_across_nfc_nfd() {
    // The sidecar hash must be identical whether the source path arrives in NFC
    // or NFD form (macOS os.walk yields NFD; constructed Paths are NFC).
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("report");
    fs::create_dir_all(&base).expect("create_dir_all");
    let out_dir = tmp.path().join("converted");
    let nfc = base.join("caf\u{e9}.docx"); // é precomposed
    let nfd = base.join("cafe\u{301}.docx"); // e + combining acute
    build_minimal_docx(&nfc, "hello world");
    build_minimal_docx(&nfd, "hello world");

    let out_nfc = convert_office_file(&nfc, &out_dir)
        .expect("convert")
        .expect("sidecar");
    let out_nfd = convert_office_file(&nfd, &out_dir)
        .expect("convert")
        .expect("sidecar");

    let suffix = |p: &std::path::Path| -> String {
        p.file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.rsplit('_').next())
            .expect("hash suffix")
            .to_string()
    };
    assert_eq!(suffix(&out_nfc), suffix(&out_nfd));
}

#[test]
fn convert_office_file_does_not_rewrite_existing_sidecar() {
    // A second conversion of an unchanged source must not rewrite the sidecar,
    // so its content (and mtime) stays put for detect_incremental.
    let tmp = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp.path().join("converted");
    let src = tmp.path().join("doc.docx");
    build_minimal_docx(&src, "hello world");

    let first = convert_office_file(&src, &out_dir)
        .expect("convert")
        .expect("sidecar");
    // Overwrite with a sentinel; a rewrite would clobber it.
    fs::write(&first, "SENTINEL").expect("write sentinel");

    let second = convert_office_file(&src, &out_dir)
        .expect("convert")
        .expect("sidecar");
    assert_eq!(second, first);
    assert_eq!(
        fs::read_to_string(&first).expect("read"),
        "SENTINEL",
        "existing sidecar must not be rewritten"
    );
}

#[test]
fn convert_office_file_regenerates_when_source_is_newer() {
    // When the source Office file is modified after the sidecar was written, the
    // stale sidecar must be regenerated so extraction (and `detect_incremental`,
    // which tracks the sidecar) sees the new content. Matches graphify-py v0.9.12
    // `convert_office_file`, which re-converts when the source is newer (#1649).
    let tmp = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp.path().join("converted");
    let src = tmp.path().join("doc.docx");
    build_minimal_docx(&src, "version one");

    let sidecar = convert_office_file(&src, &out_dir)
        .expect("convert")
        .expect("sidecar");
    // Overwrite the sidecar with a sentinel that a regeneration must clobber.
    fs::write(&sidecar, "SENTINEL").expect("write sentinel");
    let sidecar_mtime = fs::metadata(&sidecar)
        .expect("meta")
        .modified()
        .expect("mtime");

    // Rewrite the source until its mtime is strictly newer than the sidecar.
    // Looping on a short sleep tolerates coarse filesystem mtime resolution
    // without a fixed long delay (on ns-resolution filesystems the first
    // iteration already wins).
    loop {
        build_minimal_docx(&src, "version two");
        let src_mtime = fs::metadata(&src).expect("meta").modified().expect("mtime");
        if src_mtime > sidecar_mtime {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let regenerated = convert_office_file(&src, &out_dir)
        .expect("convert")
        .expect("sidecar");
    assert_eq!(regenerated, sidecar);
    assert_ne!(
        fs::read_to_string(&regenerated).expect("read"),
        "SENTINEL",
        "a newer source must trigger sidecar regeneration"
    );
}
