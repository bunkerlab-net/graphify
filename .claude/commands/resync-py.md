# Resync graphify with graphify-py

Analyse all commits in `./graphify-py` since the last pinned submodule commit and port the applicable changes to this
Rust workspace.

## How to use

```bash
/resync-py
```

## Instructions

### Phase 0 — Branch and commit the submodule update

Do this before any analysis so the submodule pointer is captured on a dedicated branch
regardless of whether Rust changes follow.

> **Tracking branch:** `graphify-py` is pinned to the `v8` branch of the upstream
> repo (see `.gitmodules`), **not** `main`. `git submodule update --remote` reads
> that branch from `.gitmodules` automatically — there's nothing to pass on the
> command line. If upstream cuts a new major (e.g. `v9`) and we want to follow it,
> that is a separate, deliberate change: edit `.gitmodules`, run
> `git submodule sync graphify-py`, then re-run Phase 0. Do **not** silently
> upgrade the tracked branch as part of a routine resync.

1. Record the current (old) submodule commit and confirm the tracked branch:

   ```bash
   git submodule status graphify-py
   git config -f .gitmodules submodule.graphify-py.branch    # should print: v8
   ```

2. Advance the submodule along the tracked branch:

   ```bash
   git submodule update --init --remote graphify-py
   ```

3. Identify the new commit hash:

   ```bash
   git diff graphify-py
   ```

   The `+Subproject commit <new>` line gives the full hash. Use the first 7 characters as `<short>`.

4. Create a branch named after the new commit and commit the pointer:

   ```bash
   git checkout -b resync/graphify-py/<short>
   git add graphify-py
   git commit -m "Update graphify-py submodule to <short>"
   ```

### Phase 1 — Discover what changed in Python

Use `<old>` (from Phase 0 step 1) and `<new>` (from Phase 0 step 3) to diff the Python changes.

1. List all new commits:

   ```bash
   git -C ./graphify-py log --oneline <old>..<new>
   ```

2. Diff the interesting directory (check stat first; full diff can exceed 30 KB):

   ```bash
   git -C ./graphify-py diff <old>..<new> --stat -- graphify/ tests/
   ```

   Then diff each interesting file individually.

3. Group changes by theme: security, features, bug fixes, documentation, tests.

### Phase 2 — Determine what's applicable to Rust

Not every Python change has a Rust equivalent. Skip:

- Python build system changes (`pyproject.toml`, `requirements.txt`, `uv.lock`)
- Python-specific idioms with no Rust equivalent (e.g. `usedforsecurity=False` on md5)
- Pure documentation changes that describe Python-only behaviour
- Python test framework changes (pytest fixtures, conftest plumbing)

**Do port:**

- New or updated CLI commands, flags, or subcommand behaviour
- New extractor/detector/ingest format support
- Security hardening (SSRF guard rules, input validation, error sanitization,
  query/size limits)
- Changes to skip-dir lists, file filters, or mining heuristics
- Bug fixes to logic that exists in both codebases
- Changes to the JSON output schema (validate / build_from_json / fixtures) —
  byte-identical golden parity is a hard goal
- New parity test cases in `graphify-py/tests/test_<module>.py` — port them into
  the matching crate's `tests/parity.rs`

### Phase 3 — Plan

For each applicable change, identify:

- Which crate(s) need to change (see mapping table below)
- Whether the change is a new file, a modification, or a constant update
- Any new dependencies required (per-crate `Cargo.toml`, not the workspace root)

Present the plan grouped into work units before writing any code.

### Phase 4 — Implement

Work through the plan unit by unit. After each unit, verify the touched crate with:

```bash
cargo clippy -p graphify-<name> --all-targets -- -D warnings
cargo nextest run -p graphify-<name>
```

After all units are complete, run the full workspace gates:

```bash
cargo fmt
cargo clippy --all-targets --all-features
cargo nextest run
hk check
```

Per `AGENTS.md`:

- Never use `cargo check` — always `cargo clippy`.
- Tests that touch the filesystem MUST use `tempfile::tempdir()`.
- Tests live in dedicated `tests/*.rs` files, never inline.
- HTTP tests that hit `127.0.0.1` (mockito) must use
  `graphify_security::test_support::fetch_allow_private` to bypass the SSRF guard.

### Phase 5 — Update documentation

1. Update `///` doc-comments on all modified functions/modules to reflect new behaviour.
2. Update `README.md` and `USAGE.md`:
   - CLI reference (new commands/flags/subcommands)
   - Supported extraction/detection formats
   - Workspace tree (new crates)
   - Any differences from `graphify-py` worth calling out

### Phase 6 — Commit

The submodule pointer is already committed from Phase 0. Stage only the Rust changes and commit:

```text
Resync with graphify-py @ <short-hash>

Ports: <bullet list of what was ported>
```

If there are no Rust changes to port, no additional commit is needed — Phase 0 is the only commit.

### Phase 7 — CodeRabbit review (required before push)

Before pushing any code, run the CodeRabbit CLI against the committed changes:

```bash
coderabbit review --agent --base main --type committed
```

`--base main` matches this repo's default branch. If the default branch has been
renamed, substitute its name — the CLI compares the working branch against
`--base` to decide what to review.

If the CLI itself fails to run (network outage, expired auth, CLI build
issues), do not silently skip Phase 7:

- Re-try the command once after sanity-checking connectivity, auth/token, and
  CLI version (`coderabbit --version`).
- If it still fails, document the failure and any manual verification you ran in
  a follow-up commit message and the PR body. Escalate to the user (the project
  owner, addressed as "Tech Priest" in conversation per the global
  `CLAUDE.md`) before pushing.

Address every issue CodeRabbit raises:

- Apply fixes as new commits on the same branch (do **not** amend prior commits).
  Each round of fixes ships as its own commit so the review history is preserved
  in `git log` and CodeRabbit's iterative findings stay auditable. Amending would
  collapse that trail and rewrite hashes that prior CodeRabbit comments referenced.
- Re-run `coderabbit review --agent --base main --type committed` after every fix
  or dispute commit so the next review sees the resolution.
- Phase 8 may proceed only when every finding from the latest review is resolved
  (either fixed or documented as false positives with user/project-owner approval).

If a finding looks like a false positive or you disagree with it:

- Document the deviation in the commit message of a follow-up commit (or the PR
  body once Phase 8 opens the PR), quoting the relevant CodeRabbit finding text
  and the reason it does not apply.
- Escalate when uncertain: ask the user (the project owner; see `CLAUDE.md`
  for the project's preferred form of address) to confirm the dispute before
  pushing, rather than silently dismissing the finding.
- Re-run `coderabbit review --agent --base main --type committed` after
  documenting the dispute so the new commit is on record.

### Phase 8 — Push and open a PR

Only after CodeRabbit has signed off — meaning the last
`coderabbit review --agent --base main --type committed` invocation exited 0
**and** its JSONL output ends with
`{"type":"complete","status":"review_completed","findings":0}` (or the
remaining findings are documented as approved false positives in a
preceding commit):

1. Push the branch to the remote.
2. Open a PR following the project's standard PR workflow.

## Key file mappings (Python → Rust)

Each Python module in `graphify-py/graphify/` maps to a crate under `crates/`.
The crate's `src/lib.rs` is the entry point and `tests/parity.rs` is the 1:1
port of `graphify-py/tests/test_<module>.py`.

| Python file                         | Rust crate              |
| ----------------------------------- | ----------------------- |
| `graphify/analyze.py`               | `graphify-analyze`      |
| `graphify/benchmark.py`             | `graphify-benchmark`    |
| `graphify/build.py`                 | `graphify-build`        |
| `graphify/cache.py`                 | `graphify-cache`        |
| `graphify/cluster.py`               | `graphify-cluster`      |
| `graphify/dedup.py`                 | `graphify-dedup`        |
| `graphify/detect.py`                | `graphify-detect`       |
| `graphify/export.py`                | `graphify-export`       |
| `graphify/extract.py`               | `graphify-extract`      |
| `graphify/global_graph.py`          | `graphify-global`       |
| `graphify/google_workspace.py`      | `graphify-google`       |
| `graphify/hooks.py`                 | `graphify-hooks`        |
| `graphify/callflow_html.py`         | `graphify-html`         |
| `graphify/tree_html.py`             | `graphify-html`         |
| `graphify/ingest.py`                | `graphify-ingest`       |
| `graphify/llm.py`                   | `graphify-llm`          |
| `graphify/manifest.py`              | `graphify-manifest`     |
| `graphify/prs.py`                   | `graphify-prs`          |
| `graphify/report.py`                | `graphify-report`       |
| `graphify/security.py`              | `graphify-security`     |
| `graphify/serve.py`                 | `graphify-serve`        |
| `graphify/transcribe.py`            | `graphify-transcribe`   |
| `graphify/validate.py`              | `graphify-validate`     |
| `graphify/watch.py`                 | `graphify-watch`        |
| `graphify/wiki.py`                  | `graphify-wiki`         |
| `graphify/__main__.py`, `cli.py`    | `src/main.rs`           |
| `graphify/skill*.md`                | N/A (agent skill docs)  |

If `graphify-py` adds a brand-new top-level module with no matching crate,
escalate to the Tech Priest before introducing a new workspace member — adding
a crate edits the workspace root `Cargo.toml`, which `AGENTS.md` flags as a
gated change.

## Common porting patterns

- Python `dict` with observable key order → `indexmap::IndexMap<String, Value>`
  (never `HashMap`). Anywhere JSON is emitted, ordering is observable.
- Python `set` with observable order → `indexmap::IndexSet<T>`.
- Python `list` → `Vec<T>`.
- Python `pathlib.Path` → `std::path::PathBuf` (owned) or `&Path` (borrowed).
- Python `Optional[T]` → `Option<T>`.
- Python `raise SomeError("msg")` → `thiserror`-derived error enum on the crate.
  When a `pytest.raises(..., match="substring")` exists, the substring **must**
  appear byte-for-byte in the Rust `Display` output.
- Python `json.dumps(...)` → `serde_json::to_string[_pretty]` with the
  `preserve_order` feature (already enabled at the workspace level).
- Python `requests.get(url)` → workspace HTTP client behind
  `graphify-security`'s SSRF guard. Tests against mockito must use
  `graphify_security::test_support::fetch_allow_private`.
- Python `hashlib.sha256(...)` for deterministic IDs → `sha2::Sha256::digest`,
  hex-encoded.
- Python `sys.exit(1)` inside library code → return `Result::Err`; only the
  CLI binary (`src/main.rs`) maps errors to process exit codes.
- Python `logger.exception(...)` → `eprintln!` to stderr. Stdout is reserved for
  structured output (JSON, parsable text).

## Lint gotchas specific to this workspace

`AGENTS.md` sets `pedantic = "deny"` and bans `.unwrap()` / `.expect("...")` in
non-test code. When porting:

- Use `?` propagation or explicit `match` arms. Prefer `.expect("invariant")`
  over `.unwrap()` only where the invariant is documented in a comment.
- For `Lazy<Regex>` of a known-good literal pattern, document the invariant and
  apply `#[allow(clippy::unwrap_used)]` on the closure — never globally.
- In `tests/*.rs`, the file-top `#![allow(clippy::expect_used, clippy::unwrap_used)]`
  is acceptable. Prefer `?`-returning `#[test] fn foo() -> anyhow::Result<()>`
  signatures where practical.
- Pass `Copy` types (`u32`, `bool`, small enums) by value, not by reference.
- Use `&str` over `String` in function signatures unless ownership is required.
- Use `.iter().copied()` over `.iter().cloned()` for `Copy` types.

## Definition of done (per resync)

- `cargo nextest run` passes across the workspace.
- `cargo clippy --all-targets --all-features` is warning-free.
- `hk check` passes.
- `cargo llvm-cov nextest` shows ≥ 95% line + branch coverage for every crate
  that was touched.
- Every new parity test in `graphify-py/tests/test_<module>.py` has a Rust
  equivalent in the matching crate's `tests/parity.rs`.
- CodeRabbit's final review reports zero findings (or all findings are
  documented as approved false positives).
