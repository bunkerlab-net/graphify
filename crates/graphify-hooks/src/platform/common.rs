//! Cross-platform helpers and the two multi-platform entry points (`install_platform_skill`,
//! `uninstall_all`).
//!
//! This module centralises all shared utilities — embedded skill files, shared markdown
//! constants, JSON helpers, filesystem utilities — so that the per-platform modules stay
//! lean and free of duplication.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::HooksError;

// ---------------------------------------------------------------------------
// Embedded skill files (compile-time, from the graphify-py submodule).
// ---------------------------------------------------------------------------

pub(super) const SKILL_MD: &str = include_str!("../../../../graphify-py/graphify/skill.md");
pub(super) const SKILL_CODEX_MD: &str =
    include_str!("../../../../graphify-py/graphify/skill-codex.md");
pub(super) const SKILL_OPENCODE_MD: &str =
    include_str!("../../../../graphify-py/graphify/skill-opencode.md");
pub(super) const SKILL_AIDER_MD: &str =
    include_str!("../../../../graphify-py/graphify/skill-aider.md");
pub(super) const SKILL_COPILOT_MD: &str =
    include_str!("../../../../graphify-py/graphify/skill-copilot.md");
pub(super) const SKILL_CLAW_MD: &str =
    include_str!("../../../../graphify-py/graphify/skill-claw.md");
pub(super) const SKILL_DROID_MD: &str =
    include_str!("../../../../graphify-py/graphify/skill-droid.md");
pub(super) const SKILL_TRAE_MD: &str =
    include_str!("../../../../graphify-py/graphify/skill-trae.md");
pub(super) const SKILL_KIRO_MD: &str =
    include_str!("../../../../graphify-py/graphify/skill-kiro.md");
pub(super) const SKILL_PI_MD: &str = include_str!("../../../../graphify-py/graphify/skill-pi.md");
pub(super) const SKILL_WINDOWS_MD: &str =
    include_str!("../../../../graphify-py/graphify/skill-windows.md");
pub(super) const SKILL_VSCODE_MD: &str =
    include_str!("../../../../graphify-py/graphify/skill-vscode.md");

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
pub(super) const SKILL_REGISTRATION: &str = "\n# graphify\n\
- **graphify** (`~/.claude/skills/graphify/SKILL.md`) \
- any input to knowledge graph. Trigger: `/graphify`\n\
When the user types `/graphify`, invoke the Skill tool \
with `skill: \"graphify\"` before doing anything else.\n";

// ---------------------------------------------------------------------------
// Hook value builders
// ---------------------------------------------------------------------------

/// Build the Claude `PreToolUse` hook entry (byte-identical to Python's `_SETTINGS_HOOK`).
pub(super) fn settings_hook() -> Value {
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
pub(super) fn gemini_hook() -> Value {
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

// ---------------------------------------------------------------------------
// Shared filesystem helpers
// ---------------------------------------------------------------------------

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
pub(super) fn remove_graphify_section(content: &str) -> String {
    const MARKER: &str = "## graphify";
    let Some(marker_byte) = content.find(MARKER) else {
        return content.trim_end().to_string();
    };
    let section_start = content[..marker_byte]
        .rfind(|c: char| c != '\n')
        .map_or(0, |i| i + 1);

    let after_marker = marker_byte + MARKER.len();
    let section_end = content[after_marker..]
        .find("\n## ")
        .map_or(content.len(), |rel| after_marker + rel);

    let head = &content[..section_start];
    let tail = &content[section_end..];
    format!("{head}{tail}").trim_end().to_string()
}

/// Write `content` to `path` atomically (tmp then rename).
pub(super) fn write_atomic(path: &Path, content: &str) -> Result<(), HooksError> {
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
pub(super) fn install_skill(skill_content: &str, dst: &Path) -> Result<PathBuf, HooksError> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    write_atomic(dst, skill_content)?;
    Ok(dst.to_path_buf())
}

/// Remove a skill file and attempt to prune empty parent directories (up to 3 levels).
pub(super) fn remove_skill(skill_dst: &Path) {
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
pub(super) fn read_json_or_empty(path: &Path) -> Value {
    if !path.exists() {
        return Value::Object(serde_json::Map::new());
    }
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()))
}

/// Write a `serde_json::Value` to `path` with 2-space indentation.
pub(super) fn write_json(path: &Path, value: &Value) -> Result<(), HooksError> {
    let json = serde_json::to_string_pretty(value).map_err(|e| HooksError::Json(e.to_string()))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, json.as_bytes())?;
    Ok(())
}

/// Return the user's home directory as a `PathBuf`.
///
/// Falls back to `"."` if `HOME`/`USERPROFILE` is not set (should not happen
/// in normal operation but avoids a panic in test environments).
pub(super) fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_or_else(|_| PathBuf::from("."), PathBuf::from)
}

// ---------------------------------------------------------------------------
// Multi-platform entry points
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

/// Remove graphify from every platform detected in the current project.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn uninstall_all(project_dir: &Path, purge: bool) -> Result<String, HooksError> {
    use super::{
        antigravity::antigravity_uninstall, claude::claude_uninstall, codex::uninstall_codex_hook,
        cursor::cursor_uninstall, gemini::gemini_uninstall, kiro::kiro_uninstall,
        opencode::uninstall_opencode_plugin, vscode::vscode_uninstall,
    };
    use crate::platform::agents::agents_uninstall;

    let mut msgs = vec!["Uninstalling graphify from all detected platforms...\n".to_string()];

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
