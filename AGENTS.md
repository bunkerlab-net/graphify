# Agents

This repository contains a Rust workspace that is a complete 1:1 reimplementation of
`./graphify-py` (a git submodule pointing at the Python reference). Subagents working
on individual module ports MUST follow these conventions.

## Rules

- Format code: `cargo fmt`
- Lint code: `cargo clippy --all-targets --all-features`
- Run tests: `cargo nextest`
- Run pre-commit hooks: `hk check`
- Fix failing hooks: `hk fix`
- CodeRabbit Review: `coderabbit review --agent --base master`
- Test coverage report: `cargo llvm-cov nextest`
- Do not use `cargo check`, use `cargo clippy` instead
- Any test that interacts with the filesystem **MUST** be isolated and use a temporary directory

## Workspace layout

```text
graphify/
├── Cargo.toml                 # workspace root — DO NOT modify unless adding a NEW crate
├── crates/
│   ├── graphify-<module>/
│   │   ├── Cargo.toml         # add per-crate deps here
│   │   ├── src/lib.rs         # port lives here
│   │   └── tests/parity.rs    # parity tests against graphify-py/tests/test_<module>.py
├── graphify-py/               # READ-ONLY reference (git submodule). Never edit.
└── src/main.rs                # graphify CLI binary
```

## Strict lints (deny-level)

All crates inherit lints via `[lints] workspace = true`

Practical impact:

- When deciding to suppress a lint, **give genuine attempt at resolving the issue first**
  - **Only** then suppress the lint with a valid reason why with an inline comment
- `.unwrap()` and `.expect("...")` are forbidden in non-test code. Use `?` propagation
  or explicit `match` arms. For `Lazy<Regex>` of a known-good literal pattern, document
  the invariant in a comment and add `#[allow(clippy::unwrap_used)]` on the closure.
- `pedantic = "deny"` is harsh — write idiomatic Rust:
  - Pass small types by value, not reference.
  - Use `#[must_use]` on pure functions returning owned values.
  - Use `&str` not `String` in function signatures unless ownership is required.
  - Use `iter().copied()` over `iter().cloned()` for `Copy` types.
- In test files (`tests/*.rs`), prefer `?` returning `Result`. Where unavoidable, suppress
  per-block: `#![allow(clippy::expect_used, clippy::unwrap_used)]` at file top is OK
  for `tests/parity.rs`.

## Porting conventions

1. **Read the Python module + its corresponding test file together.** They are the spec.
2. **Map Python types to Rust idiomatically:**
   - `dict` → `serde_json::Value`, `indexmap::IndexMap<String, Value>`, or a struct
     with `#[derive(Serialize, Deserialize)]`. Use `IndexMap` (not `HashMap`) anywhere
     ordering is observable (i.e., anywhere outputs are serialized).
   - `list` → `Vec<T>`
   - `set` → `indexmap::IndexSet<T>` for stable ordering
   - `pathlib.Path` → `std::path::PathBuf` / `&Path`
   - `Optional[T]` → `Option<T>`
   - Raised exceptions → `thiserror::Error` enum per crate
3. **Mirror exception messages byte-for-byte where tests assert on them.** Python tests
   commonly use `pytest.raises(..., match="substring")` — the substring must appear in
   the Rust `Display` output.
4. **Use `serde_json` with `features = ["preserve_order"]`** (already in
   `workspace.dependencies`) so JSON output ordering matches Python's `json.dumps`.
5. **Reuse workspace-level crates from `[workspace.dependencies]`.** Reference them as
   `name = { workspace = true }`. Add new deps to your crate's Cargo.toml only — never
   to the workspace root unless explicitly approved.

## Test conventions

- Each crate has `tests/parity.rs` containing 1:1 ports of the matching
  `graphify-py/tests/test_<module>.py`.
- Use `tempfile::tempdir()` for filesystem tests (matches Python's `tmp_path` fixture).
- For HTTP tests, use `mockito`. The mockito server binds to `127.0.0.1`, which the
  SSRF guard in `graphify-security` rejects. Use
  `graphify_security::test_support::fetch_allow_private` for those tests.
- Network-dependent tests should be prefixed `net_` so they can be isolated.

## What you must NOT touch

- `graphify-py/` (the submodule).
- The workspace root `Cargo.toml`'s `[workspace.dependencies]` or `[workspace.lints]`
  sections, unless your task explicitly requires adding a new dep — in which case
  add it to your crate's `Cargo.toml` first and only escalate to workspace if it's
  shared by multiple crates.

## Definition of done (per crate)

- `cargo nextest run -p graphify-<name>` passes.
- `cargo clippy -p graphify-<name> --all-targets -- -D warnings` passes with zero warnings.
- Coverage for the crate as reported by `cargo llvm-cov` is ≥ 95% (line + branch).
- The corresponding Python test file's test cases all have Rust equivalents in
  `tests/parity.rs`.
