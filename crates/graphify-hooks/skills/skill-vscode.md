---
name: graphify
description: "any input (code, docs, papers, images) → knowledge graph → clustered communities → HTML + JSON + audit report. Use when user asks any question about a codebase, project content, architecture, or file relationships — especially if graphify-out/ exists. Provides persistent graph with god nodes, community detection, and BFS/DFS query tools."
trigger: /graphify
---

# /graphify (VS Code / GitHub Copilot)

Turn any folder of files into a navigable knowledge graph with community
detection, an honest audit trail, and three outputs: interactive HTML,
GraphRAG-ready JSON, and a plain-language `GRAPH_REPORT.md`.

This is the **Rust** distribution of graphify (`bunkerlab-net/graphify`). A
single self-contained `graphify` binary runs the whole pipeline — no
Python, no inline `python -c "..."` plumbing.

## Usage

```text
/graphify                     # full pipeline on current directory
/graphify <path>              # full pipeline on specific path
/graphify <path> --update     # incremental — re-extract only new/changed files
/graphify <path> --no-viz     # skip visualization, just report + JSON
/graphify <path> --wiki       # build agent-crawlable wiki
/graphify query "<question>"  # BFS traversal — broad context
```

## What You Must Do When Invoked

If the user invoked `/graphify --help` or `/graphify -h`, print the `## Usage`
block verbatim and stop.

If no path was given, use `.` (current directory).

**Fast path — existing graph:** If `graphify-out/graph.json` exists and the
user is asking a natural-language question (not a rebuild flag), run
`graphify query "<question>"` immediately and skip Steps 1–2.

Follow these steps in order. The Rust binary is a single executable that
runs identically on Windows PowerShell, macOS, and Linux — no shell-
specific heredocs or pipes are needed.

### Step 1 — Ensure graphify is installed

```bash
graphify --version
```

If the command fails, install via cargo (requires Rust from
<https://rustup.rs/>):

```bash
cargo install --git https://github.com/bunkerlab-net/graphify.git
```

For semantic extraction, set one LLM credential before running Step 2.
Supported: `ANTHROPIC_API_KEY` (Claude), `OPENAI_API_KEY`, `GEMINI_API_KEY`
or `GOOGLE_API_KEY`, `KIMI_API_KEY`, `DEEPSEEK_API_KEY`, or AWS credentials
(for Bedrock). Without one, the pipeline still runs but skips semantic
extraction — you'll get an AST-only graph for code and nothing extracted
from docs/papers/images.

### Step 2 — Run the pipeline

```bash
graphify extract <PATH>
```

That single command does the whole job: detect → AST → semantic →
cluster → report → HTML + JSON. No subagent dispatch from the host editor.

Useful flags:

- `--backend <name>` (`claude` | `openai` | `gemini` | `kimi` | `deepseek` | `bedrock`)
- `--model <name>`, `--token-budget 60000`, `--max-concurrency 4`
- `--no-cluster`, `--resolution 1.0`, `--exclude-hubs 0.99`
- `--exclude <glob>` (repeatable), `--dedup-llm`, `--google-workspace`

If the corpus is larger than ~500 files or ~2,000,000 words, warn the user
and ask which subfolder to run on before invoking `graphify extract`.

### After the pipeline

Read `graphify-out/GRAPH_REPORT.md` and quote these sections directly in
chat (do **not** ask the user to open the file):

- **God Nodes**
- **Surprising Connections**
- **Suggested Questions**

Then offer to trace the single most interesting suggested question via
`graphify query "<question>"`.

## For /graphify query

```bash
graphify query "QUESTION" [--dfs] [--budget 3000]
```

Answer using only what the graph output contains. Save the answer back:

```bash
graphify save-result --question "QUESTION" --answer "ANSWER" --type query --nodes NODE1 NODE2
```

## For --update (incremental)

```bash
graphify update <PATH>
```

Re-extracts only changed files and merges into the existing graph.

## For --wiki

```bash
graphify export wiki
```

Writes `graphify-out/wiki/index.md` plus one article per community.

## Honesty rules

- Never invent edges. If unsure, the binary marks them `AMBIGUOUS` —
  preserve that.
- Never skip the corpus size warning for large corpora.
- Always show token cost from `GRAPH_REPORT.md`.
