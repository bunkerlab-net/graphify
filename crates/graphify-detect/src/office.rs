//! Office-file and PDF text-extraction helpers.
//!
//! Ports `extract_pdf_text`, `docx_to_markdown`, `xlsx_to_markdown`,
//! `xlsx_extract_structure`, and `convert_office_file` from
//! `graphify-py/graphify/detect.py`.
//!
//! All functions return empty/default values on failure, mirroring the Python
//! `except Exception: return ""` pattern so that a malformed or unsupported
//! file never aborts a corpus scan.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use hex;
use sha2::{Digest as _, Sha256};

use crate::error::DetectError;

// ── PDF extraction ────────────────────────────────────────────────────────────

/// Extract plain text from a PDF file.
///
/// Pages are concatenated with a form-feed character (`\x0c`) between them,
/// matching `pypdf`'s `page.extract_text()` join convention.
///
/// Returns an empty string on any error or if the PDF contains no extractable
/// text (e.g. scanned image-only PDF). Never propagates an error to the caller.
#[must_use]
pub fn extract_pdf_text(path: &Path) -> String {
    extract_pdf_text_inner(path).unwrap_or_default()
}

/// Inner fallible PDF extraction. Public wrapper [`extract_pdf_text`] converts
/// errors to an empty string so a bad file never aborts a corpus scan.
fn extract_pdf_text_inner(path: &Path) -> Result<String, DetectError> {
    use lopdf::Document;

    // lopdf::Document::load returns a lopdf error — map to DetectError::Office.
    let doc = Document::load(path).map_err(|e| DetectError::Office(e.to_string()))?;

    let mut pages: Vec<String> = Vec::new();
    // `Document::get_pages()` returns `BTreeMap<u32, ObjectId>` keyed by the
    // 1-indexed page number, which is the form `extract_text` expects.
    for page_num in doc.get_pages().keys().copied() {
        // extract_text returns Result; skip pages that error or are blank.
        if let Ok(text) = doc.extract_text(&[page_num])
            && !text.is_empty()
        {
            pages.push(text);
        }
    }

    // Python joins with "\n" between pages (pypdf appends \n per page).
    // We use "\x0c" (form-feed) as per the task spec, which is the conventional
    // page separator for programmatic text extraction.
    Ok(pages.join("\x0c"))
}

// ── DOCX → Markdown ───────────────────────────────────────────────────────────
//
// The DOCX parser is split into a `DocxState` struct that carries the
// accumulating context, plus small per-event handlers. Splitting the work this
// way keeps `parse_docx_xml` short enough to satisfy `clippy::too_many_lines`
// and lets the per-event logic be read independently.

/// State-machine context used while parsing a `word/document.xml` stream.
///
/// Only the variants actively pushed onto `DocxState::ctx_stack` need to
/// exist; an empty stack already represents the document-root context.
#[derive(Debug, Clone, PartialEq)]
enum DocxContext {
    Paragraph,
    Run,
    Table,
    Row,
    Cell,
}

/// Mutable accumulator threaded through the DOCX event loop.
///
/// All paragraph/run/table/cell scratchpads live here so the event handler
/// methods can update them in place without long argument lists.
#[derive(Debug, Default)]
struct DocxState {
    lines: Vec<String>,
    ctx_stack: Vec<DocxContext>,
    current_para_style: String,
    current_run_text: String,
    current_para_text: String,
    table_rows: Vec<Vec<String>>,
    current_row: Vec<String>,
    current_cell_text: String,
    in_run_pr: bool,
    in_para_pr: bool,
}

/// Convert a `.docx` file to Markdown text.
///
/// Paragraphs are separated by blank lines. Heading styles produce `#`/`##`/`###`
/// prefixes; List-style paragraphs produce `- ` bullets; plain paragraphs are
/// output as-is. Tables are rendered as Markdown pipe tables.
///
/// Returns an empty string on any error (e.g. file not found, corrupt zip).
#[must_use]
pub fn docx_to_markdown(path: &Path) -> String {
    docx_to_markdown_inner(path).unwrap_or_default()
}

/// Inner fallible DOCX-to-Markdown conversion. Public wrapper
/// [`docx_to_markdown`] converts errors to an empty string.
fn docx_to_markdown_inner(path: &Path) -> Result<String, DetectError> {
    use std::io::BufReader;
    use zip::ZipArchive;

    let file = std::fs::File::open(path).map_err(DetectError::Io)?;
    let reader = BufReader::new(file);
    let mut archive = ZipArchive::new(reader).map_err(|e| DetectError::Office(e.to_string()))?;

    // Read word/document.xml
    let xml = {
        let mut entry = archive
            .by_name("word/document.xml")
            .map_err(|e| DetectError::Office(e.to_string()))?;
        let mut buf = String::new();
        entry.read_to_string(&mut buf).map_err(DetectError::Io)?;
        buf
    };

    parse_docx_xml(&xml)
}

/// Drive the `word/document.xml` event loop, dispatching to per-event helpers.
///
/// Splitting the work into small handlers keeps each function under the
/// 100-line clippy budget and makes the event semantics easy to follow.
fn parse_docx_xml(xml: &str) -> Result<String, DetectError> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut state = DocxState::default();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e) | Event::Empty(ref e)) => handle_docx_start(&mut state, e),
            Ok(Event::End(ref e)) => handle_docx_end(&mut state, e),
            Ok(Event::Text(ref e)) => handle_docx_text(&mut state, e),
            Ok(Event::Eof) => break,
            Err(e) => return Err(DetectError::Office(e.to_string())),
            _ => {}
        }
        buf.clear();
    }

    Ok(state.lines.join("\n"))
}

/// Handle a `Start` or `Empty` event by pushing context and reading attributes.
///
/// Splitting this out keeps `parse_docx_xml` within the clippy line budget and
/// makes the start-tag dispatch independently testable.
fn handle_docx_start(state: &mut DocxState, e: &quick_xml::events::BytesStart) {
    // Bind the QName so its borrowed bytes outlive the inner `match`.
    let qname = e.name();
    match local_name(qname.as_ref()) {
        "pPr" => state.in_para_pr = true,
        "rPr" => state.in_run_pr = true,
        "p" => {
            // A paragraph inside a table cell is structurally identical for
            // our purposes; we only care that it pushes the Paragraph ctx.
            state.ctx_stack.push(DocxContext::Paragraph);
            state.current_para_text.clear();
            state.current_para_style.clear();
        }
        "r" => {
            state.ctx_stack.push(DocxContext::Run);
            state.current_run_text.clear();
        }
        "tbl" => {
            state.ctx_stack.push(DocxContext::Table);
            state.table_rows.clear();
        }
        "tr" => {
            state.ctx_stack.push(DocxContext::Row);
            state.current_row.clear();
        }
        "tc" => {
            state.ctx_stack.push(DocxContext::Cell);
            state.current_cell_text.clear();
        }
        "pStyle" if state.in_para_pr => {
            // The Word style name lives in the `val` attribute of `<w:pStyle>`.
            for attr in e.attributes().flatten() {
                if local_name(attr.key.as_ref()) == "val" {
                    state.current_para_style = String::from_utf8_lossy(&attr.value).into_owned();
                }
            }
        }
        _ => {}
    }
}

/// Handle a `End` event by popping context and flushing accumulated text.
///
/// Splitting this out keeps `parse_docx_xml` within the clippy line budget.
fn handle_docx_end(state: &mut DocxState, e: &quick_xml::events::BytesEnd) {
    let qname = e.name();
    match local_name(qname.as_ref()) {
        "pPr" => state.in_para_pr = false,
        "rPr" => state.in_run_pr = false,
        "r" => {
            if !state.current_run_text.is_empty() {
                state.current_para_text.push_str(&state.current_run_text);
                state.current_run_text.clear();
            }
            state.ctx_stack.pop();
        }
        "p" => flush_docx_paragraph(state),
        "tc" => {
            state
                .current_row
                .push(state.current_cell_text.trim().to_string());
            state.current_cell_text.clear();
            state.ctx_stack.pop();
        }
        "tr" => {
            state
                .table_rows
                .push(std::mem::take(&mut state.current_row));
            state.ctx_stack.pop();
        }
        "tbl" => {
            render_table_into(&state.table_rows, &mut state.lines);
            state.table_rows.clear();
            state.ctx_stack.pop();
        }
        _ => {}
    }
}

/// Decide whether a finished paragraph belongs to a table cell or to the
/// document body, and flush its text accordingly.
///
/// Extracted from `handle_docx_end` so the per-event matcher stays compact.
fn flush_docx_paragraph(state: &mut DocxState) {
    let text = state.current_para_text.trim().to_string();
    let in_cell = state
        .ctx_stack
        .iter()
        .rev()
        .skip(1)
        .any(|c| *c == DocxContext::Cell);
    if in_cell {
        if !state.current_cell_text.is_empty() {
            state.current_cell_text.push(' ');
        }
        state.current_cell_text.push_str(&text);
    } else {
        let line = render_para_line(&state.current_para_style, &text);
        state.lines.push(line);
    }
    state.ctx_stack.pop();
    state.current_para_text.clear();
    state.current_para_style.clear();
}

/// Append a text event's characters to the run-text scratchpad.
///
/// quick-xml 0.40 dropped `BytesText::unescape()` from the default-features
/// build, so we decode lossily then call the free `quick_xml::escape::unescape`
/// to resolve named entities.
fn handle_docx_text(state: &mut DocxState, e: &quick_xml::events::BytesText) {
    if state.in_run_pr
        || state.in_para_pr
        || !matches!(state.ctx_stack.last(), Some(DocxContext::Run))
    {
        return;
    }
    let raw = String::from_utf8_lossy(e.as_ref()).into_owned();
    let txt = quick_xml::escape::unescape(&raw)
        .map(std::borrow::Cow::into_owned)
        .unwrap_or(raw);
    state.current_run_text.push_str(&txt);
}

/// Map a Word paragraph style name to a Markdown prefix and return the full line.
///
/// Only Heading 1–3 and List styles need a prefix; everything else is emitted
/// as plain text to avoid introducing spurious Markdown structure.
fn render_para_line(style: &str, text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    if style.starts_with("Heading 1") {
        format!("# {text}")
    } else if style.starts_with("Heading 2") {
        format!("## {text}")
    } else if style.starts_with("Heading 3") {
        format!("### {text}")
    } else if style.starts_with("List") {
        format!("- {text}")
    } else {
        text.to_string()
    }
}

/// Render a 2-D grid of cell strings as a Markdown pipe table and append to `lines`.
///
/// The first row is treated as the header; a separator row is inserted
/// automatically so the output is valid GitHub Flavored Markdown.
fn render_table_into(rows: &[Vec<String>], lines: &mut Vec<String>) {
    if rows.is_empty() {
        return;
    }
    let header = format!("| {} |", rows[0].join(" | "));
    let sep = format!(
        "| {} |",
        rows[0]
            .iter()
            .map(|_| "---")
            .collect::<Vec<_>>()
            .join(" | ")
    );
    lines.push(header);
    lines.push(sep);
    for row in rows.iter().skip(1) {
        lines.push(format!("| {} |", row.join(" | ")));
    }
}

/// Strip an XML namespace prefix (`w:p` → `p`, `r:t` → `t`).
///
/// Word XML uses namespace-prefixed tag names (`w:p`, `w:r`, etc.). Stripping
/// the prefix lets the event handlers match on the bare local name, which is
/// language-independent and simpler to read.
fn local_name(raw: &[u8]) -> &str {
    let s = std::str::from_utf8(raw).unwrap_or("");
    if let Some(pos) = s.find(':') {
        &s[pos + 1..]
    } else {
        s
    }
}

// ── XLSX → Markdown ───────────────────────────────────────────────────────────

/// Convert an `.xlsx` file to Markdown text.
///
/// Each sheet is rendered as a `## Sheet: <name>` heading followed by a
/// Markdown pipe table. Empty rows are skipped. Returns an empty string on any
/// error.
#[must_use]
pub fn xlsx_to_markdown(path: &Path) -> String {
    xlsx_to_markdown_inner(path).unwrap_or_default()
}

/// Inner fallible XLSX-to-Markdown conversion. Public wrapper
/// [`xlsx_to_markdown`] converts errors to an empty string.
fn xlsx_to_markdown_inner(path: &Path) -> Result<String, DetectError> {
    use calamine::{Data, Reader as _, Xlsx, open_workbook};

    // `open_workbook` is generic over the reader type; the turbofish locks it
    // to `Xlsx` so calamine knows which workbook kind to load.
    let mut wb: Xlsx<_> =
        open_workbook::<Xlsx<_>, _>(path).map_err(|e| DetectError::Office(e.to_string()))?;

    let sheet_names: Vec<String> = wb.sheet_names().clone();
    let mut sections: Vec<String> = Vec::new();

    for sheet_name in sheet_names {
        let sheet = wb
            .worksheet_range(&sheet_name)
            .map_err(|e| DetectError::Office(e.to_string()))?;

        let rows: Vec<Vec<String>> = sheet
            .rows()
            .filter(|row| row.iter().any(|cell| !matches!(cell, Data::Empty)))
            .map(|row| {
                row.iter()
                    .map(|cell| match *cell {
                        Data::Empty => String::new(),
                        Data::String(ref s)
                        | Data::DateTimeIso(ref s)
                        | Data::DurationIso(ref s) => s.clone(),
                        Data::Float(f) => format_float(f),
                        Data::Int(i) => i.to_string(),
                        Data::Bool(b) => b.to_string(),
                        Data::Error(ref e) => format!("{e:?}"),
                        Data::DateTime(dt) => dt.to_string(),
                    })
                    .collect()
            })
            .collect();

        if rows.is_empty() {
            continue;
        }

        sections.push(format!("## Sheet: {sheet_name}"));
        let header = format!("| {} |", rows[0].join(" | "));
        let sep = format!(
            "| {} |",
            rows[0]
                .iter()
                .map(|_| "---")
                .collect::<Vec<_>>()
                .join(" | ")
        );
        sections.push(header);
        sections.push(sep);
        for row in rows.iter().skip(1) {
            sections.push(format!("| {} |", row.join(" | ")));
        }
    }

    Ok(sections.join("\n"))
}

/// Format a floating-point XLSX cell value, dropping the `.0` suffix for integers.
///
/// Calamine represents integer cells as `Data::Float`, so without this helper
/// the value `42` would render as `"42.0"` in the Markdown table.
fn format_float(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e15 {
        #[allow(clippy::cast_possible_truncation)]
        return (f as i64).to_string();
    }
    f.to_string()
}

// ── XLSX structure extraction ─────────────────────────────────────────────────

/// One sheet's structural summary.
#[derive(Debug, Clone)]
pub struct SheetInfo {
    /// Sheet name.
    pub name: String,
    /// Column header strings from the first non-empty row.
    pub columns: Vec<String>,
    /// Number of data rows (excluding the header row).
    pub row_count: usize,
}

/// Structural summary of an `.xlsx` file.
#[derive(Debug, Clone, Default)]
pub struct XlsxStructure {
    pub sheets: Vec<SheetInfo>,
}

/// Extract sheet names, column headers, and row counts from an `.xlsx` file.
///
/// Returns a default empty structure on any error.
#[must_use]
pub fn xlsx_extract_structure(path: &Path) -> XlsxStructure {
    xlsx_extract_structure_inner(path).unwrap_or_default()
}

/// Inner fallible XLSX structure extraction. Public wrapper
/// [`xlsx_extract_structure`] converts errors to an empty default struct.
fn xlsx_extract_structure_inner(path: &Path) -> Result<XlsxStructure, DetectError> {
    use calamine::{Data, Reader as _, Xlsx, open_workbook};

    // `open_workbook` is generic over the reader type; the turbofish locks it
    // to `Xlsx` so calamine knows which workbook kind to load.
    let mut wb: Xlsx<_> =
        open_workbook::<Xlsx<_>, _>(path).map_err(|e| DetectError::Office(e.to_string()))?;

    let sheet_names: Vec<String> = wb.sheet_names().clone();
    let mut sheets: Vec<SheetInfo> = Vec::new();

    for sheet_name in sheet_names {
        let sheet = wb
            .worksheet_range(&sheet_name)
            .map_err(|e| DetectError::Office(e.to_string()))?;

        let mut rows = sheet
            .rows()
            .filter(|row| row.iter().any(|cell| !matches!(cell, Data::Empty)));

        let columns: Vec<String> = rows
            .next()
            .map(|header_row| {
                header_row
                    .iter()
                    .map(|cell| match cell {
                        Data::String(s) => s.clone(),
                        Data::Empty => String::new(),
                        other => other.to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let row_count = rows.count();

        sheets.push(SheetInfo {
            name: sheet_name,
            columns,
            row_count,
        });
    }

    Ok(XlsxStructure { sheets })
}

// ── Office-file dispatcher ─────────────────────────────────────────────────────

/// Convert a `.docx` or `.xlsx` to a Markdown sidecar file in `out_dir`.
///
/// Returns the path of the written `.md` sidecar, or `None` when:
/// - the extension is not `.docx` or `.xlsx`,
/// - the extracted text is empty/whitespace-only, or
/// - any I/O error occurs.
///
/// The sidecar filename is `<stem>_<sha256_8hex>.md`, matching Python's
/// `hashlib.sha256(str(path.resolve()).encode()).hexdigest()[:8]` convention.
///
/// # Errors
///
/// Returns `DetectError` on I/O failure when writing the sidecar.
pub fn convert_office_file(path: &Path, out_dir: &Path) -> Result<Option<PathBuf>, DetectError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    let text = match ext.as_str() {
        "docx" => docx_to_markdown(path),
        "xlsx" => xlsx_to_markdown(path),
        _ => return Ok(None),
    };

    if text.trim().is_empty() {
        return Ok(None);
    }

    std::fs::create_dir_all(out_dir).map_err(DetectError::Io)?;

    // Stable name derived from the resolved absolute path (mirrors Python).
    // Normalize separators to `/` so a manifest written on Windows and
    // read on Unix (or vice versa) produces the same sidecar name.
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let normalized = resolved.to_string_lossy().replace('\\', "/");
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let digest = hex::encode(hasher.finalize());
    let name_hash = &digest[..8];

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document");
    let out_path = out_dir.join(format!("{stem}_{name_hash}.md"));

    let content = format!(
        "<!-- converted from {} -->\n\n{text}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("document")
    );
    std::fs::write(&out_path, content).map_err(DetectError::Io)?;

    Ok(Some(out_path))
}
