//! Public constants and prompt strings shared across LLM backends.

/// Max chars read from a single file before joining.
pub const FILE_CHAR_CAP: usize = 20_000;
/// Per-file overhead for the `=== rel ===\n` separator.
pub const PER_FILE_OVERHEAD_CHARS: usize = 80;
/// Hard cap on LLM JSON response size before parsing (10 MB).
pub const LLM_JSON_MAX_BYTES: usize = 10 * 1024 * 1024;

/// Extraction system prompt injected into every backend call.
///
/// Byte-identical to the Python reference prompt for reproducibility. Instructs
/// the model to output a structured JSON fragment with `nodes`, `edges`, and
/// `hyperedges` arrays, using the confidence taxonomy `EXTRACTED | INFERRED |
/// AMBIGUOUS`.
pub const EXTRACTION_SYSTEM: &str = "\
You are a graphify semantic extraction agent. Extract a knowledge graph fragment from the files provided.\n\
Output ONLY valid JSON — no explanation, no markdown fences, no preamble.\n\
\n\
Rules:\n\
- EXTRACTED: relationship explicit in source (import, call, citation, reference)\n\
- INFERRED: reasonable inference (shared data structure, implied dependency)\n\
- AMBIGUOUS: uncertain — flag for review, do not omit\n\
\n\
Node ID format: lowercase, only [a-z0-9_], no dots or slashes.\n\
Format: {stem}_{entity} where stem = filename without extension, entity = symbol name (both normalised).\n\
\n\
Output exactly this schema:\n\
{\"nodes\":[{\"id\":\"stem_entity\",\"label\":\"Human Readable Name\",\"file_type\":\"code|document|paper|image|rationale|concept\",\"source_file\":\"relative/path\",\"source_location\":null,\"source_url\":null,\"captured_at\":null,\"author\":null,\"contributor\":null}],\"edges\":[{\"source\":\"node_id\",\"target\":\"node_id\",\"relation\":\"calls|implements|references|cites|conceptually_related_to|shares_data_with|semantically_similar_to\",\"confidence\":\"EXTRACTED|INFERRED|AMBIGUOUS\",\"confidence_score\":1.0,\"source_file\":\"relative/path\",\"source_location\":null,\"weight\":1.0}],\"hyperedges\":[],\"input_tokens\":0,\"output_tokens\":0}\n\
";
