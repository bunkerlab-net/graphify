# graphify

Turn any folder of source code, documentation, papers, images, or videos into a queryable knowledge graph.

Point `graphify` at a directory and you get back three files:

```text
graphify-out/
├── graph.json       full graph — query without re-reading your files
├── graph.html       open in any browser — interactive viz
└── GRAPH_REPORT.md  key concepts, surprising connections, suggested questions
```

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
- **Documents, papers, images, video** — PDF, DOCX, audio transcription, OCR, Google Workspace exports.
- **Local-first** — `graph.json` lives next to your code; no daemon, no cloud, no account.
- **Optional LLM-driven semantic extraction** through OpenAI, Anthropic, Gemini, DeepSeek, Moonshot, Ollama, or Bedrock.
- **AI-assistant integration** — drop-in installers for Claude Code, Codex, Cursor, Gemini CLI, GitHub Copilot, VS Code,
  OpenCode, Aider, Factory Droid, Trae, Hermes, Kiro, Pi, Google Antigravity, and more.
- **MCP server** for any MCP-capable assistant (`graphify serve`).
- **Git hooks + merge driver** so two branches editing the same `graph.json` produce a union-merged result.
- **Cross-repo global graph** — aggregate every project you care about into one `~/.graphify/global-graph.json`.
- **Deterministic outputs** — same inputs on the same machine produce byte-identical JSON.

## Install

Requires Rust 1.90 or newer (`rustup toolchain install stable`).

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
├── crates/                    # 29 focused workspace crates
│   ├── graphify-detect/       # filesystem walking + file-type detection
│   ├── graphify-extract/      # tree-sitter / document / media extractors
│   ├── graphify-build/        # graph construction
│   ├── graphify-cluster/      # deterministic Louvain clustering
│   ├── graphify-analyze/      # god-nodes, cohesion, communities
│   ├── graphify-report/       # GRAPH_REPORT.md generator
│   ├── graphify-export/       # HTML / SVG / GraphML / Obsidian / Cypher / Neo4j
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
│   └── ...                    # benchmark, cache, dedup, ingest, manifest, transcribe, validate, watch, google
└── graphify-py/               # read-only git submodule — Python reference
```

## License

Apache-2.0.
