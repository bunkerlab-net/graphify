//! Platform install/uninstall functions.
//!
//! Ports `graphify-py/graphify/__main__.py` — the per-platform skill/config
//! installers (`claude_install`, `gemini_install`, `vscode_install`, etc.).
//!
//! # Skill markdown embedding
//!
//! Skill `.md` files are embedded at compile time via `include_str!` from the
//! `graphify-py` submodule (`graphify-py/graphify/skill*.md`).  This choice
//! avoids any runtime path-resolution for the submodule directory and keeps the
//! binary self-contained — the same binary works whether the submodule is
//! checked out or not, as long as it was present at compile time.  The
//! trade-off is that rebuilding the binary is required after bumping the Python
//! submodule, but that is already required for every other constant that mirrors
//! Python literals byte-for-byte.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::HooksError;

// ---------------------------------------------------------------------------
// Embedded skill files (compile-time, from the graphify-py submodule).
// ---------------------------------------------------------------------------

const SKILL_MD: &str = include_str!("../../../graphify-py/graphify/skill.md");
const SKILL_CODEX_MD: &str = include_str!("../../../graphify-py/graphify/skill-codex.md");
const SKILL_OPENCODE_MD: &str = include_str!("../../../graphify-py/graphify/skill-opencode.md");
const SKILL_AIDER_MD: &str = include_str!("../../../graphify-py/graphify/skill-aider.md");
const SKILL_COPILOT_MD: &str = include_str!("../../../graphify-py/graphify/skill-copilot.md");
const SKILL_CLAW_MD: &str = include_str!("../../../graphify-py/graphify/skill-claw.md");
const SKILL_DROID_MD: &str = include_str!("../../../graphify-py/graphify/skill-droid.md");
const SKILL_TRAE_MD: &str = include_str!("../../../graphify-py/graphify/skill-trae.md");
const SKILL_KIRO_MD: &str = include_str!("../../../graphify-py/graphify/skill-kiro.md");
const SKILL_PI_MD: &str = include_str!("../../../graphify-py/graphify/skill-pi.md");
const SKILL_WINDOWS_MD: &str = include_str!("../../../graphify-py/graphify/skill-windows.md");
const SKILL_VSCODE_MD: &str = include_str!("../../../graphify-py/graphify/skill-vscode.md");

// ---------------------------------------------------------------------------
// Install-surface constants (byte-identical to Python).
// ---------------------------------------------------------------------------

/// `PreToolUse` hook matcher registered in `.claude/settings.json`.
pub const SETTINGS_HOOK_MATCHER: &str = "Bash";

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

When the user types `/graphify`, invoke the `skill` tool with `skill: \"graphify\"` before doing anything else.

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

Follow the graphify skill installed at ~/.agents/skills/graphify/SKILL.md to run the full pipeline.

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
          'echo \"[graphify] knowledge graph at graphify-out/. For focused questions, run \\`graphify query \\\"<question>\\\"\\` (scoped subgraph, usually much smaller than GRAPH_REPORT.md) instead of grepping raw files. Read GRAPH_REPORT.md only for broad architecture context.\" && ' +
          output.args.command;
        reminded = true;
      }
    },
  };
};
";

/// Skill registration text appended to `~/.claude/CLAUDE.md`.
const SKILL_REGISTRATION: &str = "\n# graphify\n\
- **graphify** (`~/.claude/skills/graphify/SKILL.md`) \
- any input to knowledge graph. Trigger: `/graphify`\n\
When the user types `/graphify`, invoke the Skill tool \
with `skill: \"graphify\"` before doing anything else.\n";

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build the Claude `PreToolUse` hook entry (byte-identical to Python's `_SETTINGS_HOOK`).
fn settings_hook() -> Value {
    serde_json::json!({
        "matcher": "Bash",
        "hooks": [
            {
                "type": "command",
                "command": concat!(
                    "CMD=$(python3 -c \"",
                    "import json,sys; d=json.load(sys.stdin); ",
                    "print(d.get('tool_input',d).get('command',''))\" 2>/dev/null || true); ",
                    "case \"$CMD\" in ",
                    r"*grep*|*rg\ *|*ripgrep*|*find\ *|*fd\ *|*ack\ *|*ag\ *) ",
                    "  [ -f graphify-out/graph.json ] && ",
                    r#"  echo '{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"graphify: knowledge graph at graphify-out/. For focused questions, run `graphify query \"<question>\"` (scoped subgraph, usually much smaller than GRAPH_REPORT.md) instead of grepping raw files. Read GRAPH_REPORT.md only for broad architecture context."}}' "#,
                    "  || true ;; ",
                    "esac"
                )
            }
        ]
    })
}

/// Build the Gemini `BeforeTool` hook entry (byte-identical to Python's `_GEMINI_HOOK`).
fn gemini_hook() -> Value {
    serde_json::json!({
        "matcher": "read_file|list_directory",
        "hooks": [
            {
                "type": "command",
                "command": concat!(
                    "python -c \"",
                    "import sys,pathlib,json;",
                    "e=pathlib.Path('graphify-out/graph.json').exists();",
                    "d={'decision':'allow'};",
                    r#"e and d.update({'additionalContext':'graphify: knowledge graph at graphify-out/. For focused questions, run `graphify query "<question>"` (scoped subgraph, usually much smaller than GRAPH_REPORT.md) instead of grepping raw files. Read GRAPH_REPORT.md only for broad architecture context.'});"#,
                    "sys.stdout.write(json.dumps(d))",
                    "\""
                )
            }
        ]
    })
}

/// Idempotently update or append a graphify-owned section in shared markdown files.
///
/// Mirrors Python `_replace_or_append_section` byte-for-byte.
///
/// If `marker` is not in `content`, appends `new_section` (with a blank-line separator if
/// existing content is non-empty). If `marker` IS present, replaces the existing section
/// in place (from the first line containing `marker` to the line before the next `## ` heading
/// or EOF).
#[must_use]
pub fn replace_or_append_section(content: &str, marker: &str, new_section: &str) -> String {
    if !content.contains(marker) {
        if content.trim().is_empty() {
            return new_section.trim_start().to_string();
        }
        return format!("{}\n\n{}", content.trim_end(), new_section.trim_start());
    }

    let lines: Vec<&str> = content.split('\n').collect();
    let Some(start) = lines.iter().position(|l| l.contains(marker)) else {
        // Marker was found by contains but not by position — shouldn't happen; append.
        return format!("{}\n\n{}", content.trim_end(), new_section.trim_start());
    };

    let end = lines[start + 1..]
        .iter()
        .position(|l| l.starts_with("## "))
        .map_or(lines.len(), |rel| start + 1 + rel);

    let head = lines[..start].join("\n").trim_end().to_string();
    let tail = lines[end..].join("\n").trim_start().to_string();
    let section = new_section.trim().to_string();

    let mut parts: Vec<String> = Vec::new();
    if !head.is_empty() {
        parts.push(head);
    }
    parts.push(section);
    if !tail.is_empty() {
        parts.push(tail);
    }
    let mut out = parts.join("\n\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Remove a `## graphify` section from markdown content.
///
/// Equivalent to Python: `re.sub(r"\n*## graphify\n.*?(?=\n## |\Z)", "", content, flags=re.DOTALL)`.
/// The `regex` crate does not support lookahead, so this is implemented with
/// pure string manipulation.
fn remove_graphify_section(content: &str) -> String {
    const MARKER: &str = "## graphify";
    // Find the start of the graphify section, stripping leading blank lines.
    let Some(marker_byte) = content.find(MARKER) else {
        return content.trim_end().to_string();
    };
    // Extend backward past any leading `\n` characters.
    let section_start = content[..marker_byte]
        .rfind(|c: char| c != '\n')
        .map_or(0, |i| i + 1);

    // Find where the section ends: next `## ` heading at the start of a line, or EOF.
    let after_marker = marker_byte + MARKER.len();
    let section_end = content[after_marker..]
        .find("\n## ")
        .map_or(content.len(), |rel| after_marker + rel);

    // Stitch head + tail.
    let head = &content[..section_start];
    let tail = &content[section_end..];
    format!("{head}{tail}").trim_end().to_string()
}

/// Write `content` to `path` atomically (tmp then rename).
fn write_atomic(path: &Path, content: &str) -> Result<(), HooksError> {
    let tmp = path.with_extension(path.extension().map_or_else(
        || "tmp".to_string(),
        |e| format!("{}.tmp", e.to_string_lossy()),
    ));
    fs::write(&tmp, content.as_bytes())?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    Ok(())
}

/// Install a skill markdown file to `dst`, creating parent directories.
///
/// Returns the path written.
fn install_skill(skill_content: &str, dst: &Path) -> Result<PathBuf, HooksError> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    write_atomic(dst, skill_content)?;
    Ok(dst.to_path_buf())
}

/// Remove a skill file and attempt to prune empty parent directories (up to 3 levels).
fn remove_skill(skill_dst: &Path) {
    if skill_dst.exists() {
        let _ = fs::remove_file(skill_dst);
    }
    let version_file = skill_dst
        .parent()
        .map(|p| p.join(".graphify_version"))
        .unwrap_or_default();
    if version_file.exists() {
        let _ = fs::remove_file(&version_file);
    }
    let Some(p0) = skill_dst.parent() else {
        return;
    };
    let Some(p1) = p0.parent() else {
        return;
    };
    let Some(p2) = p1.parent() else {
        return;
    };
    for dir in &[p0, p1, p2] {
        if fs::remove_dir(dir).is_err() {
            break;
        }
    }
}

/// Resolve the graphify executable path (mirrors `_resolve_graphify_exe`).
#[must_use]
pub fn resolve_graphify_exe() -> String {
    // Try which/where equivalent: check PATH
    if let Ok(output) = std::process::Command::new("which").arg("graphify").output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return path;
        }
    }
    "graphify".to_string()
}

/// Read a JSON file, returning an empty object on parse failure.
fn read_json_or_empty(path: &Path) -> Value {
    if !path.exists() {
        return Value::Object(serde_json::Map::new());
    }
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()))
}

/// Write a `serde_json::Value` to `path` with 2-space indentation.
fn write_json(path: &Path, value: &Value) -> Result<(), HooksError> {
    let json = serde_json::to_string_pretty(value).map_err(|e| HooksError::Json(e.to_string()))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, json.as_bytes())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Claude
// ---------------------------------------------------------------------------

/// Write the graphify section to `project_dir/CLAUDE.md` and install the
/// `PreToolUse` hook in `project_dir/.claude/settings.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn claude_install(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();
    let target = project_dir.join("CLAUDE.md");

    let new_content = if target.exists() {
        let content = fs::read_to_string(&target)?;
        replace_or_append_section(&content, CLAUDE_MD_MARKER, CLAUDE_MD_SECTION)
    } else {
        CLAUDE_MD_SECTION.trim_start().to_string()
    };

    if target.exists() && fs::read_to_string(&target).is_ok_and(|c| c == new_content) {
        msgs.push(format!(
            "graphify already configured in {} (no change)",
            target.display()
        ));
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, new_content.as_bytes())?;
        msgs.push(format!("graphify section written to {}", target.display()));
    }

    msgs.push(install_claude_hook(project_dir)?);

    msgs.push(String::new());
    msgs.push("Claude Code will now check the knowledge graph before answering".to_string());
    msgs.push("codebase questions and rebuild it after code changes.".to_string());

    Ok(msgs.join("\n"))
}

/// Remove the graphify section from `project_dir/CLAUDE.md` and remove the
/// `PreToolUse` hook from `project_dir/.claude/settings.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn claude_uninstall(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();
    let target = project_dir.join("CLAUDE.md");

    if target.exists() {
        let content = fs::read_to_string(&target)?;
        if content.contains(CLAUDE_MD_MARKER) {
            let cleaned = remove_graphify_section(&content);
            if cleaned.is_empty() {
                fs::remove_file(&target)?;
                msgs.push(format!(
                    "CLAUDE.md was empty after removal - deleted {}",
                    target.display()
                ));
            } else {
                fs::write(&target, format!("{cleaned}\n").as_bytes())?;
                msgs.push(format!(
                    "graphify section removed from {}",
                    target.display()
                ));
            }
        } else {
            msgs.push("graphify section not found in CLAUDE.md - nothing to do".to_string());
        }
    } else {
        msgs.push("No CLAUDE.md found in current directory - nothing to do".to_string());
    }

    msgs.push(uninstall_claude_hook(project_dir)?);
    Ok(msgs.join("\n"))
}

/// Add graphify `PreToolUse` hook to `project_dir/.claude/settings.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn install_claude_hook(project_dir: &Path) -> Result<String, HooksError> {
    let settings_path = project_dir.join(".claude").join("settings.json");
    let mut settings = read_json_or_empty(&settings_path);

    let hooks = settings
        .as_object_mut()
        .and_then(|o| {
            o.entry("hooks")
                .or_insert_with(|| Value::Object(serde_json::Map::new()))
                .as_object_mut()
        })
        .ok_or_else(|| HooksError::Json("hooks is not an object".to_string()))?;

    let pre_tool = hooks
        .entry("PreToolUse")
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Value::Array(arr) = pre_tool {
        // Remove stale graphify entries (Glob|Grep or Bash matcher + graphify in payload).
        arr.retain(|h| {
            let matcher = h.get("matcher").and_then(Value::as_str).unwrap_or("");
            let is_stale_matcher = matcher == "Glob|Grep" || matcher == SETTINGS_HOOK_MATCHER;
            let has_graphify = h.to_string().contains("graphify");
            !(is_stale_matcher && has_graphify)
        });
        arr.push(settings_hook());
    }

    write_json(&settings_path, &settings)?;
    Ok("  .claude/settings.json  ->  PreToolUse hook registered".to_string())
}

/// Remove graphify `PreToolUse` hook from `project_dir/.claude/settings.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn uninstall_claude_hook(project_dir: &Path) -> Result<String, HooksError> {
    let settings_path = project_dir.join(".claude").join("settings.json");
    if !settings_path.exists() {
        return Ok(String::new());
    }
    let mut settings = read_json_or_empty(&settings_path);
    let pre_tool = settings
        .pointer_mut("/hooks/PreToolUse")
        .and_then(Value::as_array_mut);
    let Some(arr) = pre_tool else {
        return Ok(String::new());
    };
    let before = arr.len();
    arr.retain(|h| {
        let matcher = h.get("matcher").and_then(Value::as_str).unwrap_or("");
        let is_stale = matcher == "Glob|Grep" || matcher == SETTINGS_HOOK_MATCHER;
        !(is_stale && h.to_string().contains("graphify"))
    });
    if arr.len() == before {
        return Ok(String::new());
    }
    write_json(&settings_path, &settings)?;
    Ok("  .claude/settings.json  ->  PreToolUse hook removed".to_string())
}

// ---------------------------------------------------------------------------
// Gemini
// ---------------------------------------------------------------------------

/// Install graphify skill + GEMINI.md section + `BeforeTool` hook for Gemini CLI.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn gemini_install(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    // Skill to ~/.gemini/skills/graphify/SKILL.md (or ~/.agents/ on Windows).
    let skill_dst = if cfg!(target_os = "windows") {
        dirs_home()
            .join(".agents")
            .join("skills")
            .join("graphify")
            .join("SKILL.md")
    } else {
        dirs_home()
            .join(".gemini")
            .join("skills")
            .join("graphify")
            .join("SKILL.md")
    };
    install_skill(SKILL_MD, &skill_dst)?;
    msgs.push(format!("  skill installed  ->  {}", skill_dst.display()));

    // GEMINI.md
    let target = project_dir.join("GEMINI.md");
    let new_content = if target.exists() {
        let content = fs::read_to_string(&target)?;
        replace_or_append_section(&content, CLAUDE_MD_MARKER, GEMINI_MD_SECTION)
    } else {
        GEMINI_MD_SECTION.to_string()
    };
    if target.exists() && fs::read_to_string(&target).is_ok_and(|c| c == new_content) {
        msgs.push(format!(
            "graphify already configured in {} (no change)",
            target.display()
        ));
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, new_content.as_bytes())?;
        msgs.push(format!("graphify section written to {}", target.display()));
    }

    msgs.push(install_gemini_hook(project_dir)?);
    msgs.push(String::new());
    msgs.push("Gemini CLI will now check the knowledge graph before answering".to_string());
    msgs.push("codebase questions and rebuild it after code changes.".to_string());

    Ok(msgs.join("\n"))
}

/// Remove the graphify section from GEMINI.md, uninstall hook, and remove skill.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn gemini_uninstall(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let skill_dst = if cfg!(target_os = "windows") {
        dirs_home()
            .join(".agents")
            .join("skills")
            .join("graphify")
            .join("SKILL.md")
    } else {
        dirs_home()
            .join(".gemini")
            .join("skills")
            .join("graphify")
            .join("SKILL.md")
    };
    if skill_dst.exists() {
        msgs.push(format!("  skill removed    ->  {}", skill_dst.display()));
    }
    remove_skill(&skill_dst);

    let target = project_dir.join("GEMINI.md");
    if !target.exists() {
        msgs.push("No GEMINI.md found in current directory - nothing to do".to_string());
        return Ok(msgs.join("\n"));
    }
    let content = fs::read_to_string(&target)?;
    if !content.contains(CLAUDE_MD_MARKER) {
        msgs.push("graphify section not found in GEMINI.md - nothing to do".to_string());
        return Ok(msgs.join("\n"));
    }
    let cleaned = remove_graphify_section(&content);
    if cleaned.is_empty() {
        fs::remove_file(&target)?;
        msgs.push(format!(
            "GEMINI.md was empty after removal - deleted {}",
            target.display()
        ));
    } else {
        fs::write(&target, format!("{cleaned}\n").as_bytes())?;
        msgs.push(format!(
            "graphify section removed from {}",
            target.display()
        ));
    }
    msgs.push(uninstall_gemini_hook(project_dir)?);
    Ok(msgs.join("\n"))
}

/// Add graphify `BeforeTool` hook to `project_dir/.gemini/settings.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn install_gemini_hook(project_dir: &Path) -> Result<String, HooksError> {
    let settings_path = project_dir.join(".gemini").join("settings.json");
    let mut settings = read_json_or_empty(&settings_path);

    let hooks = settings
        .as_object_mut()
        .and_then(|o| {
            o.entry("hooks")
                .or_insert_with(|| Value::Object(serde_json::Map::new()))
                .as_object_mut()
        })
        .ok_or_else(|| HooksError::Json("hooks is not an object".to_string()))?;

    let before_tool = hooks
        .entry("BeforeTool")
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Value::Array(arr) = before_tool {
        arr.retain(|h| !h.to_string().contains("graphify"));
        arr.push(gemini_hook());
    }

    write_json(&settings_path, &settings)?;
    Ok("  .gemini/settings.json  ->  BeforeTool hook registered".to_string())
}

/// Remove graphify `BeforeTool` hook from `project_dir/.gemini/settings.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn uninstall_gemini_hook(project_dir: &Path) -> Result<String, HooksError> {
    let settings_path = project_dir.join(".gemini").join("settings.json");
    if !settings_path.exists() {
        return Ok(String::new());
    }
    let mut settings = read_json_or_empty(&settings_path);
    let before_tool = settings
        .pointer_mut("/hooks/BeforeTool")
        .and_then(Value::as_array_mut);
    let Some(arr) = before_tool else {
        return Ok(String::new());
    };
    let before = arr.len();
    arr.retain(|h| !h.to_string().contains("graphify"));
    if arr.len() == before {
        return Ok(String::new());
    }
    write_json(&settings_path, &settings)?;
    Ok("  .gemini/settings.json  ->  BeforeTool hook removed".to_string())
}

// ---------------------------------------------------------------------------
// VS Code / Copilot Chat
// ---------------------------------------------------------------------------

/// Install graphify skill + `.github/copilot-instructions.md` section for VS Code Copilot Chat.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn vscode_install(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let skill_dst = dirs_home()
        .join(".copilot")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    install_skill(SKILL_VSCODE_MD, &skill_dst)?;
    msgs.push(format!("  skill installed  ->  {}", skill_dst.display()));

    let instructions = project_dir.join(".github").join("copilot-instructions.md");
    if let Some(parent) = instructions.parent() {
        fs::create_dir_all(parent)?;
    }
    let (new_content, label) = if instructions.exists() {
        let content = fs::read_to_string(&instructions)?;
        let new =
            replace_or_append_section(&content, CLAUDE_MD_MARKER, VSCODE_INSTRUCTIONS_SECTION);
        let label = if new == content {
            "already configured (no change)"
        } else if content.contains(CLAUDE_MD_MARKER) {
            "updated"
        } else {
            "added"
        };
        (new, label)
    } else {
        (VSCODE_INSTRUCTIONS_SECTION.to_string(), "created")
    };

    if instructions.exists() && label == "already configured (no change)" {
        msgs.push(format!("  {}  ->  {label}", instructions.display()));
    } else {
        fs::write(&instructions, new_content.as_bytes())?;
        msgs.push(format!("  {}  ->  {label}", instructions.display()));
    }

    msgs.push(String::new());
    msgs.push(
        "VS Code Copilot Chat configured. Type /graphify in the chat panel to build the graph."
            .to_string(),
    );
    msgs.push("Note: for GitHub Copilot CLI (terminal), use: graphify copilot install".to_string());
    Ok(msgs.join("\n"))
}

/// Remove graphify VS Code Copilot Chat skill and `.github/copilot-instructions.md` section.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn vscode_uninstall(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();
    let skill_dst = dirs_home()
        .join(".copilot")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    if skill_dst.exists() {
        msgs.push(format!("  skill removed    ->  {}", skill_dst.display()));
    }
    remove_skill(&skill_dst);

    let instructions = project_dir.join(".github").join("copilot-instructions.md");
    if !instructions.exists() {
        return Ok(msgs.join("\n"));
    }
    let content = fs::read_to_string(&instructions)?;
    if !content.contains(CLAUDE_MD_MARKER) {
        return Ok(msgs.join("\n"));
    }
    let cleaned = remove_graphify_section(&content);
    if cleaned.is_empty() {
        fs::remove_file(&instructions)?;
        msgs.push(format!(
            "  {}  ->  deleted (was empty after removal)",
            instructions.display()
        ));
    } else {
        fs::write(&instructions, format!("{cleaned}\n").as_bytes())?;
        msgs.push(format!(
            "  graphify section removed from {}",
            instructions.display()
        ));
    }
    Ok(msgs.join("\n"))
}

// ---------------------------------------------------------------------------
// Copilot CLI (skill-only install)
// ---------------------------------------------------------------------------

/// Install graphify skill for GitHub Copilot CLI (`~/.copilot/skills/graphify/SKILL.md`).
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn copilot_install() -> Result<String, HooksError> {
    let skill_dst = dirs_home()
        .join(".copilot")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    install_skill(SKILL_COPILOT_MD, &skill_dst)?;
    Ok(format!("  skill installed  ->  {}", skill_dst.display()))
}

/// Remove graphify skill for GitHub Copilot CLI.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn copilot_uninstall() -> Result<String, HooksError> {
    let skill_dst = dirs_home()
        .join(".copilot")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    if !skill_dst.exists() {
        return Ok("nothing to remove".to_string());
    }
    let msg = format!("skill removed: {}", skill_dst.display());
    remove_skill(&skill_dst);
    Ok(msg)
}

// ---------------------------------------------------------------------------
// Pi
// ---------------------------------------------------------------------------

/// Install graphify skill for Pi coding agent.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn pi_install() -> Result<String, HooksError> {
    let skill_dst = dirs_home()
        .join(".pi")
        .join("agent")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    install_skill(SKILL_PI_MD, &skill_dst)?;
    Ok(format!("  skill installed  ->  {}", skill_dst.display()))
}

/// Remove graphify skill for Pi coding agent.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn pi_uninstall() -> Result<String, HooksError> {
    let skill_dst = dirs_home()
        .join(".pi")
        .join("agent")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    if skill_dst.exists() {
        let msg = format!("  skill removed    ->  {}", skill_dst.display());
        remove_skill(&skill_dst);
        Ok(msg)
    } else {
        Ok(String::new())
    }
}

// ---------------------------------------------------------------------------
// Kiro
// ---------------------------------------------------------------------------

/// Install graphify skill + steering file for Kiro IDE/CLI.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn kiro_install(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    // Skill → .kiro/skills/graphify/SKILL.md  (project-local, like Python does)
    let skill_dst = project_dir
        .join(".kiro")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    install_skill(SKILL_KIRO_MD, &skill_dst)?;
    msgs.push("  .kiro/skills/graphify/SKILL.md  ->  /graphify skill".to_string());

    // Steering → .kiro/steering/graphify.md (wholly owned, overwrite on upgrade).
    let steering_dir = project_dir.join(".kiro").join("steering");
    fs::create_dir_all(&steering_dir)?;
    let steering_dst = steering_dir.join("graphify.md");
    let current = if steering_dst.exists() {
        fs::read_to_string(&steering_dst)?
    } else {
        String::new()
    };
    if current == KIRO_STEERING {
        msgs.push("  .kiro/steering/graphify.md  ->  already configured (no change)".to_string());
    } else {
        let action = if steering_dst.exists() {
            "updated"
        } else {
            "written"
        };
        fs::write(&steering_dst, KIRO_STEERING.as_bytes())?;
        msgs.push(format!(
            "  .kiro/steering/graphify.md  ->  always-on steering {action}"
        ));
    }

    msgs.push(String::new());
    msgs.push("Kiro will now read the knowledge graph before every conversation.".to_string());
    msgs.push("Use /graphify to build or update the graph.".to_string());
    Ok(msgs.join("\n"))
}

/// Remove graphify skill + steering file for Kiro.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn kiro_uninstall(project_dir: &Path) -> Result<String, HooksError> {
    let mut removed: Vec<String> = Vec::new();

    let skill_dst = project_dir
        .join(".kiro")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    if skill_dst.exists() {
        fs::remove_file(&skill_dst)?;
        removed.push(".kiro/skills/graphify/SKILL.md".to_string());
        // Remove parent dir if empty
        if let Some(p) = skill_dst.parent() {
            let _ = fs::remove_dir(p);
        }
    }

    let steering_dst = project_dir
        .join(".kiro")
        .join("steering")
        .join("graphify.md");
    if steering_dst.exists() {
        fs::remove_file(&steering_dst)?;
        removed.push(".kiro/steering/graphify.md".to_string());
    }

    if removed.is_empty() {
        Ok("Removed: nothing to remove".to_string())
    } else {
        Ok(format!("Removed: {}", removed.join(", ")))
    }
}

// ---------------------------------------------------------------------------
// Antigravity
// ---------------------------------------------------------------------------

const ANTIGRAVITY_RULES_PATH: &str = ".agents/rules/graphify.md";
const ANTIGRAVITY_WORKFLOW_PATH: &str = ".agents/workflows/graphify.md";

/// Install graphify for Google Antigravity: skill + rules + workflows.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn antigravity_install(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    // Skill to ~/.agents/skills/graphify/SKILL.md
    let skill_dst = dirs_home()
        .join(".agents")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    install_skill(SKILL_MD, &skill_dst)?;
    msgs.push(format!("  skill installed  ->  {}", skill_dst.display()));

    // Inject YAML frontmatter if missing.
    if skill_dst.exists() {
        let content = fs::read_to_string(&skill_dst)?;
        if !content.starts_with("---\n") {
            let frontmatter = "---\nname: graphify-manager\ndescription: Rebuild the code graph or perform manual CLI queries when MCP server is offline.\n---\n\n";
            fs::write(&skill_dst, format!("{frontmatter}{content}").as_bytes())?;
        }
    }

    // Rules file.
    let rules_path = project_dir.join(ANTIGRAVITY_RULES_PATH);
    if let Some(parent) = rules_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if rules_path.exists() {
        let existing = fs::read_to_string(&rules_path)?;
        if existing.trim() == ANTIGRAVITY_RULES.trim() {
            msgs.push(format!(
                "graphify rule already configured at {} (no change)",
                rules_path.display()
            ));
        } else {
            fs::write(&rules_path, ANTIGRAVITY_RULES.as_bytes())?;
            msgs.push(format!("graphify rule updated at {}", rules_path.display()));
        }
    } else {
        fs::write(&rules_path, ANTIGRAVITY_RULES.as_bytes())?;
        msgs.push(format!("graphify rule written to {}", rules_path.display()));
    }

    // Workflow file.
    let wf_path = project_dir.join(ANTIGRAVITY_WORKFLOW_PATH);
    if let Some(parent) = wf_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if wf_path.exists() {
        let existing = fs::read_to_string(&wf_path)?;
        if existing.trim() == ANTIGRAVITY_WORKFLOW.trim() {
            msgs.push(format!(
                "graphify workflow already configured at {} (no change)",
                wf_path.display()
            ));
        } else {
            fs::write(&wf_path, ANTIGRAVITY_WORKFLOW.as_bytes())?;
            msgs.push(format!(
                "graphify workflow updated at {}",
                wf_path.display()
            ));
        }
    } else {
        fs::write(&wf_path, ANTIGRAVITY_WORKFLOW.as_bytes())?;
        msgs.push(format!(
            "graphify workflow written to {}",
            wf_path.display()
        ));
    }

    msgs.push(String::new());
    msgs.push("Antigravity will now check the knowledge graph before answering".to_string());
    msgs.push("codebase questions. Run /graphify first to build the graph.".to_string());
    msgs.push(String::new());
    msgs.push("To enable full MCP architecture navigation, add this to ~/.gemini/antigravity/mcp_config.json:".to_string());
    msgs.push("  \"graphify\": {".to_string());
    msgs.push("    \"command\": \"uv\",".to_string());
    msgs.push("    \"args\": [\"run\", \"--with\", \"graphifyy\", \"--with\", \"mcp\", \"-m\", \"graphify.serve\", \"${workspace.path}/graphify-out/graph.json\"]".to_string());
    msgs.push("  }".to_string());

    Ok(msgs.join("\n"))
}

/// Remove graphify Antigravity rules, workflow, and skill files.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn antigravity_uninstall(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let rules_path = project_dir.join(ANTIGRAVITY_RULES_PATH);
    if rules_path.exists() {
        fs::remove_file(&rules_path)?;
        msgs.push(format!(
            "graphify rule removed from {}",
            rules_path.display()
        ));
    } else {
        msgs.push("No graphify Antigravity rule found - nothing to do".to_string());
    }

    let wf_path = project_dir.join(ANTIGRAVITY_WORKFLOW_PATH);
    if wf_path.exists() {
        fs::remove_file(&wf_path)?;
        msgs.push(format!(
            "graphify workflow removed from {}",
            wf_path.display()
        ));
    }

    let skill_dst = dirs_home()
        .join(".agents")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    if skill_dst.exists() {
        msgs.push(format!(
            "graphify skill removed from {}",
            skill_dst.display()
        ));
        remove_skill(&skill_dst);
    }

    Ok(msgs.join("\n"))
}

// ---------------------------------------------------------------------------
// Cursor
// ---------------------------------------------------------------------------

/// Write `.cursor/rules/graphify.mdc` with `alwaysApply: true`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn cursor_install(project_dir: &Path) -> Result<String, HooksError> {
    let rule_path = project_dir
        .join(".cursor")
        .join("rules")
        .join("graphify.mdc");
    if let Some(parent) = rule_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if rule_path.exists() && fs::read_to_string(&rule_path).is_ok_and(|c| c == CURSOR_RULE) {
        return Ok(format!(
            "graphify rule at {} already configured (no change)",
            rule_path.display()
        ));
    }
    let action = if rule_path.exists() {
        "updated"
    } else {
        "written"
    };
    fs::write(&rule_path, CURSOR_RULE.as_bytes())?;
    let mut msgs = vec![format!("graphify rule {action} at {}", rule_path.display())];
    msgs.push(String::new());
    msgs.push("Cursor will now always include the knowledge graph context.".to_string());
    msgs.push("Run /graphify . first to build the graph if you haven't already.".to_string());
    Ok(msgs.join("\n"))
}

/// Remove `.cursor/rules/graphify.mdc`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn cursor_uninstall(project_dir: &Path) -> Result<String, HooksError> {
    let rule_path = project_dir
        .join(".cursor")
        .join("rules")
        .join("graphify.mdc");
    if !rule_path.exists() {
        return Ok("No graphify Cursor rule found - nothing to do".to_string());
    }
    fs::remove_file(&rule_path)?;
    Ok(format!(
        "graphify Cursor rule removed from {}",
        rule_path.display()
    ))
}

// ---------------------------------------------------------------------------
// OpenCode plugin
// ---------------------------------------------------------------------------

const OPENCODE_PLUGIN_PATH: &str = ".opencode/plugins/graphify.js";
const OPENCODE_CONFIG_PATH: &str = ".opencode/opencode.json";

/// Write `graphify.js` plugin and register it in `opencode.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn install_opencode_plugin(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let plugin_file = project_dir.join(OPENCODE_PLUGIN_PATH);
    if let Some(parent) = plugin_file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&plugin_file, OPENCODE_PLUGIN_JS.as_bytes())?;
    msgs.push(format!(
        "  {OPENCODE_PLUGIN_PATH}  ->  tool.execute.before hook written"
    ));

    let config_file = project_dir.join(OPENCODE_CONFIG_PATH);
    let mut config = read_json_or_empty(&config_file);

    let plugins = config
        .as_object_mut()
        .map(|o| {
            o.entry("plugin")
                .or_insert_with(|| Value::Array(Vec::new()))
        })
        .ok_or_else(|| HooksError::Json("config is not an object".to_string()))?;

    let entry = ".opencode/plugins/graphify.js";
    let already = if let Value::Array(arr) = &plugins {
        arr.iter().any(|v| v.as_str() == Some(entry))
    } else {
        false
    };

    if already {
        msgs.push(format!(
            "  {OPENCODE_CONFIG_PATH}  ->  plugin already registered (no change)"
        ));
    } else {
        if let Value::Array(arr) = plugins {
            arr.push(Value::String(entry.to_string()));
        }
        write_json(&config_file, &config)?;
        msgs.push(format!("  {OPENCODE_CONFIG_PATH}  ->  plugin registered"));
    }

    Ok(msgs.join("\n"))
}

/// Remove `graphify.js` plugin and deregister from `opencode.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn uninstall_opencode_plugin(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let plugin_file = project_dir.join(OPENCODE_PLUGIN_PATH);
    if plugin_file.exists() {
        fs::remove_file(&plugin_file)?;
        msgs.push(format!("  {OPENCODE_PLUGIN_PATH}  ->  removed"));
    }

    let config_file = project_dir.join(OPENCODE_CONFIG_PATH);
    if !config_file.exists() {
        return Ok(msgs.join("\n"));
    }
    let mut config = read_json_or_empty(&config_file);
    let entry = ".opencode/plugins/graphify.js";
    let plugins = config.pointer_mut("/plugin").and_then(Value::as_array_mut);
    if let Some(arr) = plugins {
        let before = arr.len();
        arr.retain(|v| v.as_str() != Some(entry));
        if arr.len() != before {
            if arr.is_empty()
                && let Some(obj) = config.as_object_mut()
            {
                obj.remove("plugin");
            }
            write_json(&config_file, &config)?;
            msgs.push(format!("  {OPENCODE_CONFIG_PATH}  ->  plugin deregistered"));
        }
    }

    Ok(msgs.join("\n"))
}

// ---------------------------------------------------------------------------
// Codex hook
// ---------------------------------------------------------------------------

/// Add graphify `PreToolUse` hook to `project_dir/.codex/hooks.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn install_codex_hook(project_dir: &Path) -> Result<String, HooksError> {
    let hooks_path = project_dir.join(".codex").join("hooks.json");
    if let Some(parent) = hooks_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut existing = read_json_or_empty(&hooks_path);

    let graphify_exe = resolve_graphify_exe();
    let hook_entry = serde_json::json!({
        "matcher": "Bash",
        "hooks": [{"type": "command", "command": format!("{graphify_exe} hook-check")}]
    });

    let pre_tool = existing
        .as_object_mut()
        .and_then(|o| {
            o.entry("hooks")
                .or_insert_with(|| Value::Object(serde_json::Map::new()))
                .as_object_mut()
        })
        .map(|h| {
            h.entry("PreToolUse")
                .or_insert_with(|| Value::Array(Vec::new()))
        })
        .ok_or_else(|| HooksError::Json("PreToolUse is not valid".to_string()))?;

    if let Value::Array(arr) = pre_tool {
        arr.retain(|h| !h.to_string().contains("graphify"));
        arr.push(hook_entry);
    }

    write_json(&hooks_path, &existing)?;
    Ok(format!(
        "  .codex/hooks.json  ->  PreToolUse hook registered ({graphify_exe} hook-check)"
    ))
}

/// Remove graphify `PreToolUse` hook from `project_dir/.codex/hooks.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn uninstall_codex_hook(project_dir: &Path) -> Result<String, HooksError> {
    let hooks_path = project_dir.join(".codex").join("hooks.json");
    if !hooks_path.exists() {
        return Ok(String::new());
    }
    let mut existing = read_json_or_empty(&hooks_path);
    let pre_tool = existing
        .pointer_mut("/hooks/PreToolUse")
        .and_then(Value::as_array_mut);
    let Some(arr) = pre_tool else {
        return Ok(String::new());
    };
    let before = arr.len();
    arr.retain(|h| !h.to_string().contains("graphify"));
    if arr.len() == before {
        return Ok(String::new());
    }
    write_json(&hooks_path, &existing)?;
    Ok("  .codex/hooks.json  ->  PreToolUse hook removed".to_string())
}

// ---------------------------------------------------------------------------
// AGENTS.md (shared by codex, opencode, aider, claw, droid, trae, trae-cn, hermes)
// ---------------------------------------------------------------------------

/// Write the graphify section to `project_dir/AGENTS.md`.
///
/// For `codex` also installs `.codex/hooks.json`.
/// For `opencode` also installs `.opencode/plugins/graphify.js`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn agents_install(project_dir: &Path, platform: &str) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();
    let target = project_dir.join("AGENTS.md");

    let new_content = if target.exists() {
        let content = fs::read_to_string(&target)?;
        replace_or_append_section(&content, CLAUDE_MD_MARKER, AGENTS_MD_SECTION)
    } else {
        AGENTS_MD_SECTION.to_string()
    };

    if target.exists() && fs::read_to_string(&target).is_ok_and(|c| c == new_content) {
        msgs.push(format!(
            "graphify already configured in {} (no change)",
            target.display()
        ));
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, new_content.as_bytes())?;
        msgs.push(format!("graphify section written to {}", target.display()));
    }

    if platform == "codex" {
        msgs.push(install_codex_hook(project_dir)?);
    } else if platform == "opencode" {
        msgs.push(install_opencode_plugin(project_dir)?);
    }

    let platform_cap = {
        let mut s = platform.to_string();
        if let Some(c) = s.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        s
    };

    msgs.push(String::new());
    msgs.push(format!(
        "{platform_cap} will now check the knowledge graph before answering"
    ));
    msgs.push("codebase questions and rebuild it after code changes.".to_string());

    if !matches!(platform, "codex" | "opencode") {
        msgs.push(String::new());
        msgs.push(
            "Note: unlike Claude Code, there is no PreToolUse hook equivalent for".to_string(),
        );
        msgs.push(format!(
            "{platform_cap} — the AGENTS.md rules are the always-on mechanism."
        ));
    }

    Ok(msgs.join("\n"))
}

/// Remove the graphify section from `project_dir/AGENTS.md`.
///
/// For `opencode` also removes the `OpenCode` plugin.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn agents_uninstall(project_dir: &Path, platform: &str) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();
    let target = project_dir.join("AGENTS.md");

    if !target.exists() {
        msgs.push("No AGENTS.md found in current directory - nothing to do".to_string());
        return Ok(msgs.join("\n"));
    }

    let content = fs::read_to_string(&target)?;
    if !content.contains(CLAUDE_MD_MARKER) {
        msgs.push("graphify section not found in AGENTS.md - nothing to do".to_string());
        return Ok(msgs.join("\n"));
    }

    let cleaned = remove_graphify_section(&content);
    if cleaned.is_empty() {
        fs::remove_file(&target)?;
        msgs.push(format!(
            "AGENTS.md was empty after removal - deleted {}",
            target.display()
        ));
    } else {
        fs::write(&target, format!("{cleaned}\n").as_bytes())?;
        msgs.push(format!(
            "graphify section removed from {}",
            target.display()
        ));
    }

    if platform == "opencode" {
        msgs.push(uninstall_opencode_plugin(project_dir)?);
    }

    Ok(msgs.join("\n"))
}

// ---------------------------------------------------------------------------
// Global install (skill-file only, via platform name)
// ---------------------------------------------------------------------------

/// Skill-only install for platforms that only need a home-dir skill file.
///
/// Maps `platform` to the correct skill content + destination path and copies
/// the file, mirroring the Python `install(platform=...)` function.
///
/// Supported: `claude`, `windows`, `codex`, `opencode`, `aider`, `copilot`,
/// `claw`, `droid`, `trae`, `trae-cn`, `hermes`, `kiro`, `pi`, `antigravity`,
/// `antigravity-windows`.
///
/// Also writes `~/.claude/CLAUDE.md` for `claude` and `windows` platforms.
///
/// # Errors
///
/// Returns `HooksError::UnknownPlatform` for unrecognised names, `HooksError::Io`
/// on filesystem failures.
pub fn install_platform_skill(platform: &str) -> Result<String, HooksError> {
    // Determine skill content + destination relative to home dir.
    let (skill_content, home_rel): (&str, &str) = match platform {
        "claude" | "windows" => {
            let skill = if platform == "windows" {
                SKILL_WINDOWS_MD
            } else {
                SKILL_MD
            };
            (skill, ".claude/skills/graphify/SKILL.md")
        }
        "codex" => (SKILL_CODEX_MD, ".agents/skills/graphify/SKILL.md"),
        "opencode" => (
            SKILL_OPENCODE_MD,
            ".config/opencode/skills/graphify/SKILL.md",
        ),
        "aider" => (SKILL_AIDER_MD, ".aider/graphify/SKILL.md"),
        "copilot" => (SKILL_COPILOT_MD, ".copilot/skills/graphify/SKILL.md"),
        "claw" => (SKILL_CLAW_MD, ".openclaw/skills/graphify/SKILL.md"),
        "droid" => (SKILL_DROID_MD, ".factory/skills/graphify/SKILL.md"),
        "trae" => (SKILL_TRAE_MD, ".trae/skills/graphify/SKILL.md"),
        "trae-cn" => (SKILL_TRAE_MD, ".trae-cn/skills/graphify/SKILL.md"),
        "hermes" => (SKILL_CLAW_MD, ".hermes/skills/graphify/SKILL.md"),
        "kiro" => (SKILL_KIRO_MD, ".kiro/skills/graphify/SKILL.md"),
        "pi" => (SKILL_PI_MD, ".pi/agent/skills/graphify/SKILL.md"),
        "antigravity" => (SKILL_MD, ".agents/skills/graphify/SKILL.md"),
        "antigravity-windows" => (SKILL_WINDOWS_MD, ".agents/skills/graphify/SKILL.md"),
        other => return Err(HooksError::UnknownPlatform(other.to_string())),
    };

    // Check CLAUDE_CONFIG_DIR override for claude/windows.
    let skill_dst = if matches!(platform, "claude" | "windows") {
        if let Ok(cfg_dir) = std::env::var("CLAUDE_CONFIG_DIR") {
            PathBuf::from(cfg_dir)
                .join("skills")
                .join("graphify")
                .join("SKILL.md")
        } else {
            dirs_home().join(home_rel)
        }
    } else {
        dirs_home().join(home_rel)
    };

    install_skill(skill_content, &skill_dst)?;
    let mut msgs = vec![format!("  skill installed  ->  {}", skill_dst.display())];

    // For claude/windows: register in ~/.claude/CLAUDE.md
    if matches!(platform, "claude" | "windows") {
        let claude_md = if let Ok(cfg_dir) = std::env::var("CLAUDE_CONFIG_DIR") {
            PathBuf::from(cfg_dir).join("CLAUDE.md")
        } else {
            dirs_home().join(".claude").join("CLAUDE.md")
        };
        if claude_md.exists() {
            let content = fs::read_to_string(&claude_md)?;
            if content.contains("graphify") {
                msgs.push("  CLAUDE.md        ->  already registered (no change)".to_string());
            } else {
                let new = format!("{}{}", content.trim_end(), SKILL_REGISTRATION);
                if let Some(parent) = claude_md.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&claude_md, new.as_bytes())?;
                msgs.push(format!(
                    "  CLAUDE.md        ->  skill registered in {}",
                    claude_md.display()
                ));
            }
        } else {
            if let Some(parent) = claude_md.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&claude_md, SKILL_REGISTRATION.trim_start().as_bytes())?;
            msgs.push(format!(
                "  CLAUDE.md        ->  created at {}",
                claude_md.display()
            ));
        }
    }

    msgs.push(String::new());
    msgs.push("Done. Open your AI coding assistant and type:".to_string());
    msgs.push(String::new());
    msgs.push("  /graphify .".to_string());
    msgs.push(String::new());
    Ok(msgs.join("\n"))
}

// ---------------------------------------------------------------------------
// Uninstall all
// ---------------------------------------------------------------------------

/// Remove graphify from every platform detected in the current project.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn uninstall_all(project_dir: &Path, purge: bool) -> Result<String, HooksError> {
    let mut msgs = vec!["Uninstalling graphify from all detected platforms...\n".to_string()];

    // Collect results, ignoring per-platform errors (best-effort uninstall).
    let steps: Vec<Result<String, HooksError>> = vec![
        claude_uninstall(project_dir),
        gemini_uninstall(project_dir),
        vscode_uninstall(project_dir),
        cursor_uninstall(project_dir),
        kiro_uninstall(project_dir),
        antigravity_uninstall(project_dir),
        // AGENTS.md covers codex, aider, opencode, claw, droid, trae, trae-cn, hermes
        agents_uninstall(project_dir, ""),
        uninstall_opencode_plugin(project_dir),
        uninstall_codex_hook(project_dir),
    ];

    for step in steps {
        match step {
            Ok(msg) if !msg.is_empty() => msgs.push(msg),
            Ok(_) => {}
            Err(e) => msgs.push(format!("  warning: {e}")),
        }
    }

    if purge {
        let out = project_dir.join("graphify-out");
        if out.exists() {
            fs::remove_dir_all(&out)?;
            msgs.push("\n  graphify-out/  ->  deleted (--purge)".to_string());
        } else {
            msgs.push("\n  graphify-out/  ->  not found (nothing to purge)".to_string());
        }
    }

    msgs.push("\nDone. Run 'pip uninstall graphifyy' to remove the package itself.".to_string());
    Ok(msgs.join("\n"))
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

/// Return the user's home directory as a `PathBuf`.
///
/// Falls back to `"."` if `HOME`/`USERPROFILE` is not set (should not happen
/// in normal operation but avoids a panic in test environments).
fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_or_else(|_| PathBuf::from("."), PathBuf::from)
}
