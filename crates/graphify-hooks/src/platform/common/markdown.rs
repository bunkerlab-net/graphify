//! Markdown / config section constants written into per-platform install
//! surfaces (CLAUDE.md, AGENTS.md, GEMINI.md, copilot-instructions.md,
//! cursor rules, kiro steering, antigravity rules/workflows, opencode plugin).
//!
//! Each constant is byte-identical to the Python reference, so the rendered
//! files match exactly.

/// `PreToolUse` hook matcher registered in `.claude/settings.json` (Bash search).
pub const SETTINGS_HOOK_MATCHER: &str = "Bash";

/// `PreToolUse` hook matcher for the Read/Glob nudge (#1114).
pub const READ_SETTINGS_HOOK_MATCHER: &str = "Read|Glob";

/// Claude Code CLAUDE.md section marker.
pub const CLAUDE_MD_MARKER: &str = "## graphify";

/// Content written into the project-local `CLAUDE.md`.
pub const CLAUDE_MD_SECTION: &str = "\
## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- For codebase questions, first run `graphify query \"<question>\"` when graphify-out/graph.json exists. Use `graphify path \"<A>\" \"<B>\"` for relationships and `graphify explain \"<concept>\"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
";

/// AGENTS.md section shared by codex, opencode, aider, claw, droid, trae, trae-cn, hermes.
pub const AGENTS_MD_SECTION: &str = "\
## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

When the user types `/graphify`, use the installed graphify skill or instructions before doing anything else.

Rules:
- For codebase questions, first run `graphify query \"<question>\"` when graphify-out/graph.json exists. Use `graphify path \"<A>\" \"<B>\"` for relationships and `graphify explain \"<concept>\"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
";

/// GEMINI.md section.
pub const GEMINI_MD_SECTION: &str = "\
## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- For codebase questions, first run `graphify query \"<question>\"` when graphify-out/graph.json exists. Use `graphify path \"<A>\" \"<B>\"` for relationships and `graphify explain \"<concept>\"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
";

/// VS Code Copilot Chat `.github/copilot-instructions.md` section.
pub const VSCODE_INSTRUCTIONS_SECTION: &str = "\
## graphify

For any question about this repo's architecture, structure, components, or how to add/modify/find
code, your first action should be `graphify query \"<question>\"` when `graphify-out/graph.json`
exists. Use `graphify path \"<A>\" \"<B>\"` for relationship questions and `graphify explain \"<concept>\"`
for focused-concept questions. These return a scoped subgraph, usually much smaller than the full
report or raw grep output.

Triggers: \"how do I…\", \"where is…\", \"what does … do\", \"add/modify a <component>\",
\"explain the architecture\", or anything that depends on how files or classes relate.

If `graphify-out/wiki/index.md` exists, use it for broad navigation. Read `graphify-out/GRAPH_REPORT.md`
only for broad architecture review or when query/path/explain do not surface enough context. Only read
source files when (a) modifying/debugging specific code, (b) the graph lacks the needed detail, or
(c) the graph is missing or stale.

Type `/graphify` in Copilot Chat to build or update the graph.
";

/// Antigravity `.agents/rules/graphify.md` content.
pub const ANTIGRAVITY_RULES: &str = "\
---
trigger: always_on
description: Consult the graphify knowledge graph at graphify-out/ for codebase and architecture questions.
---

## graphify

This project has a graphify knowledge graph at graphify-out/.

Rules:
- For codebase or architecture questions, when `graphify-out/graph.json` exists, first run `graphify query \"<question>\"` (CLI) or `query_graph` (MCP). Use `graphify path \"<A>\" \"<B>\"` / `shortest_path` for relationships and `graphify explain \"<concept>\"` / `get_node` for focused concepts. These return a scoped subgraph, usually much smaller than `GRAPH_REPORT.md` or raw grep output.
- If graphify-out/wiki/index.md exists, navigate it instead of reading raw files
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context
- After modifying code files in this session, run `graphify update .` to keep the graph current (AST-only, no API cost)
";

/// Antigravity `.agents/workflows/graphify.md` content.
pub const ANTIGRAVITY_WORKFLOW: &str = "\
---
name: graphify
description: Turn any folder of files into a navigable knowledge graph
---

# Workflow: graphify

Follow the graphify skill installed at ~/.gemini/config/skills/graphify/SKILL.md to run the full pipeline.

If no path argument is given, use `.` (current directory).
";

/// Kiro `.kiro/steering/graphify.md` content.
pub const KIRO_STEERING: &str = "\
---
inclusion: always
---

graphify: A knowledge graph of this project lives in `graphify-out/`. \
For codebase, architecture, or dependency questions, when `graphify-out/graph.json` exists, \
first run `graphify query \"<question>\"` (or `graphify path \"<A>\" \"<B>\"` / `graphify explain \"<concept>\"`). \
These return a scoped subgraph, usually much smaller than `GRAPH_REPORT.md` or raw grep output. \
Read `GRAPH_REPORT.md` only for broad architecture review or when those commands do not surface enough context.
";

/// Cursor `.cursor/rules/graphify.mdc` content.
pub const CURSOR_RULE: &str = "\
---
description: graphify knowledge graph context
alwaysApply: true
---

This project has a graphify knowledge graph at graphify-out/.

- For codebase or architecture questions, when `graphify-out/graph.json` exists, first run `graphify query \"<question>\"` (or `graphify path \"<A>\" \"<B>\"` / `graphify explain \"<concept>\"`). These return a scoped subgraph, usually much smaller than `GRAPH_REPORT.md` or raw grep output.
- If graphify-out/wiki/index.md exists, navigate it instead of reading raw files
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context
- After modifying code files in this session, run `graphify update .` to keep the graph current (AST-only, no API cost)
";

/// `OpenCode` `tool.execute.before` plugin JS.
pub const OPENCODE_PLUGIN_JS: &str = "\
// graphify OpenCode plugin
// Injects a knowledge graph reminder before bash tool calls when the graph exists.
import { existsSync } from \"fs\";
import { join } from \"path\";

export const GraphifyPlugin = async ({ directory }) => {
  let reminded = false;

  return {
    \"tool.execute.before\": async (input, output) => {
      if (reminded) return;
      if (!existsSync(join(directory, \"graphify-out\", \"graph.json\"))) return;

      if (input.tool === \"bash\") {
        output.args.command =
          'echo \"[graphify] knowledge graph at graphify-out/. For focused questions, run graphify query with your question (scoped subgraph, usually much smaller than GRAPH_REPORT.md) instead of grepping raw files. Read GRAPH_REPORT.md only for broad architecture context.\" && ' +
          output.args.command;
        reminded = true;
      }
    },
  };
};
";

/// Kilo Code `tool.execute.before` plugin (`.kilo/plugins/graphify.js`).
///
/// Structurally mirrors the `OpenCode` plugin (one-shot graph reminder injected
/// into the next `bash` command when `graphify-out/graph.json` exists), but the
/// injected echo text differs — this one points at `GRAPH_REPORT.md` — so it is
/// not byte-identical to the `OpenCode` plugin. It IS byte-identical to the
/// Python `_KILO_PLUGIN_JS`.
pub const KILO_PLUGIN_JS: &str = r#"// graphify Kilo plugin
// Injects a knowledge graph reminder before bash tool calls when the graph exists.
import { existsSync } from "fs";
import { join } from "path";

export const GraphifyPlugin = async ({ directory }) => {
  let reminded = false;

  return {
    "tool.execute.before": async (input, output) => {
      if (reminded) return;
      if (!existsSync(join(directory, "graphify-out", "graph.json"))) return;

      if (input.tool === "bash") {
        output.args.command =
          'echo "[graphify] Knowledge graph available. Read graphify-out/GRAPH_REPORT.md for god nodes and architecture context before searching files." && ' +
          output.args.command;
        reminded = true;
      }
    },
  };
};
"#;
