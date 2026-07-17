//! Subcommand dispatch.
//!
//! Maps a parsed [`Command`] to the handler function that implements it.

use anyhow::Result;

use crate::cli;
use crate::cli::args::Command;

/// Dispatch a parsed [`Command`] to its handler function.
// Exhaustive command-routing match; one arm per subcommand reads clearer flat
// than split across helpers.
#[allow(clippy::too_many_lines)]
pub(crate) fn dispatch(cmd: Command) -> Result<()> {
    match cmd {
        Command::Validate { path } => cli::validate::cmd_validate(&path),
        Command::Install {
            platform,
            platform_positional,
            project,
        } => dispatch_install(platform.as_deref(), platform_positional.as_deref(), project),
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
        cmd @ (Command::ClusterOnly { .. } | Command::Label { .. }) => dispatch_cluster_only(cmd),
        Command::Provider { cmd } => cli::provider::cmd_provider(cmd),
        cmd @ Command::Query { .. } => dispatch_query(cmd),
        Command::Path { from, to, graph } => cli::query::cmd_path(&from, &to, graph.as_deref()),
        Command::Explain { node, graph } => cli::query::cmd_explain(&node, graph.as_deref()),
        cmd @ Command::SaveResult { .. } => dispatch_save_result(cmd),
        cmd @ Command::Reflect { .. } => dispatch_reflect(cmd),
        Command::CheckUpdate { path } => cli::watch::cmd_check_update(&path),
        cmd @ Command::Tree { .. } => dispatch_tree(cmd),
        cmd @ Command::Extract { .. } => dispatch_extract(cmd),
        Command::Export { cmd } => cli::export::cmd_export(cmd),
        cmd @ Command::Add { .. } => dispatch_add(cmd),
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
        cmd @ Command::Prs { .. } => dispatch_prs(cmd),
        Command::Serve {
            graph,
            transport,
            host,
            port,
            api_key,
            path,
            json_response,
            stateless,
            session_timeout,
        } => cli::serve::cmd_serve(cli::serve::ServeOptions {
            graph: graph.as_deref(),
            transport,
            host,
            port,
            api_key,
            path,
            json_response,
            stateless,
            session_timeout,
        }),
        Command::Affected {
            query,
            relations,
            depth,
            graph,
        } => cli::affected::cmd_affected(&query, &relations, depth, graph.as_deref()),
        Command::Diagnose { cmd } => cli::diagnose::cmd_diagnose(cmd),
        Command::CacheCheck {
            files_from,
            root,
            mode,
            deep,
        } => {
            // `--deep` is shorthand for `--mode deep`; an explicit non-empty
            // `--mode` otherwise selects the namespace (#1894).
            let resolved = if deep {
                Some("deep")
            } else {
                mode.as_deref().filter(|m| !m.is_empty())
            };
            cli::cache_check::cmd_cache_check(&files_from, &root, resolved)
        }
        // Cross-platform no-op — mirrors Python `__main__.py:1905-1909`.
        // Codex Desktop rejects hookSpecificOutput.additionalContext on
        // PreToolUse, so installed hooks must exit silently. Graph guidance
        // reaches the agent via AGENTS.md / skill instead.
        Command::HookCheck => Ok(()),
        Command::Claude { cmd: c } => cli::install::cmd_platform("claude", &c),
        Command::Codebuddy { cmd: c } => cli::install::cmd_platform("codebuddy", &c),
        Command::Gemini { cmd: c } => cli::install::cmd_platform("gemini", &c),
        Command::Cursor { cmd: c } => cli::install::cmd_platform("cursor", &c),
        Command::Vscode { cmd: c } => cli::install::cmd_platform("vscode", &c),
        Command::Copilot { cmd: c } => cli::install::cmd_platform("copilot", &c),
        Command::Kiro { cmd: c } => cli::install::cmd_platform("kiro", &c),
        Command::Kilo { cmd: c } => cli::install::cmd_kilo(&c),
        Command::Pi { cmd: c } => cli::install::cmd_platform("pi", &c),
        Command::Antigravity { cmd: c } => cli::install::cmd_platform("antigravity", &c),
        Command::Codex { cmd: c } => cli::install::cmd_platform("codex", &c),
        Command::Amp { cmd: c } => cli::install::cmd_platform("amp", &c),
        Command::Opencode { cmd: c } => cli::install::cmd_platform("opencode", &c),
        Command::Aider { cmd: c } => cli::install::cmd_platform("aider", &c),
        Command::Claw { cmd: c } => cli::install::cmd_platform("claw", &c),
        Command::Droid { cmd: c } => cli::install::cmd_platform("droid", &c),
        Command::Trae { cmd: c } => cli::install::cmd_platform("trae", &c),
        Command::TraeCn { cmd: c } => cli::install::cmd_platform("trae-cn", &c),
        Command::Hermes { cmd: c } => cli::install::cmd_platform("hermes", &c),
        Command::Devin { cmd: c } => cli::install::cmd_platform("devin", &c),
        // `agents` and its `skills` alias share one amp-twin handler (#1432).
        Command::Agents { cmd: c } | Command::Skills { cmd: c } => {
            cli::install::cmd_agents_platform(&c)
        }
    }
}

/// Resolve the install platform from `--platform` and the positional fallback, then install.
///
/// Defaults to `"windows"` on Windows and `"claude"` on all other targets when
/// neither flag is provided, mirroring Python's platform-detection fallback.
fn dispatch_install(platform: Option<&str>, positional: Option<&str>, project: bool) -> Result<()> {
    let resolved = match (platform, positional) {
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
    cli::install::cmd_install(&resolved, project)
}

fn dispatch_cluster_only(cmd: Command) -> Result<()> {
    // `cluster-only` and `label` share the same handler; `label` forces a relabel.
    let (
        path,
        no_viz,
        graph,
        resolution,
        exclude_hubs,
        min_community_size,
        no_label,
        backend,
        model,
        max_concurrency,
        batch_size,
        timing,
        missing_only,
        force,
    ) = match cmd {
        Command::ClusterOnly {
            path,
            no_viz,
            graph,
            resolution,
            exclude_hubs,
            min_community_size,
            no_label,
            backend,
            model,
            max_concurrency,
            batch_size,
            timing,
            missing_only,
        } => (
            path,
            no_viz,
            graph,
            resolution,
            exclude_hubs,
            min_community_size,
            no_label,
            backend,
            model,
            max_concurrency,
            batch_size,
            timing,
            missing_only,
            false,
        ),
        Command::Label {
            path,
            no_viz,
            graph,
            resolution,
            exclude_hubs,
            min_community_size,
            backend,
            model,
            max_concurrency,
            batch_size,
            timing,
            missing_only,
        } => (
            path,
            no_viz,
            graph,
            resolution,
            exclude_hubs,
            min_community_size,
            false,
            backend,
            model,
            max_concurrency,
            batch_size,
            timing,
            missing_only,
            true,
        ),
        _ => unreachable!("dispatch_cluster_only invoked with wrong variant"),
    };
    cli::cluster_only::cmd_cluster_only(
        &path,
        no_viz,
        graph.as_deref(),
        resolution,
        exclude_hubs,
        min_community_size,
        cli::cluster_only::LabelOptions {
            no_label,
            backend: backend.as_deref(),
            model: model.as_deref(),
            force_relabel: force,
            max_concurrency,
            batch_size,
            timing,
            missing_only,
        },
    )
}

fn dispatch_query(cmd: Command) -> Result<()> {
    let Command::Query {
        question,
        dfs,
        context,
        budget,
        graph,
    } = cmd
    else {
        unreachable!("dispatch_query invoked with wrong variant")
    };
    cli::query::cmd_query(&question, dfs, &context, budget, graph.as_deref())
}

fn dispatch_save_result(cmd: Command) -> Result<()> {
    let Command::SaveResult {
        question,
        answer,
        answer_file,
        query_type,
        nodes,
        memory_dir,
        outcome,
        correction,
    } = cmd
    else {
        unreachable!("dispatch_save_result invoked with wrong variant")
    };
    // `--answer-file` lets callers pass a long/multiline answer via a file instead
    // of a fragile inline arg (Windows/PowerShell quoting), #1502. It wins over
    // `--answer`; with neither, fail with a message naming both flags. The file
    // content is preserved exactly (indentation, trailing newlines) to match
    // inline `--answer`, which is unstripped — diverging from graphify-py
    // (__main__.py:2982), which `.strip()`s the file.
    let answer = match answer_file {
        Some(path) => std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("--answer-file {}: {e}", path.display()))?,
        None => answer.ok_or_else(|| anyhow::anyhow!("--answer or --answer-file is required"))?,
    };
    cli::save_result::cmd_save_result(
        &question,
        &answer,
        &query_type,
        &nodes,
        &memory_dir,
        outcome.as_deref(),
        correction.as_deref(),
    )
}

fn dispatch_reflect(cmd: Command) -> Result<()> {
    let Command::Reflect {
        memory_dir,
        out,
        graph,
        analysis,
        labels,
        half_life_days,
        min_corroboration,
        if_stale,
    } = cmd
    else {
        unreachable!("dispatch_reflect invoked with wrong variant")
    };
    cli::reflect::cmd_reflect(cli::reflect::ReflectArgs {
        memory_dir,
        out,
        graph,
        analysis,
        labels,
        half_life_days,
        min_corroboration,
        if_stale,
    })
}

fn dispatch_tree(cmd: Command) -> Result<()> {
    let Command::Tree {
        graph,
        output,
        root,
        max_children,
        top_k_edges,
        label,
    } = cmd
    else {
        unreachable!("dispatch_tree invoked with wrong variant")
    };
    cli::tree::cmd_tree(
        graph.as_deref(),
        output.as_deref(),
        root.as_deref(),
        max_children,
        top_k_edges,
        label.as_deref(),
    )
}

fn dispatch_extract(cmd: Command) -> Result<()> {
    let Command::Extract {
        path,
        backend,
        model,
        mode,
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
        cargo,
        postgres,
        timing,
        code_only,
        force,
    } = cmd
    else {
        unreachable!("dispatch_extract invoked with wrong variant")
    };
    let deep_mode = matches!(mode, Some(cli::args::ExtractMode::Deep));
    cli::extract::cmd_extract(cli::extract::ExtractOptions {
        path: &path,
        out: out.as_deref(),
        exclude: &exclude,
        google_workspace,
        llm: cli::extract::LlmOptions {
            backend: backend.as_deref(),
            model: model.as_deref(),
            deep_mode,
            max_workers,
            token_budget,
            max_concurrency,
            api_timeout,
            dedup_llm,
        },
        cluster: cli::extract::ClusterOptions {
            no_cluster,
            resolution,
            exclude_hubs,
        },
        global: cli::extract::GlobalOptions {
            global,
            as_tag: as_tag.as_deref(),
        },
        introspect: cli::extract::IntrospectOptions {
            cargo,
            postgres: postgres.as_deref(),
        },
        timing,
        code_only,
        force,
    })
}

fn dispatch_add(cmd: Command) -> Result<()> {
    let Command::Add {
        url,
        author,
        contributor,
        dir,
    } = cmd
    else {
        unreachable!("dispatch_add invoked with wrong variant")
    };
    cli::add::cmd_add(&url, author.as_deref(), contributor.as_deref(), &dir)
}

fn dispatch_prs(cmd: Command) -> Result<()> {
    let Command::Prs {
        number,
        repo,
        base,
        limit,
        triage,
        worktrees,
        conflicts,
        wrong_base,
        graph,
    } = cmd
    else {
        unreachable!("dispatch_prs invoked with wrong variant")
    };
    // Accept `123` or `#123`; mirrors Python's `arg.lstrip("#").isdigit()` branch.
    let parsed_number = number
        .as_deref()
        .map(|s| s.trim_start_matches('#'))
        .and_then(|s| s.parse::<u64>().ok());
    let args = graphify_prs::PrsArgs {
        repo,
        base,
        pr_number: parsed_number,
        do_triage: triage,
        do_worktrees: worktrees,
        do_conflicts: conflicts,
        show_wrong_base: wrong_base,
        graph_path: graph.or_else(|| Some(std::path::PathBuf::from("graphify-out/graph.json"))),
        limit,
    };
    cli::prs::cmd_prs(&args)
}
