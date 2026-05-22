//! `graphify` CLI binary.
//!
//! Ports `graphify-py/graphify/__main__.py`.

#![allow(clippy::too_many_lines)]
#![allow(clippy::missing_errors_doc)]

mod cli;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "graphify",
    version,
    about = "Turn any folder of code, docs, papers, images, or videos into a queryable knowledge graph"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Validate an extraction JSON file against the graphify schema.
    Validate {
        /// Path to the extraction JSON file.
        path: PathBuf,
    },

    /// Install the graphify skill to a platform config directory.
    Install {
        /// Target platform (claude, windows, codex, opencode, aider, claw, droid,
        /// trae, trae-cn, gemini, cursor, antigravity, hermes, kiro, pi).
        ///
        /// Accepts either `graphify install --platform <name>` or the
        /// positional shorthand `graphify install <name>` for parity with
        /// Python's argv-parsing fallback (see `__main__.py:1358`).
        #[arg(long)]
        platform: Option<String>,
        /// Optional positional platform (mutually exclusive with `--platform`).
        platform_positional: Option<String>,
    },

    /// Remove graphify from all detected platforms in one shot.
    Uninstall {
        /// Also delete graphify-out/ directory.
        #[arg(long)]
        purge: bool,
    },

    /// Manage git hooks (post-commit/post-checkout).
    Hook {
        #[command(subcommand)]
        cmd: HookCmd,
    },

    /// Manage the global graph (~/.graphify/global-graph.json).
    Global {
        #[command(subcommand)]
        cmd: GlobalCmd,
    },

    /// Measure token reduction vs naive full-corpus approach.
    Benchmark {
        /// Path to graph.json (default graphify-out/graph.json).
        graph: Option<PathBuf>,
    },

    /// Watch a folder and rebuild the graph on code changes.
    Watch { path: PathBuf },

    /// Re-extract code files and update the graph (no LLM needed).
    Update {
        path: PathBuf,
        #[arg(long)]
        force: bool,
        #[arg(long = "no-cluster")]
        no_cluster: bool,
    },

    /// Rerun clustering on an existing graph.json and regenerate report.
    #[command(name = "cluster-only")]
    ClusterOnly {
        path: PathBuf,
        #[arg(long = "no-viz")]
        no_viz: bool,
        #[arg(long)]
        graph: Option<PathBuf>,
        #[arg(long, default_value_t = 1.0)]
        resolution: f64,
        #[arg(long = "exclude-hubs")]
        exclude_hubs: Option<f64>,
        #[arg(long = "min-community-size", default_value_t = 3)]
        min_community_size: usize,
    },

    /// BFS traversal of graph.json for a question.
    Query {
        question: String,
        #[arg(long)]
        dfs: bool,
        #[arg(long)]
        context: Vec<String>,
        #[arg(long, default_value_t = 2000)]
        budget: usize,
        #[arg(long)]
        graph: Option<PathBuf>,
    },

    /// Shortest path between two nodes in graph.json.
    Path {
        from: String,
        to: String,
        #[arg(long)]
        graph: Option<PathBuf>,
    },

    /// Plain-language explanation of a node and its neighbors.
    Explain {
        node: String,
        #[arg(long)]
        graph: Option<PathBuf>,
    },

    /// Save a Q&A result to graphify-out/memory/ for graph feedback loop.
    #[command(name = "save-result")]
    SaveResult {
        #[arg(long)]
        question: String,
        #[arg(long)]
        answer: String,
        #[arg(long = "type", default_value = "query")]
        query_type: String,
        #[arg(long, num_args = 0..)]
        nodes: Vec<String>,
        #[arg(long = "memory-dir", default_value = "graphify-out/memory")]
        memory_dir: PathBuf,
    },

    /// Check `needs_update` flag and notify if semantic re-extraction is pending.
    #[command(name = "check-update")]
    CheckUpdate { path: PathBuf },

    /// Emit a D3 v7 collapsible-tree HTML for graph.json.
    Tree {
        #[arg(long)]
        graph: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        root: Option<PathBuf>,
        #[arg(long = "max-children", default_value_t = 200)]
        max_children: usize,
        #[arg(long = "top-k-edges", default_value_t = 12)]
        top_k_edges: usize,
        #[arg(long)]
        label: Option<String>,
    },

    /// Headless full extraction (AST + semantic LLM) for CI/scripts.
    Extract {
        path: PathBuf,
        #[arg(long)]
        backend: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long = "max-workers")]
        max_workers: Option<usize>,
        #[arg(long = "token-budget", default_value_t = 60_000)]
        token_budget: usize,
        #[arg(long = "max-concurrency", default_value_t = 4)]
        max_concurrency: usize,
        #[arg(long = "api-timeout", default_value_t = 600)]
        api_timeout: u64,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long = "google-workspace")]
        google_workspace: bool,
        #[arg(long = "no-cluster")]
        no_cluster: bool,
        #[arg(long = "global")]
        global: bool,
        #[arg(long = "as", value_name = "TAG")]
        as_tag: Option<String>,
        /// Louvain resolution parameter (default 1.0; >1 = more, smaller communities).
        #[arg(long, default_value_t = 1.0)]
        resolution: f64,
        /// Exclude hub nodes above this degree percentile (0.0–1.0) before clustering.
        #[arg(long = "exclude-hubs")]
        exclude_hubs: Option<f64>,
        /// Extra path globs to exclude from detection (repeatable).
        #[arg(long = "exclude")]
        exclude: Vec<String>,
        /// Run LLM-driven dedup tiebreak after clustering (deferred — currently a no-op).
        #[arg(long = "dedup-llm")]
        dedup_llm: bool,
    },

    /// Export graph to various formats.
    Export {
        #[command(subcommand)]
        cmd: ExportCmd,
    },

    /// Fetch a URL and save it to ./raw, then update the graph.
    Add {
        url: String,
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        contributor: Option<String>,
        #[arg(long, default_value = "./raw")]
        dir: PathBuf,
    },

    /// Clone a GitHub repo locally and print its path for /graphify.
    Clone {
        url: String,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Git merge driver: union-merge two graph.json files.
    #[command(name = "merge-driver")]
    MergeDriver {
        base: PathBuf,
        current: PathBuf,
        other: PathBuf,
    },

    /// Merge two or more graph.json files into one cross-repo graph.
    #[command(name = "merge-graphs")]
    MergeGraphs {
        graphs: Vec<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Merge multiple extraction JSON chunk files into one.
    #[command(name = "merge-chunks")]
    MergeChunks {
        chunks: Vec<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },

    /// Merge cached semantic results with fresh extraction output.
    #[command(name = "merge-semantic")]
    MergeSemantic {
        #[arg(long)]
        cached: Option<PathBuf>,
        #[arg(long)]
        new: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },

    /// GitHub PR dashboard.
    Prs {
        /// Optional PR number to drill into. Accepts both `123` and `--number 123`
        /// for parity with Python's positional/digit argv detection.
        number: Option<String>,
        #[arg(long)]
        repo: Option<String>,
        /// Base branch to filter PRs by. Defaults to the repo's default branch.
        #[arg(long, short = 'b')]
        base: Option<String>,
        /// Cap the number of PRs fetched from GitHub. Defaults to 30.
        #[arg(long, default_value_t = 30)]
        limit: usize,
        #[arg(long)]
        triage: bool,
        #[arg(long)]
        worktrees: bool,
        #[arg(long)]
        conflicts: bool,
        #[arg(long = "wrong-base")]
        wrong_base: bool,
        /// Path to graph.json for impact analysis (default `graphify-out/graph.json`).
        #[arg(long)]
        graph: Option<PathBuf>,
    },

    /// MCP stdio server.
    Serve {
        #[arg(long)]
        graph: Option<PathBuf>,
    },

    /// Check semantic cache for a list of files.
    #[command(name = "cache-check")]
    CacheCheck {
        files_from: PathBuf,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },

    /// Silent gate run on every editor tool use.
    #[command(name = "hook-check")]
    HookCheck,

    // Platform-specific install/uninstall (delegate to graphify-hooks).
    Claude {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    Gemini {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    Cursor {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    Vscode {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    Copilot {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    Kiro {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    Pi {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    Antigravity {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    Codex {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    Opencode {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    Aider {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    Claw {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    Droid {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    Trae {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    #[command(name = "trae-cn")]
    TraeCn {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    Hermes {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum HookCmd {
    /// Install post-commit/post-checkout git hooks.
    Install,
    /// Remove git hooks.
    Uninstall,
    /// Check if git hooks are installed.
    Status,
}

#[derive(Debug, Subcommand)]
pub(crate) enum GlobalCmd {
    /// Add or update a project graph in the global graph.
    Add {
        graph: PathBuf,
        #[arg(long = "as", value_name = "TAG")]
        as_tag: Option<String>,
    },
    /// Remove a repo's nodes from the global graph.
    Remove { tag: String },
    /// List repos in the global graph.
    List,
    /// Print path to the global graph file.
    Path,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ExportCmd {
    /// Mermaid-based architecture/call-flow HTML.
    #[command(name = "callflow-html")]
    CallflowHtml {
        #[arg(long)]
        graph: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long = "max-sections")]
        max_sections: Option<usize>,
        #[arg(long = "diagram-scale")]
        diagram_scale: Option<f64>,
        #[arg(long = "max-diagram-nodes")]
        max_diagram_nodes: Option<usize>,
        #[arg(long = "max-diagram-edges")]
        max_diagram_edges: Option<usize>,
        #[arg(long)]
        report: Option<PathBuf>,
        #[arg(long)]
        sections: Option<PathBuf>,
    },
    Html {
        #[arg(long)]
        graph: Option<PathBuf>,
        /// Path to community labels JSON (default: `<graph_dir>/.graphify_labels.json`).
        #[arg(long)]
        labels: Option<PathBuf>,
        /// Skip graph.html and delete an existing one if present.
        #[arg(long = "no-viz")]
        no_viz: bool,
        /// Suppress rendering when the graph has more than this many nodes.
        #[arg(long = "node-limit", default_value_t = 5000)]
        node_limit: usize,
    },
    Obsidian {
        #[arg(long)]
        graph: Option<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
        /// Path to community labels JSON (default: `<graph_dir>/.graphify_labels.json`).
        #[arg(long)]
        labels: Option<PathBuf>,
    },
    Wiki {
        #[arg(long)]
        graph: Option<PathBuf>,
        /// Path to community labels JSON (default: `<graph_dir>/.graphify_labels.json`).
        #[arg(long)]
        labels: Option<PathBuf>,
    },
    Svg {
        #[arg(long)]
        graph: Option<PathBuf>,
        /// Path to community labels JSON (default: `<graph_dir>/.graphify_labels.json`).
        #[arg(long)]
        labels: Option<PathBuf>,
    },
    Graphml {
        #[arg(long)]
        graph: Option<PathBuf>,
    },
    Neo4j {
        #[arg(long)]
        graph: Option<PathBuf>,
        /// Push directly to a live Neo4j instance at this URI (e.g. `bolt://host:7687`).
        #[arg(long)]
        push: Option<String>,
        /// Neo4j username for `--push` (default `neo4j`).
        #[arg(long, default_value = "neo4j")]
        user: String,
        /// Neo4j password for `--push` (or set `NEO4J_PASSWORD`).
        #[arg(long)]
        password: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum PlatformCmd {
    /// Install graphify integration for this platform.
    Install,
    /// Uninstall graphify integration for this platform.
    Uninstall,
}

fn main() -> Result<()> {
    // Register a best-effort stat-index flush on process exit.
    graphify_cache::ensure_atexit_flush_registered();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.command {
        None => {
            println!("graphify {} — run with --help", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(cmd) => dispatch(cmd),
    }
}

#[allow(clippy::too_many_lines)]
fn dispatch(cmd: Command) -> Result<()> {
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
        Command::HookCheck => Ok(()), // silent no-op until needs_update logic ports
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
