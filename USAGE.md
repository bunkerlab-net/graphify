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
```

Optional LLM-driven semantic extraction is wired through `--backend`/`--model`/`--token-budget`/`--max-concurrency`/
`--api-timeout`/`--max-workers` (see `graphify extract --help` and the [LLM backends](#llm-backends) section).

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

### `cluster-only <path>`

Rerun clustering on an existing `graph.json` and regenerate the report and HTML viz. Useful after tweaking cluster
parameters or when you only want to refresh `GRAPH_REPORT.md`.

```bash
graphify cluster-only .
graphify cluster-only . --no-viz                   # skip graph.html (saves time on >5k-node graphs)
graphify cluster-only . --graph other/graph.json   # use a non-default graph location
```

---

## Querying the graph

All query commands default to `graphify-out/graph.json`; pass `--graph <path>` to point elsewhere.

### Edge vocabulary

Every edge in `graph.json` carries a `relation` and (optionally) a `context`.
The query / affected / explain / serve commands filter on these.

**Relations** (`--relation` on `affected`):

| Relation       | Emitted by                                                               |
| -------------- | ------------------------------------------------------------------------ |
| `contains`     | File / config node → contained entity (function, class, `mcp_server`).   |
| `method`       | Class node → method.                                                     |
| `calls`        | Function / method node → callee, resolved within the file or cross-file. |
| `imports`      | File node → imported module.                                             |
| `imports_from` | File node → imported symbol from another file (`from x import y`).       |
| `re_exports`   | Module → re-exported module (`export … from 'x'`).                       |
| `inherits`     | Class → base class. Java's source-level `extends` is normalised here.    |
| `implements`   | Class → interface (Java / C# / TypeScript).                              |
| `references`   | Function / method / class → type referenced in its signature or body.    |
| `requires_env` | MCP server → env-var *name* it depends on (values are never read).       |

`references` edges typically carry a `context` describing *how* the type is
used; older extractors (SQL, for one) still emit `references` without a
`context`, so consumers should treat the field as optional.

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
| `command`        | MCP server → its executable (`npx`, `uvx`, …).                   |
| `package`        | MCP server → npm / pypi package parsed from its args.            |

`parameter_type`, `return_type`, `generic_arg`, and `attribute` are emitted by
the Python, C#, Java, and TypeScript extractors. Other languages emit the
structural relations but skip the per-signature reference pass.

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
(the feedback loop).

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
```

`export neo4j` defaults to writing a Cypher script. With `--push <uri>` it streams nodes and relationships to a live
Neo4j instance. The password can come from `--password` or the `NEO4J_PASSWORD` env var; the user defaults to `neo4j`.

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
graphify serve                       # stdio-only JSON-RPC, mounts graphify-out/graph.json
graphify serve --graph other.json
```

Exposes tools `query_graph`, `get_node`, `get_neighbors`, `get_community`, `god_nodes`, `graph_stats`, `shortest_path`
to any MCP client.

### Per-platform installers

Each writes a section to the platform's instructions file + (where applicable) a tool-use hook that nudges the assistant
toward `graphify query` instead of grepping raw files.

```bash
graphify claude install        # CLAUDE.md + PreToolUse hook (Claude Code)
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
graphify antigravity install   # .antigravity/ rules
graphify pi install            # global Pi config
graphify devin install         # ~/.config/devin/skills/graphify/SKILL.md (Devin CLI)
graphify devin install --project  # .devin/skills/... + .windsurf/rules/graphify.md
```

Each has a matching `... uninstall`.

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

### Environment variables

| Variable                    | Effect                                                           |
| --------------------------- | ---------------------------------------------------------------- |
| `GRAPHIFY_OUT`              | Override the output directory name (default `graphify-out`).     |
| `GRAPHIFY_FORCE`            | Same effect as `--force` on `update`.                            |
| `GRAPHIFY_VIZ_NODE_LIMIT`   | Cap nodes before HTML export is skipped (default 5000).          |
| `GRAPHIFY_GOOGLE_WORKSPACE` | Truthy value enables `.gdoc/.gsheet/.gslides` export by default. |
| `GRAPHIFY_BEDROCK_MODEL`    | Override the default model for the Bedrock backend.              |
| `GRAPHIFY_BEDROCK_BASE_URL` | Override the Bedrock Runtime endpoint URL (mainly for tests).    |
| `GRAPHIFY_CLUSTER_PROGRESS` | Truthy value prints per-level cluster progress to stderr.        |
| `GRAPHIFY_CLUSTER_BACKEND`  | `leiden` (default) or `louvain` to force the fallback.           |

### LLM backends

The semantic-extraction layer routes to one of: `gemini`, `kimi`, `claude`, `openai`, `deepseek`, `ollama`, `bedrock`.
The active backend is auto-detected from environment variables:

| Backend    | Required env                                         |
| ---------- | ---------------------------------------------------- |
| `openai`   | `OPENAI_API_KEY`                                     |
| `gemini`   | `GOOGLE_API_KEY`                                     |
| `claude`   | `ANTHROPIC_API_KEY`                                  |
| `kimi`     | `MOONSHOT_API_KEY`                                   |
| `deepseek` | `DEEPSEEK_API_KEY`                                   |
| `ollama`   | local daemon — set `OLLAMA_HOST` if non-default      |
| `bedrock`  | Any AWS credential-chain entry — see paragraph below |

The Bedrock backend uses `aws-sdk-bedrockruntime`, which resolves credentials through the standard AWS provider chain:
`AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` (with optional `AWS_SESSION_TOKEN`), `AWS_PROFILE` from
`~/.aws/credentials`, `AWS_WEB_IDENTITY_TOKEN_FILE` (IRSA, GitHub OIDC), `AWS_CONTAINER_CREDENTIALS_*` (ECS task
roles), IMDS, and SSO. `AWS_REGION` alone is **not** sufficient to auto-select Bedrock — credentials must also be
present, otherwise auto-detection falls through to the next backend.

Force a backend with `--backend`; override its default model with `--model`.

### Determinism note

`graph.json` is written with `serde_json` configured for `preserve_order`, so node and edge insertion order is preserved
across runs. Cluster IDs come from a deterministic community-detection pass at resolution 1.0 — Leiden by default via
`leiden-rs` with `random_seed=42`, or Louvain when `GRAPHIFY_CLUSTER_BACKEND=louvain` is set. Two extractions of the
same input on the same machine should produce byte-identical JSON; cluster IDs can drift between machines with
different rayon thread counts on graphs with degenerate communities.

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
