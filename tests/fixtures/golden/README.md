# Golden Fixtures Manifest

These fixtures were generated from the Python graphify reference implementation
and are used for byte-identity (and structured-comparison) testing against the
Rust port.

## Generation metadata

| Field | Value |
|-------|-------|
| Python version | 3.14.5 |
| graphify-py git SHA | `076e6b7c06d0018a027ecc37249d82518999f639` |
| graphifyy package version | 0.8.14 |
| Generated on | 2026-05-22 |
| Generation script | `/tmp/gen_fixtures.py` |

## Corpora

### `tiny/`

**Purpose:** Smoke test — minimal viable input.

**Contents:**
- `hello.py` — one `hello(name)` function, ~7 lines

**Pipeline result:** 3 nodes, 2 edges, 1 community

**Generation command (equivalent):**
```python
detection = detect(Path("tests/fixtures/inputs/tiny"))
result = extract([Path("tests/fixtures/inputs/tiny/hello.py")], cache_root=...)
G = build([result], root=...)
communities = cluster(G)
```

---

### `single_python_file/`

**Purpose:** Single-file Python extraction — classes, methods, module-level functions, docstrings as rationale nodes.

**Contents:**
- `shapes.py` — `Point`, `Circle`, `Rectangle`, `Triangle` geometry classes with area/perimeter methods.

**Pipeline result:** 24 nodes (after 8 deduped), 43 edges, 5 communities

---

### `multi_python_modules/`

**Purpose:** Cross-file Python import resolution. 4 modules with explicit `from X import Y` dependencies.

**Contents:**
- `models.py` — `User`, `Task`, `Project`, `Priority`, `Status`
- `storage.py` — `UserStore`, `TaskStore`, `ProjectStore` (imports from `models`)
- `service.py` — `TaskService` (imports from `models` and `storage`)
- `cli.py` — `run_demo()` (imports from `models` and `service`)

**Pipeline result:** 51 nodes (after 7 deduped), 86 edges, 7 communities

**Notable:** Cross-file import edges are tagged `INFERRED`.

---

### `mixed_languages/`

**Purpose:** Multi-language corpus. Tests the Python + TypeScript + Go extractors
alongside a Markdown document.

**Contents:**
- `server.py` — HTTP server (Python)
- `client.ts` — API client (TypeScript)
- `proxy.go` — Reverse proxy (Go)
- `README.md` — Architecture documentation

**Pipeline result:** 34 nodes, 36 edges, 6 communities

---

### `docs_only/`

**Purpose:** Markdown-only corpus. Tests the heading-based AST extractor for `.md` files.
Despite the name, graphify *does* extract nodes from Markdown headings.

**Contents:**
- `overview.md` — Project overview
- `architecture.md` — Broker cluster design
- `api.md` — REST API reference

**Pipeline result:** 23 nodes (after 2 deduped), 21 edges, 5 communities

**Note:** All 3 files are classified as `document` by `detect()`, but the
markdown AST extractor is still invoked via `_get_extractor()`.

---

### `per_language_samples/`

**Purpose:** Language coverage — exercises 9+ language extractors using fixtures
copied directly from `graphify-py/tests/fixtures/`.

**Contents:**
- `sample.py` — Python
- `sample_calls.py` — Python with cross-function calls
- `sample.go` — Go
- `sample.rs` — Rust
- `sample.ts` — TypeScript
- `sample.tsx` — TypeScript/React
- `dynamic_import.ts` — TypeScript dynamic imports
- `typescript_advanced.ts` — Advanced TypeScript patterns
- `sample.java` — Java
- `sample.kt` — Kotlin
- `sample.rb` — Ruby
- `crate_a/src/lib.rs` — Rust (multi-file crate)
- `crate_b/src/lib.rs` — Rust (multi-file crate)

**Pipeline result:** 76 nodes (after 3 deduped), 88 edges, 11 communities

---

## Directory Layout

```
tests/fixtures/golden/<corpus>/
  inputs.txt                # sorted `find . -type f` of input corpus
  stage_01_detect.json      # output of detect() - file list + word counts
  stage_02_extract.json     # combined AST extraction - all nodes + edges
  stage_03_graph.json       # NetworkX node-link format (pre-clustering)
  stage_04_clustered.json   # graph with community attrs + community map
  stage_05_analysis.json    # god_nodes, surprising_connections, questions
  GRAPH_REPORT.md           # human-readable summary
  graph.json                # official export via to_json()
  graph.html                # interactive HTML visualization
```

## Non-Deterministic Fields

### GRAPH_REPORT.md — date in title

Every report header contains the generation date:
```
# Graph Report - <corpus>  (YYYY-MM-DD)
```

**Rust test strategy:** Strip or regex-ignore the date when comparing.
Pattern: `\(\d{4}-\d{2}-\d{2}\)`

---

### stage_01_detect.json — absolute file paths

The `scan_root`, and all paths in `files.code`, `files.document`, etc., are
absolute paths on the generating machine.

Example:
```json
{
  "scan_root": "/Users/robbie/Documents/projects/bunkerlab/graphify/tests/fixtures/inputs/tiny",
  "files": {
    "code": ["/Users/robbie/.../tiny/hello.py"]
  }
}
```

**Rust test strategy:** Compare basenames / relative tails only. Or compare
only the file *counts* and *types*, not full paths.

---

### graph.json — `built_at_commit` (optional)

Present only when the pipeline is run inside a git repository. These fixtures
were generated with inputs in a non-git directory so the field is absent.
Rust code should treat it as `Option<String>`.

---

### Node/edge ordering in stage_03_graph.json and stage_04_clustered.json

NetworkX `node_link_data()` emits nodes in dict insertion order, which depends
on Python dict ordering. This is stable for a given run but the field-level
ordering within each node/edge dict should not be relied upon for byte comparison.

**Rust test strategy:** Compare nodes and edges as unordered collections keyed
on `id` (for nodes) and `(source, target, relation)` tuple (for edges).

---

### stage_05_analysis.json — `surprising_connections` ordering

The surprise list is sorted by score. Scores involve betweenness centrality
which is deterministic but may produce ties resolved by dict ordering.

**Rust test strategy:** Compare as unordered set of `{source, target}` pairs.

---

### Cohesion score floating-point representation

Cohesion scores are exact rationals (actual_edges / possible_edges) stored as
Python floats rounded to 6 decimal places. Rust `f64` may differ at the
last bit due to different float printing.

**Rust test strategy:** Allow ±1e-6 tolerance on `cohesion_scores` values.

---

## All Stages Ran Successfully

| Corpus | Nodes | Edges | Communities | detect | extract | build | cluster | analyze | export |
|--------|-------|-------|-------------|--------|---------|-------|---------|---------|--------|
| tiny | 3 | 2 | 1 | OK | OK | OK | OK | OK | OK |
| single_python_file | 24 | 43 | 5 | OK | OK | OK | OK | OK | OK |
| multi_python_modules | 51 | 86 | 7 | OK | OK | OK | OK | OK | OK |
| mixed_languages | 34 | 36 | 6 | OK | OK | OK | OK | OK | OK |
| docs_only | 23 | 21 | 5 | OK | OK | OK | OK | OK | OK |
| per_language_samples | 76 | 88 | 11 | OK | OK | OK | OK | OK | OK |
