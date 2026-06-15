---
name: graphify
description: "any input (code, docs, papers, images, videos) → knowledge graph. Use when user asks any question about a codebase, documents, or project content — especially if graphify-out/ exists, treat the question as a /graphify query."
trigger: /graphify
---

# /graphify

Turn any folder of files into a navigable knowledge graph with community
detection, an honest audit trail, and three outputs: interactive HTML,
GraphRAG-ready JSON, and a plain-language `GRAPH_REPORT.md`.

This is the **Rust** distribution of graphify (`bunkerlab-net/graphify`). A
single `graphify` binary runs the whole pipeline — there is no Python, no
inline `python -c "..."` plumbing, no subagent dispatch from the host.

## Usage

```text
/graphify                                             # full pipeline on current directory
/graphify <path>                                      # full pipeline on specific path
/graphify https://github.com/<owner>/<repo>           # clone repo, then run full pipeline on it
/graphify https://github.com/<owner>/<repo> --branch <branch>  # clone a specific branch
/graphify <url1> <url2> ...                           # clone multiple repos, build each, merge into one cross-repo graph
/graphify <path> --update                             # incremental — re-extract only new/changed files
/graphify <path> --cluster-only                       # rerun clustering on existing graph
/graphify <path> --no-viz                             # skip visualization, just report + JSON
/graphify <path> --svg                                # also export graph.svg
/graphify <path> --graphml                            # export graph.graphml (Gephi, yEd)
/graphify <path> --neo4j                              # generate graphify-out/cypher.txt for Neo4j
/graphify <path> --neo4j-push bolt://localhost:7687   # push directly to Neo4j
/graphify <path> --falkordb                           # generate graphify-out/cypher.txt for FalkorDB
/graphify <path> --falkordb-push falkordb://localhost:6379   # push directly to FalkorDB (needs the `falkordb` build feature)
/graphify <path> --mcp                                # start MCP stdio server for agent access
/graphify <path> --watch                              # watch folder, auto-rebuild on code changes (no LLM needed)
/graphify <path> --wiki                               # build agent-crawlable wiki (index.md + per-community article)
/graphify <path> --obsidian --obsidian-dir ~/vaults/my-project  # write Obsidian vault to custom path
/graphify add <url>                                   # fetch URL, save to ./raw, update graph
/graphify add <url> --author "Name"                   # tag who wrote it
/graphify add <url> --contributor "Name"              # tag who added it to the corpus
/graphify query "<question>"                          # BFS traversal — broad context
/graphify query "<question>" --dfs                    # DFS — trace a specific path
/graphify query "<question>" --budget 1500            # cap answer at N tokens
/graphify path "AuthModule" "Database"                # shortest path between two concepts
/graphify explain "SwinTransformer"                   # plain-language explanation of a node
```

## What graphify is for

Drop any folder of code, docs, papers, images, or video into graphify and
get a queryable knowledge graph. Persistent across sessions, honest audit
trail (`EXTRACTED` / `INFERRED` / `AMBIGUOUS`), community detection surfaces
cross-document connections you wouldn't think to ask about.

## What You Must Do When Invoked

If the user invoked `/graphify --help` or `/graphify -h` (with no other
arguments), print the contents of the `## Usage` section above verbatim and
stop. Do not run any commands, do not detect files, do not default the path
to `.`. Just print the Usage block and return.

**Fast path — existing graph:** Before doing anything else, check whether
`graphify-out/graph.json` exists in the current working directory. If it
does AND the user's request is a natural-language question (e.g. "How does
X work?", "What calls Y?", "Trace the data flow through Z") and NOT an
explicit rebuild command (`--update`, `--cluster-only`, or a bare path/URL
that implies fresh extraction): **skip Steps 1–4 entirely and jump straight
to `## For /graphify query`.** Run `graphify query "<question>"` immediately.
Do not re-detect. Do not check corpus size. The graph is already built —
use it.

If no path was given, use `.` (current directory). Do not ask the user for a
path.

If the path argument starts with `https://github.com/` or
`http://github.com/`, treat it as a GitHub URL — run Step 0 first, then
continue with the resolved local path.

Follow these steps in order. Do not skip steps.

### Step 0 — Clone GitHub repo(s) (only if a GitHub URL was given)

**Single repo:**

```bash
LOCAL_PATH=$(graphify clone <github-url> --branch <branch>)
# Use LOCAL_PATH as the target for all subsequent steps.
```

**Multiple repos (cross-repo graph):**

```bash
graphify clone <url1>   # → ~/.graphify/repos/<owner1>/<repo1>
graphify clone <url2>   # → ~/.graphify/repos/<owner2>/<repo2>
# Run the pipeline (Steps 1–4) on each local path, then merge:
graphify merge-graphs \
  ~/.graphify/repos/<owner1>/<repo1>/graphify-out/graph.json \
  ~/.graphify/repos/<owner2>/<repo2>/graphify-out/graph.json \
  --out graphify-out/cross-repo-graph.json
```

Graphify clones into `~/.graphify/repos/<owner>/<repo>` and reuses existing
clones on repeat runs. Each node in the merged graph carries a `repo`
attribute so you can filter by origin.

### Step 1 — Ensure graphify is installed

```bash
if ! command -v graphify >/dev/null 2>&1; then
    cargo install --git https://github.com/bunkerlab-net/graphify.git
fi
```

This installs the Rust binary into `~/.cargo/bin/graphify`. If `cargo` is
not present, install Rust first via `https://rustup.rs/`. No Python is
needed — `graphify` is a self-contained executable.

For semantic extraction (Step 2), set **one** of the supported backends
via environment variables before running the pipeline:

- `ANTHROPIC_API_KEY` for Claude (default if set)
- `OPENAI_API_KEY` for OpenAI
- `GEMINI_API_KEY` or `GOOGLE_API_KEY` for Gemini
- `KIMI_API_KEY` for Moonshot Kimi
- `DEEPSEEK_API_KEY` for DeepSeek
- AWS credentials (e.g. via `aws configure`, env vars, or instance profile)
  for Bedrock — `graphify` autodetects the Bedrock SDK credential chain when
  AWS credentials are configured

If no LLM backend is configured, the pipeline still runs but skips the
semantic-extraction pass — you'll get an AST-only graph for code, and
nothing extracted from docs / papers / images.

### Step 2 — Run the full pipeline

```bash
graphify extract <PATH>
```

That single command:

1. Detects files (code, docs, papers, images, video).
2. Runs AST extraction for code (deterministic, free).
3. Runs semantic extraction via the configured LLM backend.
4. Builds the graph, runs community detection (Leiden, with Louvain
   fallback).
5. Writes `graphify-out/graph.json`, `graphify-out/GRAPH_REPORT.md`, and
   the interactive HTML view.

Useful flags:

- `--backend <name>` — pick a specific backend (`claude`, `openai`,
  `gemini`, `kimi`, `deepseek`, `bedrock`) instead of autodetect.
- `--model <name>` — override the default model for the chosen backend.
- `--token-budget <N>` — adjust the per-chunk token budget (default 60000).
- `--max-concurrency <N>` — adjust parallel LLM workers (default 4).
- `--no-cluster` — skip community detection (useful for very large
  corpora where you only want the raw graph).
- `--resolution <r>` — Louvain resolution (`>1` = more, smaller
  communities; default 1.0).
- `--exclude-hubs <p>` — drop hub nodes above the given degree percentile
  (0.0–1.0) before clustering.
- `--exclude <glob>` — repeatable; extra path globs to skip during
  detection.
- `--dedup-llm` — run LLM-driven dedup tiebreak after clustering.
- `--google-workspace` — enable Google Workspace ingest.

If the corpus has more than ~500 files or 2,000,000 words, warn the user
and offer to narrow to a specific subfolder before running. Use
`graphify diagnose corpus <PATH>` if you want a pre-flight size summary
without running extraction.

If `graphify extract` finishes successfully, **read
`graphify-out/GRAPH_REPORT.md`** and quote these sections inline (do **not**
paste the whole report):

- **God Nodes**
- **Surprising Connections**
- **Suggested Questions**

Then immediately offer to explore. Pick the single most interesting
suggested question from the report — the one that crosses the most
community boundaries or has the most surprising bridge node — and ask:

> "The most interesting question this graph can answer: **[question]**.
> Want me to trace it?"

If the user says yes, run `graphify query "[question]"` and walk them
through the answer using the graph structure. Each answer should end with
a natural follow-up so the session feels like navigation, not a one-shot
report.

The graph is the map. Your job after the pipeline is to be the guide.

### Step 3 — Optional exports (only if a corresponding flag was given)

Each export is its own subcommand and reuses the existing graph; do not
re-run extraction.

```bash
graphify export svg          # if --svg
graphify export graphml      # if --graphml
graphify export wiki         # if --wiki
graphify export neo4j        # if --neo4j (writes cypher.txt)
graphify export neo4j --push bolt://localhost:7687 --user neo4j --password PASSWORD  # if --neo4j-push
graphify export falkordb     # if --falkordb (writes cypher.txt)
graphify export falkordb --push falkordb://localhost:6379  # if --falkordb-push (needs the `falkordb` feature)
graphify export obsidian [--dir ~/vaults/my-project]  # only if --obsidian
graphify serve graphify-out/graph.json   # if --mcp (stdio MCP server)
```

### Step 4 — Token-reduction benchmark (only if total_words > 5000)

```bash
graphify benchmark
```

Print the output directly. Skip silently for small corpora.

---

## For /graphify query

```bash
graphify query "QUESTION"                # BFS — broad context
graphify query "QUESTION" --dfs          # DFS — trace a chain
graphify query "QUESTION" --budget 3000  # cap tokens
```

Answer using **only** what the graph output contains. Quote
`source_location` when citing a specific fact. If the graph lacks enough
information, say so — do not hallucinate edges.

After writing the answer, save it back so the next `--update` picks it up:

```bash
graphify save-result --question "QUESTION" --answer "ANSWER" --type query --nodes NODE1 NODE2
```

---

## For /graphify path

```bash
graphify path "NODE_A" "NODE_B"
```

Explain the path in plain language — what each hop means and why it's
significant.

```bash
graphify save-result --question "Path from NODE_A to NODE_B" --answer "ANSWER" --type path_query --nodes NODE_A NODE_B
```

---

## For /graphify explain

```bash
graphify explain "NODE_NAME"
```

Write a 3–5 sentence explanation of what this node is, what it connects to,
and why those connections matter. Cite source locations.

```bash
graphify save-result --question "Explain NODE_NAME" --answer "ANSWER" --type explain --nodes NODE_NAME
```

---

## For /graphify add

Fetch a URL and add it to the corpus.

```bash
graphify add <URL> [--author "Name"] [--contributor "Name"]
```

Supported URL types (auto-detected):

- YouTube / any video URL → audio downloaded via yt-dlp, transcribed
- Twitter/X → fetched via oEmbed, saved as `.md`
- arXiv → abstract + metadata saved as `.md`
- PDF → downloaded as `.pdf`
- Images (`.png`, `.jpg`, `.webp`) → downloaded for vision extraction
- Any webpage → converted to markdown

After a successful add, run `graphify update ./raw` to merge the new file
into the existing graph.

---

## For --update

Incremental re-extraction — only files changed since the last run.

```bash
graphify update <PATH>
```

This re-uses the existing `graphify-out/graph.json`, re-extracts only the
changed files, and merges. AST-only paths run without an LLM backend; if
any changed file is a doc / paper / image, semantic extraction runs on
those changed files.

---

## For --cluster-only

Re-cluster the existing graph without re-extracting.

```bash
graphify cluster-only [--resolution 1.0] [--exclude-hubs 0.99]
```

---

## For --watch

Watch a folder and rebuild on file changes.

```bash
graphify watch <PATH> --debounce 3
```

Code-file changes (`.rs`, `.py`, `.ts`, …) re-run AST extraction
immediately without an LLM. Doc/paper/image changes write a
`graphify-out/needs_update` flag and notify — run `graphify update <PATH>`
to incorporate them.

Press `Ctrl+C` to stop. Use it from a background terminal during agentic
workflows so code-only waves are picked up automatically.

---

## For git commit hook

Install a post-commit hook that auto-rebuilds the graph after every
commit.

```bash
graphify hook install     # install
graphify hook uninstall   # remove
graphify hook status      # check
```

After every `git commit`, the hook detects which code files changed
(`git diff HEAD~1`), re-runs AST extraction on those files, and rebuilds
`graph.json` and `GRAPH_REPORT.md`. Doc / image changes are ignored by the
hook — run `graphify update` manually for those. If a post-commit hook
already exists, graphify appends rather than replacing.

---

## For native CLAUDE.md integration

Run once per project to make graphify always-on in Claude Code sessions:

```bash
graphify claude install
```

This writes a `## graphify` section to the local `CLAUDE.md` that
instructs Claude to check the graph before answering codebase questions
and rebuild it after code changes.

```bash
graphify claude uninstall   # remove the section
```

---

## Honesty Rules

- Never invent an edge. If unsure, the binary marks it `AMBIGUOUS` —
  preserve that.
- Never skip the corpus size warning for large corpora.
- Always show the token cost from `GRAPH_REPORT.md` in your summary.
- Never hide cohesion scores behind symbols — show the raw number.
- Never run HTML viz on a graph with more than 5,000 nodes without
  warning the user; `graphify extract` switches to a community-aggregated
  view automatically above that threshold.
