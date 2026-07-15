# graphify

Turn any folder of source code, documentation, papers, images, or videos into a queryable knowledge graph.

Point `graphify` at a directory and you get back a `graphify-out/` folder. The
three files most people interact with directly:

```text
graphify-out/
├── graph.json                  full graph — query without re-reading your files
├── graph.html                  open in any browser — interactive viz
└── GRAPH_REPORT.md             key concepts, surprising connections, suggested questions
```

A handful of sidecars alongside them carry intermediate state for incremental
runs, the report, and assistant integration:

```text
graphify-out/
├── manifest.json               per-file fingerprint for incremental updates
├── .graphify_root              marker so child runs find the project root
├── .graphify_analysis.json     analysis sidecar feeding GRAPH_REPORT.md
├── .graphify_labels.json       community label cache (skip the LLM next time)
├── .graphify_learning.json     work-memory overlay — learned verdicts for report/serve/explain/viz (graphify reflect)
├── stage_02_extract.json       cached extraction output for incremental runs
└── .graphify_semantic_marker   set when semantic extraction has already run
```

Additional output is written under `graphify-out/` only when you ask for it:
`wiki/` (per-community articles, `graphify export wiki`), `GRAPH_TREE.html`
(`graphify tree`), `cypher.txt` (`graphify export neo4j`), `<YYYY-MM-DD>/`
backups (when `graph.json` is overwritten), and `memory/` (saved Q&A from
`graphify save-result`).

Then ask it questions instead of grepping:

```bash
graphify query "where is the rate limiter defined"
graphify path  "request_handler" "database_pool"
graphify explain "AuthMiddleware"
```

This is the Rust reimplementation of [`graphify`](https://github.com/safishamsi/graphify). The CLI surface is 1:1
with the Python reference — every public command, output file, and observable side-effect of `python -m graphify` has
a Rust equivalent, and outputs are byte-identical where the test suite asserts it.

## Features

- **26+ languages**, parsed with tree-sitter: Rust, Python, TypeScript, JavaScript, Go, Java, C, C++, C#, Ruby, PHP,
  Swift, Kotlin, Scala, Bash, Lua, Elixir, Haskell, OCaml, Zig, Solidity, R, Julia, HTML, CSS, SQL, …
  Vue / Svelte / Astro single-file components (`.vue`, `.svelte`, `.astro`) are parsed through their `<script>` blocks (#1468).
  Also reads .NET project files (`.sln`, `.csproj`, `.fsproj`, `.vbproj`), Razor components
  (`.razor`, `.cshtml`) for package, project-reference, target-framework, and `@code` extraction,
  and WPF/UWP XAML (`.xaml`) for `x:Class`, named controls, `{Binding}` paths, and ViewModel /
  code-behind resolution (#1460),
  Verilog/SystemVerilog (`.v`, `.sv`, `.svh`), BYOND DreamMaker
  (`.dm`, `.dme` source plus `.dmi` icon sheets, `.dmm` maps, and `.dmf` interface forms),
  CUDA (`.cu`, `.cuh`) and Metal (`.metal`) routed through the C++ extractor (#1480),
  and MCP config files (`.mcp.json`,
  `claude_desktop_config.json`, `mcp.json`, `mcp_servers.json`) — servers, commands, packages,
  and env-var _names_ (values are never read).
- **Package manifests & doc links** — `apm.yml`, `pyproject.toml`, `go.mod`, and `pom.xml` become canonical
  `package` nodes with `depends_on` edges (a package referenced across manifests collapses to one hub node);
  PowerShell `.psm1` modules and `.psd1` manifests emit `imports_from` edges; and Markdown links (inline,
  reference-style, and `[[wikilinks]]`) become `references` edges so hub docs (`index.md`, tables of contents)
  connect to the documents they link instead of orphaning.
- **Documents, papers, images, video** — PDF, DOCX, audio transcription, OCR, Google Workspace exports.
  Untrusted office/PDF files are screened before parsing (50 MiB on-disk cap; `.docx`/`.xlsx` zip-bomb guard at
  512 MiB decompressed / 200:1 ratio) and silently skipped if they exceed the limits, so a malicious corpus file
  cannot OOM a scan.
- **Local-first** — `graph.json` lives next to your code; no daemon, no cloud, no account.
- **Optional LLM-driven semantic extraction** through OpenAI, Claude (Anthropic), Gemini, DeepSeek, Kimi (Moonshot),
  Ollama, Bedrock, Azure OpenAI, or any OpenAI-compatible **custom provider** registered with `graphify provider add`.
  The `--backend` identifiers are `openai`, `claude`, `gemini`, `deepseek`, `kimi`, `ollama`, `bedrock`, and `azure`.
  The `openai` and `claude` backends also honour `OPENAI_BASE_URL`/`OPENAI_MODEL` and
  `ANTHROPIC_BASE_URL`/`ANTHROPIC_MODEL` to reach self-hosted OpenAI-compatible servers (llama.cpp, vLLM,
  LM Studio) or Anthropic-compatible proxies (LiteLLM) without a custom-provider entry.
  Vision-capable backends read raster images (PNG/JPG/GIF/WebP) as pixels, so a diagram or screenshot becomes a graph
  node; non-vision backends record it as a text-reference node instead.
- **Structural introspection** — `graphify extract --cargo` adds `crate -> crate` dependency edges from `Cargo.toml`
  manifests; `--postgres <DSN>` adds a live PostgreSQL schema (requires the `postgres` build feature).
- **LLM community naming** — `graphify label` (or `cluster-only`) auto-names graph communities with the configured
  backend, fanning out batches in parallel (`--max-concurrency`, `--batch-size`); degrades to `Community N`
  placeholders when no backend is available.
- **AI-assistant integration** — drop-in installers for Claude Code, CodeBuddy, Codex, Amp, Cursor, Gemini CLI,
  GitHub Copilot, VS Code, OpenCode, Aider, Factory Droid, Trae, Hermes, Kiro, Kilo Code, Pi, Devin CLI,
  Google Antigravity, and more.
- **MCP server** for any MCP-capable assistant (`graphify serve`) — stdio by default, or Streamable HTTP
  (`--transport http`, requires the `http` build feature) so one shared process can host the graph for a team.
- **Git hooks + merge driver** so two branches editing the same `graph.json` produce a union-merged result.
- **Cross-repo global graph** — aggregate every project you care about into one `~/.graphify/global-graph.json`.
- **Work memory** — `graphify save-result` records Q&A outcomes under `graphify-out/memory/`, and `graphify reflect`
  aggregates them into a deterministic `reflections/LESSONS.md` lessons doc (refreshed automatically by the
  post-commit/post-checkout hooks).
- **Deterministic outputs** — same inputs on the same machine produce byte-identical JSON.

## Install

Requires Rust 1.95 or newer (`rustup toolchain install stable`).

```bash
cargo install --git https://github.com/bunkerlab-net/graphify.git
```

Or build from a local checkout:

```bash
git clone https://github.com/bunkerlab-net/graphify.git
cd graphify
cargo install --path .
```

Verify:

```bash
graphify --version
graphify --help
```

## Quick start

From the root of any project:

```bash
graphify extract .
```

That runs the full pipeline (detect → extract → build → cluster → analyze → report → export) and writes
`graphify-out/` next to your code. Open `graphify-out/graph.html` to explore visually, or query the graph from the
command line:

```bash
graphify query "how do we authenticate users"
graphify explain "AuthMiddleware"
```

No LLM key? Build a code-only graph from the local AST alone — it skips the
semantic (LLM) pass and the doc/paper/image files, so a mixed repo still gets a
full code graph offline:

```bash
graphify extract . --code-only
```

Wire it into your AI assistant in one command:

```bash
graphify claude install      # or gemini / cursor / codex / copilot / vscode / ...
```

Your assistant will now call `graphify query` before reaching for `grep` / `rg` / `find`.

## Documentation

See [`USAGE.md`](USAGE.md) for the full command reference — every subcommand, every flag, environment variables, LLM
backends, workflows, and editor integrations.

For development conventions (lint policy, porting rules, test layout, definition-of-done per crate), see
[`AGENTS.md`](AGENTS.md).

## Workspace layout

```text
graphify/
├── src/                       # graphify CLI binary
├── crates/                    # 30 focused workspace crates
│   ├── graphify-detect/       # filesystem walking + file-type detection
│   ├── graphify-extract/      # tree-sitter / document / media extractors
│   ├── graphify-build/        # graph construction
│   ├── graphify-cluster/      # community detection (Leiden, Louvain fallback)
│   ├── graphify-analyze/      # god-nodes, cohesion, communities
│   ├── graphify-report/       # GRAPH_REPORT.md generator
│   ├── graphify-export/       # HTML / SVG / GraphML / Obsidian / Cypher / Neo4j / FalkorDB
│   ├── graphify-html/         # interactive D3 viz + Mermaid call-flow HTML
│   ├── graphify-wiki/         # per-cluster wiki articles
│   ├── graphify-serve/        # MCP stdio server
│   ├── graphify-hooks/        # git hooks + per-platform assistant installers
│   ├── graphify-global/       # ~/.graphify/global-graph.json
│   ├── graphify-prs/          # GitHub PR triage
│   ├── graphify-llm/          # LLM backend abstraction
│   ├── graphify-security/     # SSRF guard, URL allowlist, graph-load size cap
│   ├── graphify-affected/     # reverse-traversal impact analysis (`graphify affected`)
│   ├── graphify-diagnostics/  # multigraph edge-collapse diagnostic
│   ├── graphify-multigraph-compat/  # runtime keyed-edge capability probe
│   ├── graphify-scip/         # SCIP-style JSON ingest
│   ├── graphify-semantic/     # LLM extraction fragment validator
│   ├── graphify-reflect/      # work-memory reflection (LESSONS.md aggregator)
│   └── ...                    # benchmark, cache, dedup, ingest, manifest, transcribe, validate, watch, google
└── graphify-py/               # read-only git submodule — Python reference
```

## License

Apache-2.0.
