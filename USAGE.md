# Using `graphify`

`graphify` turns a folder of source code, documentation, papers, images, or videos into a queryable knowledge graph.
Point it at a directory and you get back a `graphify-out/` folder. The three files most people interact with directly:

```text
graphify-out/
├── graph.json                  full graph — query without re-reading your files
├── graph.html                  open in any browser — interactive viz
└── GRAPH_REPORT.md             key concepts, surprising connections, suggested questions
```

A handful of sidecars accompany them to make incremental runs cheap and to persist
analysis state between invocations:

```text
graphify-out/
├── manifest.json               per-file fingerprint for incremental updates
├── .graphify_root              marker so child runs find the project root
├── .graphify_analysis.json     analysis sidecar feeding GRAPH_REPORT.md
├── .graphify_labels.json       community label cache (skip the LLM next time)
├── stage_02_extract.json       cached extraction output for incremental runs
└── .graphify_semantic_marker   set when semantic extraction has already run
```

Optional output lands under `graphify-out/` only when you opt in: `wiki/`
(`graphify export wiki`), `GRAPH_TREE.html` (`graphify tree`), `cypher.txt`
(`graphify export neo4j`), `<YYYY-MM-DD>/` backups (created automatically when
`graph.json` is overwritten), and `memory/` (Q&A saved by `graphify save-result`).

This is the Rust reimplementation of the Python `graphify` reference; the CLI surface is 1:1 with `python -m graphify`.

---

## Install

Requires Rust 1.95 or newer.

```bash
cargo install --git https://github.com/bunkerlab-net/graphify.git
```

Or build from a local checkout:

```bash
git clone https://github.com/bunkerlab-net/graphify.git
cd graphify
cargo install --path .
```

Or run directly with `cargo run` from the workspace root:

```bash
cargo run -- <subcommand> [args]
```

The rest of this document assumes the binary is on your `PATH` as `graphify`.

## Quick start

From the root of any project:

```bash
graphify extract .
```

That runs the full pipeline (detect → extract → build → cluster → analyze → report → export) and writes `graphify-out/`
next to your code.

Then ask it questions:

```bash
graphify query "where is the rate limiter defined"
graphify path  "request_handler" "database_pool"
graphify explain "AuthMiddleware"
```

Open `graphify-out/graph.html` in a browser to explore visually.

---

## Building and updating the graph

### `extract <path>`

The headless full pipeline. Detects file types, extracts structure with tree-sitter (26+ languages), builds the graph,
clusters it, runs analysis, writes the report, and exports HTML.

```bash
graphify extract .                       # current directory
graphify extract ../some-repo            # any directory
graphify extract . --no-cluster          # raw extraction only
graphify extract . --out /tmp/g          # write to /tmp/g/graphify-out
graphify extract . --google-workspace    # also export .gdoc/.gsheet/.gslides sidecars via gws (requires the optional `gws` Google Workspace export CLI)
graphify extract . --global              # merge result into ~/.graphify/global-graph.json
graphify extract . --global --as my-repo # custom tag for --global
graphify extract . --mode deep           # aggressive INFERRED-edge semantic extraction
graphify extract . --cargo               # also add crate→crate dependency edges from Cargo.toml
graphify extract . --postgres "$DSN"     # also add a live PostgreSQL schema (needs the `postgres` build feature)
```

`--cargo` walks the Cargo workspace/package manifests and emits one `crate:<name>` node per internal package
plus a `crate_depends_on` edge for each internal dependency. `--postgres <DSN>` reconstructs a live database
schema as graph nodes/edges; it is compiled only with the `postgres` build feature
(`cargo install graphify --features postgres`) and otherwise exits with an error. Both augment a normal path
scan (unlike graphify-py, the Rust `extract` keeps `<PATH>` required — point it at an empty directory to
introspect a database alone). Raster images in the corpus reach the LLM as vision input; see
[LLM backends](#llm-backends).

Beyond tree-sitter source files, the AST pass also ingests **package manifests**
(`apm.yml`, `apm.yaml`, `pyproject.toml`, `go.mod`, `pom.xml`) into one canonical
`type=package` node per package plus `depends_on` edges, so a package referenced
across manifests collapses to a single hub node (#1377); **PowerShell modules**
(`.psm1`) and **manifests** (`.psd1`) — `Import-Module`, dot-sourcing, and
`RootModule`/`NestedModules`/`RequiredModules` become `imports_from` edges (#1331);
and **Markdown links** (inline, reference-style, and `[[wikilinks]]`) as
`references` edges, so a hub doc (`index.md`, a table of contents) connects to the
documents it links instead of being an orphan (#1376). Swift `import` targets
become shared `type=module` anchor nodes and cross-file member calls
(`recv.method()`) resolve through the file's local type table (#1327, #1356).

Optional LLM-driven semantic extraction is wired through `--backend`/`--model`/`--mode`/`--token-budget`/
`--max-concurrency`/`--api-timeout`/`--max-workers` (see `graphify extract --help` and the
[LLM backends](#llm-backends) section). `--mode deep` is the only mode beyond the default; it appends a
deep-extraction instruction to the LLM system prompt so the model emits richer `INFERRED` architectural
edges (shared data contracts, lifecycle coupling, multi-step flows). An unknown `--mode` value exits with
status 2.

### `update <path>`

Re-extracts code files only (no LLM round-trip). Use after edits to refresh the graph quickly.

```bash
graphify update .
graphify update . --force         # rebuild even if the new graph has fewer nodes
graphify update . --no-cluster    # skip clustering pass
```

### `watch <path>`

Watch a folder and rebuild on file changes (Code/docs/images). Runs until interrupted.

```bash
graphify watch .
```

The watch path and the post-commit hook share a shrink-guard that refuses to
overwrite `graph.json` when the rebuilt node count drops without explanation —
this catches silent corruption from half-finished extraction chunks. When the
caller hands the rebuilder an explicit list of deleted paths (the post-commit
hook does this from `git diff --name-only HEAD~1 HEAD`), the shrink is treated
as intentional and the guard is skipped — no `--force` needed for delete-heavy
commits.

When several commits land in quick succession, a post-commit hook that cannot
acquire the rebuild lock appends its changed paths to
`graphify-out/.pending_changes` instead of dropping them. The process holding
the lock drains that queue and folds the paths into its own rebuild, so no
commit's changes are lost under contention.

### `cluster-only <path>`

Rerun clustering on an existing `graph.json` and regenerate the report and HTML viz. Useful after tweaking cluster
parameters or when you only want to refresh `GRAPH_REPORT.md`.

When no `.graphify_labels.json` exists yet, `cluster-only` auto-names communities with the configured LLM backend
in a single batched call, falling back to `Community N` placeholders if no backend is configured or the call fails.
An existing labels file is preserved (re-run `graphify label` to force a refresh).

```bash
graphify cluster-only .
graphify cluster-only . --no-viz                   # skip graph.html (saves time on >5k-node graphs)
graphify cluster-only . --graph other/graph.json   # use a non-default graph location
graphify cluster-only . --no-label                 # keep "Community N" placeholders (skip LLM naming)
graphify cluster-only . --backend openai           # backend to use for naming (default: auto-detect)
```

### `label <path>`

`label` is `cluster-only` that **always** (re)names communities with the configured LLM backend, even when a
`.graphify_labels.json` already exists. Use it to refresh names after the graph changed, or to switch backends.

```bash
graphify label .                       # re-name with the auto-detected backend
graphify label . --backend gemini      # force a specific backend
graphify label . --no-viz              # skip graph.html regeneration
```

If no backend is configured (no API key), `label` degrades to `Community N` placeholders and prints a hint.

---

## Querying the graph

All query commands default to `graphify-out/graph.json`; pass `--graph <path>` to point elsewhere.

### Edge vocabulary

Every edge in `graph.json` carries a `relation` and (optionally) a `context`.
The query / affected / explain / serve commands filter on these.

**Relations** (`--relation` on `affected`):

| Relation       | Emitted by                                                                                                                             |
| -------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `contains`     | File / config node → contained entity (function, class, `mcp_server`).                                                                 |
| `method`       | Class node → method.                                                                                                                   |
| `calls`        | Function / method node → callee, resolved within the file or cross-file.                                                               |
| `imports`      | File node → imported module.                                                                                                           |
| `imports_from` | File node → imported symbol from another file (`from x import y`).                                                                     |
| `depends_on`   | Package-manifest node → a dependency package node (`apm.yml` / `pyproject.toml` / `go.mod` / `pom.xml`).                               |
| `re_exports`   | Module → re-exported module (`export … from 'x'`).                                                                                     |
| `inherits`     | Class → base class. Source-level `extends` (Java, Kotlin, Scala, PHP, Swift, Objective-C, Rust supertraits, Julia) is normalized here. |
| `implements`   | Class → interface / protocol (Java, C#, TypeScript, Kotlin, PHP, Swift, Objective-C, Rust trait `impl`).                               |
| `embeds`       | Go struct/interface embedding (anonymous field or embedded interface).                                                                 |
| `mixes_in`     | Trait mixin: PHP `use`, Scala `with`.                                                                                                  |
| `references`   | Function / method / class → type referenced in its signature or body.                                                                  |
| `instantiates` | Caller → BYOND `DreamMaker` type constructed via `new /type` (`.dm`).                                                                  |
| `uses`         | BYOND `.dmm` map → each type path referenced in its tile dictionary.                                                                   |
| `requires_env` | MCP server → env-var _name_ it depends on (values are never read).                                                                     |

`references` edges typically carry a `context` describing _how_ the type is
used; older extractors (SQL, for one) still emit `references` without a
`context`, so consumers should treat the field as optional.

Unresolved imports carry an `external: true` flag (e.g. a BYOND `#include` of a
file outside the corpus). Resolved imports use `imports_from` and omit the flag.

**Contexts** (`--context` on `query`, on `references` edges):

| Context          | Where it comes from                                              |
| ---------------- | ---------------------------------------------------------------- |
| `call`           | Call site.                                                       |
| `field`          | Class field declaration of the referenced type.                  |
| `parameter_type` | Function / method parameter typed with the referenced type.      |
| `return_type`    | Function / method return type.                                   |
| `generic_arg`    | Type argument to a generic (e.g. `Result<Payload>` → `Payload`). |
| `attribute`      | Java `@Annotation` / C# `[Attribute]` decoration.                |
| `import`         | Module / file referenced by an `import` statement.               |
| `export`         | Module re-exported by an `export … from` statement.              |
| `map`            | BYOND `.dmm` map tile → a type path it places.                   |
| `command`        | MCP server → its executable (`npx`, `uvx`, …).                   |
| `package`        | MCP server → npm / pypi package parsed from its args.            |
| `dependency`     | Package-manifest `depends_on` edge to a required package.        |

`parameter_type`, `return_type`, `generic_arg`, and `field` are emitted by the
Python, C#, Java, TypeScript, Go, Rust, Swift, Kotlin, Scala, PHP, C, C++,
Objective-C, Julia, Fortran, and PowerShell extractors. `attribute` is emitted
by the Java `@Annotation` / C# `[Attribute]` passes. A few extractors (SQL, for
one) still emit `references` without a `context`, so consumers should treat the
field as optional.

### `query "<question>"`

BFS traversal that scores nodes against the question and returns a scoped subgraph (typically far smaller than
`GRAPH_REPORT.md` or raw grep output).

```bash
graphify query "how do we authenticate users"
graphify query "..." --dfs                          # depth-first instead of breadth-first
graphify query "..." --budget 4000                  # cap output at N tokens (default 2000)
graphify query "..." --context CALLS --context IMPORTS_FROM   # repeatable edge-context filters
```

`--context` accepts canonical edge-context names (`call`, `field`, `import`,
`export`, `parameter_type`, `return_type`, `generic_arg`, `attribute`) and
their aliases. Matching is case-insensitive and whitespace is trimmed. The
full alias map:

| Canonical name   | Accepted aliases                                                                     |
| ---------------- | ------------------------------------------------------------------------------------ |
| `parameter_type` | `param`, `params`, `parameter`, `parameters`, `arg`, `args`, `argument`, `arguments` |
| `return_type`    | `return`, `returns`, `returned`                                                      |
| `generic_arg`    | `generic`, `generics`, `template`, `templates`                                       |
| `attribute`      | `annotation`, `annotations`, `decorator`, `decorators`                               |
| `call`           | `calls`, `called`, `invoke`, `invocation`                                            |
| `field`          | `fields`, `property`, `properties`, `member`, `members`                              |
| `import`         | `imports`, `imported`, `module`, `modules`                                           |
| `export`         | `exports`, `exported`                                                                |

### `path "<A>" "<B>"`

Shortest path between two nodes.

```bash
graphify path "request_handler" "database_pool"
```

### `explain "<node>"`

Plain-language explanation of a node and its neighbours.

```bash
graphify explain "AuthMiddleware"
```

### `save-result`

Save a Q&A result back into `graphify-out/memory/` so it gets re-extracted into the graph on the next `update`
(the feedback loop). Files under `graphify-out/memory/` are always detected: they bypass `.gitignore` /
`.graphifyignore` filtering, so a broad ignore pattern (e.g. `*.md`) can't silently erase generated memory notes.

```bash
graphify save-result \
    --question "how is auth scoped" \
    --answer   "AuthMiddleware checks tenant_id from JWT and binds it to the request context" \
    --type     query \
    --nodes    AuthMiddleware request_context
```

### `affected "<query>"`

Reverse-traversal impact analysis: given a node label / ID / source-file substring, enumerate every node that
depends on it (via `calls`, `imports`, `imports_from`, `re_exports`, `inherits`, etc.) up to a configurable depth.
A fast pre-flight before refactors and bulk edits.

```bash
graphify affected "AuthMiddleware"
graphify affected "AuthMiddleware" --depth 3                   # default 2
graphify affected "AuthMiddleware" --relation calls --relation imports   # repeatable filter
graphify affected "AuthMiddleware" --graph other-graph.json
```

### `diagnose multigraph`

Read-only diagnostic that reports how many edges in the on-disk graph (or raw extraction) would be silently
collapsed by the simple-graph builder. Useful when investigating whether a corpus is ready for an opt-in
multigraph build.

```bash
graphify diagnose multigraph                                    # text report
graphify diagnose multigraph --json                             # JSON envelope
graphify diagnose multigraph --max-examples 10                  # cap high-multiplicity examples
graphify diagnose multigraph --directed                         # force directed analysis
graphify diagnose multigraph --extract-path graphify-py/graphify/extract.py
```

---

## Visualisation and export

### `tree`

D3 v7 collapsible-tree HTML over the filesystem hierarchy.

```bash
graphify tree                            # → graphify-out/GRAPH_TREE.html
graphify tree --output tree.html
graphify tree --max-children 500 --top-k-edges 20
```

### `export callflow-html`

A self-contained dark-themed architecture page with Mermaid call-flow diagrams per section.

```bash
graphify export callflow-html
graphify export callflow-html --graph other/graph.json --output ARCH.html
```

### `export html`, `export obsidian`, `export svg`, `export graphml`, `export wiki`, `export neo4j`

```bash
graphify export html       # interactive D3 viz (different from `tree`)
graphify export html --no-viz --node-limit 10000      # cap size or skip generation
graphify export obsidian   # vault of per-cluster Markdown notes — open in Obsidian
graphify export svg        # spring-layout static SVG
graphify export graphml    # GraphML for Gephi / yEd / Cytoscape
graphify export wiki       # per-community markdown wiki under graphify-out/wiki/
```

All `graphify export <html|obsidian|wiki|svg|graphml|neo4j>` subcommands prefer
the analysis sidecar at `graphify-out/.graphify_analysis.json`. When the sidecar
is missing or empty — which happens after the watch / post-commit rebuild path
that only refreshes `graph.json` + `GRAPH_REPORT.md` — exports reconstruct the
community map from the per-node `community` attribute on `graph.json`. `export
wiki` only bails out when both sources are empty.

### Protected-graph backups

Before overwriting a graph that carries hand-curated state (semantic marker
or per-community labels), `graphify` copies the existing
`graphify-out/{graph.json,…}` into `graphify-out/<YYYY-MM-DD>/`. The backup is
rate-limited to one folder per day via a `sha256` comparison of `graph.json`:

- If today's backup already exists and its `graph.json` is byte-identical, the
  command is a no-op.
- If the content has changed since today's backup, the existing folder is
  overwritten in place — there is always one backup folder per day, holding
  the latest pre-overwrite state.
- The legacy `<YYYY-MM-DD>_2`, `<YYYY-MM-DD>_3`, … accumulation is gone.

Set `GRAPHIFY_NO_BACKUP=1` to disable backups entirely.

```bash
graphify export neo4j                                            # → graphify-out/cypher.txt (import via cypher-shell)
graphify export neo4j --push bolt://localhost:7687 --password ** # push directly to a live Neo4j instance
graphify export falkordb                                         # → graphify-out/cypher.txt (OpenCypher, FalkorDB-compatible)
graphify export falkordb --push falkordb://localhost:6379        # push directly to a live FalkorDB instance
```

`export neo4j` defaults to writing a Cypher script. With `--push <uri>` it streams nodes and relationships to a live
Neo4j instance. The password can come from `--password` or the `NEO4J_PASSWORD` env var; the user defaults to `neo4j`.

`export falkordb` writes the same `cypher.txt` by default (FalkorDB is OpenCypher-compatible). `--push <falkordb://…>`
streams the graph into a live FalkorDB instance via `GRAPH.QUERY` and is compiled only with the `falkordb` build
feature (`cargo install graphify --features falkordb`); without it, `--push` exits with an error while the
`cypher.txt` path still works. Optional `--user` / `--password` (or `FALKORDB_PASSWORD`) supply auth.

---

## Cross-repo and merging

### Global graph

A single `~/.graphify/global-graph.json` that aggregates multiple project graphs, keyed by repo tag.

```bash
graphify global add ./graphify-out/graph.json            # uses parent dir name as tag
graphify global add ./graphify-out/graph.json --as my-svc
graphify global list                                     # tag, node count, edge count, added_at
graphify global remove my-svc
graphify global path                                     # prints location of global-graph.json
```

### Merging

```bash
graphify merge-graphs a/graph.json b/graph.json --out merged.json
graphify merge-driver <base> <current> <other>     # configured as a git merge driver
graphify merge-chunks chunk1.json chunk2.json --out merged.json    # combine extraction JSON chunks
graphify merge-semantic --cached cached.json --new fresh.json --out out.json   # reuse cached semantic data
```

The merge driver is what `graphify hook install` registers in `.git/config` so that two branches editing the same
`graph.json` produce a union-merged result instead of a conflict.

`merge-chunks` is used when a large repo was extracted across multiple worker chunks (the headless pipeline calls it
internally; you only need it directly when wiring up custom CI fan-out). `merge-semantic` combines a cached semantic
extraction with a fresh one — the cache provides untouched files, the fresh side provides changed files.

### Adding external content

```bash
graphify add https://example.com/spec.pdf            # fetched into ./raw/, extracted on next update
graphify add https://example.com/post --author "Jane Doe" --contributor "team"
graphify clone https://github.com/owner/repo          # cloned to ~/.graphify/repos/<owner>/<repo>
graphify clone https://github.com/owner/repo --branch dev --out ./vendor/repo
```

---

## Editor and AI-assistant integration

### Git hooks

```bash
graphify hook install      # registers post-commit + post-checkout + merge driver
graphify hook status
graphify hook uninstall
```

The hook is a fast "needs update" gate — it does **not** re-run the LLM extraction on every commit. It marks the graph
stale; you re-extract on demand.

### MCP server (Claude Desktop, Codex, etc.)

```bash
graphify serve                       # stdio JSON-RPC (default), mounts graphify-out/graph.json
graphify serve --graph other.json
graphify serve --transport http      # Streamable HTTP on http://127.0.0.1:8080/mcp
graphify serve --transport http --host 0.0.0.0 --port 9000 --api-key SECRET
graphify serve --transport http --json-response   # single JSON reply instead of an SSE stream
```

Exposes tools `query_graph`, `get_node`, `get_neighbors`, `get_community`, `god_nodes`, `graph_stats`, `shortest_path`
to any MCP client.

The default `stdio` transport is the per-developer mode. The **Streamable HTTP**
transport (MCP spec 2025-03-26) lets one shared process host the graph for a
team; it is compiled only with the `http` build feature
(`cargo install graphify --features http`) and otherwise `--transport http`
exits with an error. Flags: `--host` / `--port` (bind address, default
`127.0.0.1:8080`), `--api-key` (required via `Authorization: Bearer <key>` or
`X-API-Key: <key>`; env `GRAPHIFY_API_KEY`), `--path` (mount path, default
`/mcp`), `--json-response` (one `application/json` reply instead of an SSE
stream), `--stateless` (skip per-session ids), and `--session-timeout`
(accepted for compatibility; a no-op since graphify keeps no per-session state).
Binding `0.0.0.0` without an `--api-key` prints a warning.

### Per-platform installers

Each writes a section to the platform's instructions file + (where applicable) a tool-use hook that nudges the assistant
toward `graphify query` instead of grepping raw files.

```bash
graphify claude install        # CLAUDE.md + two PreToolUse hooks (Bash search + Read/Glob)
graphify codebuddy install     # CODEBUDDY.md + .codebuddy/settings.json PreToolUse hooks + skill
graphify gemini install        # GEMINI.md + BeforeTool hook (Gemini CLI)
graphify cursor install        # .cursor/rules/graphify.mdc
graphify vscode install        # .github/copilot-instructions.md + skill copy
graphify copilot install       # ~/.copilot/skills (GitHub Copilot CLI)
graphify codex install         # AGENTS.md section
graphify amp install           # AGENTS.md section (Amp)
graphify opencode install      # AGENTS.md + tool.execute.before plugin
                               #   user scope: ~/.config/opencode/skills/graphify/SKILL.md
                               #   --project:  ./.opencode/skills/graphify/SKILL.md
graphify aider install         # AGENTS.md section
graphify claw install          # AGENTS.md section (OpenClaw)
graphify droid install         # AGENTS.md section (Factory Droid)
graphify trae install          # AGENTS.md section
graphify trae-cn install       # AGENTS.md section (Trae CN)
graphify hermes install        # AGENTS.md section (Hermes)
graphify kiro install          # .kiro/steering/graphify.md
graphify kilo install          # native skill + /graphify command (~/.config/kilo) +
                               #   AGENTS.md + .kilo/plugins/graphify.js (tool.execute.before)
graphify antigravity install   # ~/.gemini/config/skills/graphify/SKILL.md + .agents/rules + .agents/workflows
                               #   --project:  ./.agents/skills/... + .agents/rules + .agents/workflows
graphify pi install            # global Pi config
graphify devin install         # ~/.config/devin/skills/graphify/SKILL.md (Devin CLI)
graphify devin install --project  # .devin/skills/... + .windsurf/rules/graphify.md
```

Each has a matching `... uninstall`. `graphify claude uninstall` (and the
aggregate `graphify uninstall`) now also remove the installed user-scope skill
tree (`~/.claude/skills/graphify/`), not just the `CLAUDE.md` section — matching
`gemini uninstall`. `graphify kilo uninstall` removes the global skill/command,
the `.kilo` plugin, and deregisters it from `.kilo/kilo.json` (an existing
`.kilo/kilo.jsonc` is read but never rewritten, so user comments are preserved).

#### Project-scoped installs (`--project`)

Every platform install/uninstall subcommand accepts `--project`, which writes the skill under the current
working directory (e.g. `./.claude/skills/graphify/SKILL.md`) instead of the home directory. The CLAUDE.md
registration uses a relative path, and the installer prints a `git add` hint so the new files are easy to
commit into the repo:

```bash
graphify claude install --project          # writes ./.claude/skills/graphify/SKILL.md
graphify claude uninstall --project        # removes only the project-scoped install
graphify install --platform claude --project   # same via the aggregate dispatcher
```

Use this for repos where every contributor should get the graphify skill installed automatically when they
clone, without depending on the user-global home install. The user-global install (no `--project`) remains
available and is untouched by the project flag.

### Aggregate install / uninstall

```bash
graphify install --platform claude    # same as `graphify claude install`
graphify install claude               # positional shorthand also accepted
graphify uninstall                    # removes graphify from every detected platform
graphify uninstall --purge            # also deletes graphify-out/
```

The aggregate `install` is a convenience dispatcher to the per-platform installer; the aggregate `uninstall` scans
every supported platform and removes the integration wherever it finds one.

### `hook-check`

Silent gate run on every editor tool use. Returns 0 when the graph is fresh, prints a one-line `additionalContext` hint
when stale. Not normally invoked by humans — the per-platform hooks call it.

---

## PR triage

```bash
graphify prs                       # all PRs in the current repo
graphify prs --repo owner/repo --limit 50
```

Uses the local `gh` CLI to enumerate PRs, joins their changed files against the graph, and ranks by impact.

---

## Utilities

```bash
graphify validate <file.json>            # check an extraction JSON against the schema
graphify benchmark                        # token reduction vs naive full-corpus baseline
graphify benchmark other/graph.json
graphify check-update <path>             # cron-safe: exit 0 if graph is fresh, 1 if stale
graphify cache-check files.txt --root .  # report which files have a fresh semantic cache entry
```

`cache-check` reads a newline-separated list of file paths from `files.txt` (resolved relative to `--root`) and prints
which ones already have a valid entry in the semantic cache. Useful in CI to decide whether a re-extraction is worth
the LLM spend.

---

## Configuration

### File selection (`.graphifyignore` / `.graphifyinclude`)

`detect` — and therefore `extract` / `update` / `watch` — honours two optional control files at the scan root, on
top of any `.gitignore`:

- **`.graphifyignore`** — gitignore-syntax exclude list. Same last-match-wins and parent-exclusion rules as
  `.gitignore` (a `!` re-include cannot rescue a file whose ancestor directory is excluded). Loaded from the scan
  root up to the VCS root.
- **`.graphifyinclude`** — gitignore-syntax **allowlist** that re-includes files an ignore rule would otherwise
  drop. A file is kept when it (or an ancestor directory) matches an include pattern, even if `.graphifyignore` /
  `.gitignore` excludes it. Anchored directory stems cover their whole subtree — both `/src` and the globbed
  `/src*` pull in `src/deep/main.py`. The sensitive-file guard still runs after the allowlist, so an include can
  never pull secrets (`.env`, private keys, etc.) into the corpus.

Files under `graphify-out/memory/` are always detected regardless of either file. (`graphify-py` ships the
`.graphifyinclude` matcher but never wires it into `detect`, so the allowlist is inert there; the Rust port
completes the feature.)

### Environment variables

| Variable                         | Effect                                                                                                                                              |
| -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `GRAPHIFY_OUT`                   | Override the output directory name (default `graphify-out`).                                                                                        |
| `GRAPHIFY_FORCE`                 | Same effect as `--force` on `update`.                                                                                                               |
| `GRAPHIFY_VIZ_NODE_LIMIT`        | Cap nodes before HTML export is skipped (default 5000).                                                                                             |
| `GRAPHIFY_GOOGLE_WORKSPACE`      | Truthy value enables `.gdoc/.gsheet/.gslides` export by default.                                                                                    |
| `GRAPHIFY_BEDROCK_MODEL`         | Override the default model for the Bedrock backend.                                                                                                 |
| `GRAPHIFY_BEDROCK_BASE_URL`      | Override the Bedrock Runtime endpoint URL (mainly for tests).                                                                                       |
| `GRAPHIFY_CLAUDE_CLI_MODEL`      | Model passed to `claude -p --model` for the `claude-cli` backend (e.g. `haiku`, `sonnet`, or a full model id). Unset uses claude-cli's own default. |
| `OPENAI_BASE_URL`                | Point the `openai` backend at any OpenAI-compatible server (llama.cpp, vLLM, LM Studio). `GRAPHIFY_OPENAI_BASE_URL` still wins.                     |
| `OPENAI_MODEL`                   | Default model for the `openai` backend. `--model` and `GRAPHIFY_OPENAI_MODEL` still win.                                                            |
| `ANTHROPIC_BASE_URL`             | Point the `claude` backend at a custom Anthropic-compatible endpoint (LiteLLM proxy, gateways). `GRAPHIFY_CLAUDE_BASE_URL` still wins.              |
| `ANTHROPIC_MODEL`                | Default model for the `claude` backend. `--model` and `GRAPHIFY_CLAUDE_MODEL` still win.                                                            |
| `GRAPHIFY_CLUSTER_PROGRESS`      | Truthy value prints per-level cluster progress to stderr.                                                                                           |
| `GRAPHIFY_CLUSTER_BACKEND`       | `leiden` (default) or `louvain` to force the fallback.                                                                                              |
| `GRAPHIFY_ALLOW_LOCAL_PROVIDERS` | Opt in to loading a project-local `.graphify/providers.json` (ignored by default; see Custom providers).                                            |
| `OLLAMA_BASE_URL`                | Ollama endpoint (default `http://localhost:11434/v1`); a link-local/cloud-metadata host is refused, a general non-loopback host warns.              |

### LLM backends

The semantic-extraction layer routes to one of: `gemini`, `kimi`, `claude`, `openai`, `deepseek`, `ollama`,
`bedrock`, `azure`. The active backend is auto-detected from environment variables:

| Backend    | Required env                                         |
| ---------- | ---------------------------------------------------- |
| `openai`   | `OPENAI_API_KEY`                                     |
| `gemini`   | `GOOGLE_API_KEY`                                     |
| `claude`   | `ANTHROPIC_API_KEY`                                  |
| `kimi`     | `MOONSHOT_API_KEY`                                   |
| `deepseek` | `DEEPSEEK_API_KEY`                                   |
| `ollama`   | local daemon — set `OLLAMA_BASE_URL` if non-default  |
| `bedrock`  | Any AWS credential-chain entry — see paragraph below |
| `azure`    | `AZURE_OPENAI_API_KEY` + `AZURE_OPENAI_ENDPOINT`     |

The Azure OpenAI backend posts to `{endpoint}/openai/deployments/{model}/chat/completions` with an `api-key`
header. The deployment/model resolves from `--model`, else `AZURE_OPENAI_DEPLOYMENT` / `GRAPHIFY_AZURE_MODEL`
(default `gpt-4o`); the API version defaults to `2024-12-01-preview` (override with `AZURE_OPENAI_API_VERSION`).

**Vision.** `claude`, `openai`, `gemini`, `kimi`, and `bedrock` send raster images (PNG/JPG/GIF/WebP) as visual
input, and `claude-cli` reads them by path via its Read tool; other backends (including `azure`, `deepseek`, and
custom providers) record each image as a text-reference node so it still becomes a graph node. Ollama is opt-in via
`GRAPHIFY_OLLAMA_VISION=1` once a vision-capable model is selected.

The Bedrock backend uses `aws-sdk-bedrockruntime`, which resolves credentials through the standard AWS provider chain:
`AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` (with optional `AWS_SESSION_TOKEN`), `AWS_PROFILE` from
`~/.aws/credentials`, `AWS_WEB_IDENTITY_TOKEN_FILE` (IRSA, GitHub OIDC), `AWS_CONTAINER_CREDENTIALS_*` (ECS task
roles), IMDS, and SSO. `AWS_REGION` alone is **not** sufficient to auto-select Bedrock — credentials must also be
present, otherwise auto-detection falls through to the next backend.

Force a backend with `--backend`; override its default model with `--model`.

The `openai` and `claude` backends additionally honour `OPENAI_BASE_URL` /
`OPENAI_MODEL` and `ANTHROPIC_BASE_URL` / `ANTHROPIC_MODEL`, so they can target a
self-hosted OpenAI-compatible server (llama.cpp, vLLM, LM Studio) or an
Anthropic-compatible proxy/gateway (LiteLLM) without registering a custom provider
(#1273). The `GRAPHIFY_*` overrides and `--model` still take precedence.

#### Custom providers

Any OpenAI-compatible endpoint can be registered as a custom backend and used like a built-in one (e.g.
`graphify extract . --backend nvidia`, `graphify label . --backend nvidia`). At extraction/labeling time custom
providers are loaded from `~/.graphify/providers.json` (user-global) only. A **project-local**
`.graphify/providers.json` travels with a cloned/shared repo and decides where your corpus and API key are sent,
so it is **ignored by default** (with a stderr warning) and loaded only when `GRAPHIFY_ALLOW_LOCAL_PROVIDERS=1`
(or `true`/`yes`) is set — in which case it takes precedence over the global registry on a name clash. The
`provider` command manages the user-global registry:

```bash
graphify provider add nvidia \
  --base-url https://integrate.api.nvidia.com/v1 \
  --default-model minimaxai/minimax-m2.7 \
  --env-key NVIDIA_API_KEY \
  [--pricing-input 0.0 --pricing-output 0.0]
graphify provider list                 # name + base URL of each registered provider
graphify provider show nvidia          # full JSON config for one provider
graphify provider remove nvidia
```

Built-in backend names cannot be shadowed, and a registry entry is ignored unless it supplies a non-empty
`base_url`, `default_model`, and `env_key`. The `base_url` must use an `http`/`https` scheme (a custom provider
receives the full corpus plus the API key, so a non-`http(s)` `base_url` is rejected); plaintext `http` to a
non-loopback host loads but warns. Auto-detection consults custom providers **after** all built-ins, in
registry order, selecting the first whose `--env-key` variable is set. Missing `pricing` defaults to zero so cost
estimation never fails; an optional `max_completion_tokens` caps extraction output (default 8192).

### Determinism note

`graph.json` is written with `serde_json` configured for `preserve_order`, so node and edge insertion order is preserved
across runs. Cluster IDs come from a deterministic community-detection pass at resolution 1.0 — Leiden by default via
`leiden-rs` with `random_seed=42`, or Louvain when `GRAPHIFY_CLUSTER_BACKEND=louvain` is set. Communities are ordered
by size descending with a lexicographic tiebreak on their sorted member list, so an identical grouping always receives
identical integer community IDs run-to-run (no spurious "community churn" in a per-node cid diff). Two extractions of the
same input on the same machine should produce byte-identical JSON.

---

## Common workflows

### One-off: explore an unfamiliar repo

```bash
cd unfamiliar-repo
graphify extract .
open graphify-out/graph.html        # interactive viz
graphify query "what does this project do"
```

### Daily: keep the graph fresh while working

```bash
graphify hook install               # one-time
graphify watch . &                  # rebuilds on save
# ...edit...
graphify query "where do we set the rate limit"
```

### Team: share a queryable knowledge graph

```bash
graphify hook install               # auto-merges graph.json across branches
git add graphify-out/
git commit -m "Add knowledge graph"
```

Teammates pull, run `graphify update .` to refresh, and `graphify query "..."` against the shared graph.

### AI-assistant: stop grepping, start querying

```bash
graphify claude install             # or gemini/cursor/codex/...
graphify extract .
```

Your assistant will now run `graphify query` before reaching for `grep`/`rg`/`find`.

---

## Getting help

Every subcommand supports `--help`:

```bash
graphify --help
graphify extract --help
graphify export --help
graphify export callflow-html --help
```
