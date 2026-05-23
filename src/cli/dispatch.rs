//! Subcommand dispatch.
//!
//! Maps a parsed [`Command`] to the handler function that implements it.

use anyhow::Result;

use crate::cli;
use crate::cli::args::Command;

/// Dispatch a parsed [`Command`] to its handler function.
#[allow(clippy::too_many_lines)]
pub(crate) fn dispatch(cmd: Command) -> Result<()> {
    match cmd {
        Command::Validate { path } => cli::validate::cmd_validate(&path),
        Command::Install {
            platform,
            platform_positional,
        } => {
            let resolved = match (platform.as_deref(), platform_positional.as_deref()) {
                (Some(a), Some(b)) if a != b => {
                    anyhow::bail!("error: specify install platform only once")
                }
                (Some(name), _) | (None, Some(name)) => name.to_string(),
                (None, None) => {
                    // Python defaults to "windows" on Windows, "claude" elsewhere.
                    if cfg!(target_os = "windows") {
                        "windows".to_string()
                    } else {
                        "claude".to_string()
                    }
                }
            };
            cli::install::cmd_install(&resolved)
        }
        Command::Uninstall { purge } => cli::install::cmd_uninstall(purge),
        Command::Hook { cmd } => cli::hooks::cmd_hook(&cmd),
        Command::Global { cmd } => cli::global::cmd_global(cmd),
        Command::Benchmark { graph } => cli::benchmark::cmd_benchmark(graph.as_deref()),
        Command::Watch { path } => cli::watch::cmd_watch(&path),
        Command::Update {
            path,
            force,
            no_cluster,
        } => cli::extract::cmd_update(&path, force, no_cluster),
        Command::ClusterOnly {
            path,
            no_viz,
            graph,
            resolution,
            exclude_hubs,
            min_community_size,
        } => cli::cluster_only::cmd_cluster_only(
            &path,
            no_viz,
            graph.as_deref(),
            resolution,
            exclude_hubs,
            min_community_size,
        ),
        Command::Query {
            question,
            dfs,
            context,
            budget,
            graph,
        } => cli::query::cmd_query(&question, dfs, &context, budget, graph.as_deref()),
        Command::Path { from, to, graph } => cli::query::cmd_path(&from, &to, graph.as_deref()),
        Command::Explain { node, graph } => cli::query::cmd_explain(&node, graph.as_deref()),
        Command::SaveResult {
            question,
            answer,
            query_type,
            nodes,
            memory_dir,
        } => {
            cli::save_result::cmd_save_result(&question, &answer, &query_type, &nodes, &memory_dir)
        }
        Command::CheckUpdate { path } => cli::watch::cmd_check_update(&path),
        Command::Tree {
            graph,
            output,
            root,
            max_children,
            top_k_edges,
            label,
        } => cli::tree::cmd_tree(
            graph.as_deref(),
            output.as_deref(),
            root.as_deref(),
            max_children,
            top_k_edges,
            label.as_deref(),
        ),
        Command::Extract {
            path,
            backend,
            model,
            max_workers,
            token_budget,
            max_concurrency,
            api_timeout,
            out,
            google_workspace,
            no_cluster,
            global,
            as_tag,
            resolution,
            exclude_hubs,
            exclude,
            dedup_llm,
        } => cli::extract::cmd_extract(
            &path,
            no_cluster,
            out.as_deref(),
            backend.as_deref(),
            model.as_deref(),
            max_workers,
            token_budget,
            max_concurrency,
            api_timeout,
            google_workspace,
            global,
            as_tag.as_deref(),
            resolution,
            exclude_hubs,
            &exclude,
            dedup_llm,
        ),
        Command::Export { cmd } => cli::export::cmd_export(cmd),
        Command::Add {
            url,
            author,
            contributor,
            dir,
        } => cli::add::cmd_add(&url, author.as_deref(), contributor.as_deref(), &dir),
        Command::Clone { url, branch, out } => {
            cli::clone::cmd_clone(&url, branch.as_deref(), out.as_deref())
        }
        Command::MergeDriver {
            base,
            current,
            other,
        } => cli::merge::cmd_merge_driver(&base, &current, &other),
        Command::MergeGraphs { graphs, out } => {
            cli::merge::cmd_merge_graphs(&graphs, out.as_deref())
        }
        Command::MergeChunks { chunks, out } => cli::merge_chunks::cmd_merge_chunks(&chunks, &out),
        Command::MergeSemantic { cached, new, out } => {
            cli::merge_chunks::cmd_merge_semantic(cached.as_deref(), new.as_deref(), &out)
        }
        Command::Prs {
            number,
            repo,
            base,
            limit,
            triage,
            worktrees,
            conflicts,
            wrong_base,
            graph,
        } => {
            // Accept `123` or `#123`; mirrors Python's `arg.lstrip("#").isdigit()` branch.
            let parsed_number = number
                .as_deref()
                .map(|s| s.trim_start_matches('#'))
                .and_then(|s| s.parse::<u64>().ok());
            cli::prs::cmd_prs(
                parsed_number,
                repo.as_deref(),
                base.as_deref(),
                limit,
                triage,
                worktrees,
                conflicts,
                wrong_base,
                graph.as_deref(),
            )
        }
        Command::Serve { graph } => cli::serve::cmd_serve(graph.as_deref()),
        Command::CacheCheck { files_from, root } => {
            cli::cache_check::cmd_cache_check(&files_from, &root)
        }
        // Cross-platform no-op — mirrors Python `__main__.py:1905-1909`.
        // Codex Desktop rejects hookSpecificOutput.additionalContext on
        // PreToolUse, so installed hooks must exit silently. Graph guidance
        // reaches the agent via AGENTS.md / skill instead.
        Command::HookCheck => Ok(()),
        Command::Claude { cmd: c } => cli::install::cmd_platform("claude", &c),
        Command::Gemini { cmd: c } => cli::install::cmd_platform("gemini", &c),
        Command::Cursor { cmd: c } => cli::install::cmd_platform("cursor", &c),
        Command::Vscode { cmd: c } => cli::install::cmd_platform("vscode", &c),
        Command::Copilot { cmd: c } => cli::install::cmd_platform("copilot", &c),
        Command::Kiro { cmd: c } => cli::install::cmd_platform("kiro", &c),
        Command::Pi { cmd: c } => cli::install::cmd_platform("pi", &c),
        Command::Antigravity { cmd: c } => cli::install::cmd_platform("antigravity", &c),
        Command::Codex { cmd: c } => cli::install::cmd_platform("codex", &c),
        Command::Opencode { cmd: c } => cli::install::cmd_platform("opencode", &c),
        Command::Aider { cmd: c } => cli::install::cmd_platform("aider", &c),
        Command::Claw { cmd: c } => cli::install::cmd_platform("claw", &c),
        Command::Droid { cmd: c } => cli::install::cmd_platform("droid", &c),
        Command::Trae { cmd: c } => cli::install::cmd_platform("trae", &c),
        Command::TraeCn { cmd: c } => cli::install::cmd_platform("trae-cn", &c),
        Command::Hermes { cmd: c } => cli::install::cmd_platform("hermes", &c),
    }
}
