# Using `graphify`

`graphify` turns a folder of source code, documentation, papers, images, or videos into a queryable knowledge graph.
Point it at a directory, get back three files:

```text
graphify-out/
├── graph.json       full graph — query without re-reading your files
├── graph.html       open in any browser — interactive viz
└── GRAPH_REPORT.md  key concepts, surprising connections, suggested questions
```

This is the Rust reimplementation of the Python `graphify` reference; the CLI surface is 1:1 with `python -m graphify`.

---

## Install

Build from source (requires Rust 1.90+):

```bash
git clone https://github.com/bunkerlab-net/graphify
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

### `query "<question>"`

BFS traversal that scores nodes against the question and returns a scoped subgraph (typically far smaller than
`GRAPH_REPORT.md` or raw grep output).

```bash
graphify query "how do we authenticate users"
graphify query "..." --dfs                          # depth-first instead of breadth-first
graphify query "..." --budget 4000                  # cap output at N tokens (default 2000)
graphify query "..." --context CALLS --context IMPORTS_FROM   # repeatable edge-context filters
```

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

### `export html`, `export obsidian`, `export svg`, `export graphml`

```bash
graphify export html       # interactive D3 viz (different from `tree`)
graphify export obsidian   # vault of per-cluster Markdown notes — open in Obsidian
graphify export svg        # spring-layout static SVG
graphify export graphml    # GraphML for Gephi / yEd / Cytoscape
```

`export neo4j` and `export wiki` are reserved for future plug-ins (Neo4j driver / wiki backend integration).

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
```

The merge driver is what `graphify hook install` registers in `.git/config` so that two branches editing the same
`graph.json` produce a union-merged result instead of a conflict.

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
graphify opencode install      # AGENTS.md + tool.execute.before plugin
graphify aider install         # AGENTS.md section
graphify claw install          # AGENTS.md section (OpenClaw)
graphify droid install         # AGENTS.md section (Factory Droid)
graphify trae install          # AGENTS.md section
graphify kiro install          # .kiro/steering/graphify.md
graphify antigravity install   # .antigravity/ rules
graphify pi install            # global Pi config
```

Each has a matching `... uninstall`. The aggregate `graphify uninstall [--purge]` removes graphify from every detected
platform, optionally also deleting `graphify-out/`.

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
graphify validate <file.json>           # check an extraction JSON against the schema
graphify benchmark                       # token reduction vs naive full-corpus baseline
graphify benchmark other/graph.json
graphify check-update <path>            # cron-safe: exit 0 if graph is fresh, 1 if stale
```

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

### LLM backends

The semantic-extraction layer routes to one of: `gemini`, `kimi`, `claude`, `openai`, `deepseek`, `ollama`, `bedrock`.
The active backend is auto-detected from environment variables:

| Backend    | Required env                                                                                 |
| ---------- | -------------------------------------------------------------------------------------------- |
| `openai`   | `OPENAI_API_KEY`                                                                             |
| `gemini`   | `GOOGLE_API_KEY`                                                                             |
| `claude`   | `ANTHROPIC_API_KEY`                                                                          |
| `kimi`     | `MOONSHOT_API_KEY`                                                                           |
| `deepseek` | `DEEPSEEK_API_KEY`                                                                           |
| `ollama`   | local daemon — set `OLLAMA_HOST` if non-default                                              |
| `bedrock`  | `AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` (+ optional `AWS_SESSION_TOKEN`, `AWS_REGION`) |

Force a backend with `--backend`; override its default model with `--model`.

### Determinism note

`graph.json` is written with `serde_json` configured for `preserve_order`, so node and edge insertion order is preserved
across runs. Cluster IDs come from a deterministic Louvain pass at resolution 1.0. Two extractions of the same input on
the same machine should produce byte-identical JSON; cluster IDs can drift between machines with different rayon thread
counts on graphs with degenerate communities.

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
