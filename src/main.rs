//! `graphify` CLI binary.
//!
//! Ports `graphify-py/graphify/__main__.py`.

#![allow(clippy::too_many_lines)]
#![allow(clippy::missing_errors_doc)]

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use graphify_hooks::platform::{
    agents_install, agents_uninstall, antigravity_install, antigravity_uninstall, claude_install,
    claude_uninstall, copilot_install, copilot_uninstall, cursor_install, cursor_uninstall,
    gemini_install, gemini_uninstall, install_platform_skill, kiro_install, kiro_uninstall,
    pi_install, pi_uninstall, uninstall_all, vscode_install, vscode_uninstall,
};

#[derive(Debug, Parser)]
#[command(
    name = "graphify",
    version,
    about = "Turn any folder of code, docs, papers, images, or videos into a queryable knowledge graph"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate an extraction JSON file against the graphify schema.
    Validate {
        /// Path to the extraction JSON file.
        path: PathBuf,
    },

    /// Install the graphify skill to a platform config directory.
    Install {
        /// Target platform (claude, windows, codex, opencode, aider, claw, droid,
        /// trae, trae-cn, gemini, cursor, antigravity, hermes, kiro, pi).
        #[arg(long, default_value = "claude")]
        platform: String,
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

    /// GitHub PR dashboard.
    Prs {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long, default_value_t = 30)]
        limit: usize,
    },

    /// MCP stdio server.
    Serve {
        #[arg(long)]
        graph: Option<PathBuf>,
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
}

#[derive(Debug, Subcommand)]
enum HookCmd {
    /// Install post-commit/post-checkout git hooks.
    Install,
    /// Remove git hooks.
    Uninstall,
    /// Check if git hooks are installed.
    Status,
}

#[derive(Debug, Subcommand)]
enum GlobalCmd {
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
enum ExportCmd {
    /// Mermaid-based architecture/call-flow HTML.
    #[command(name = "callflow-html")]
    CallflowHtml {
        #[arg(long)]
        graph: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Html {
        #[arg(long)]
        graph: Option<PathBuf>,
    },
    Obsidian {
        #[arg(long)]
        graph: Option<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    Wiki {
        #[arg(long)]
        graph: Option<PathBuf>,
    },
    Svg {
        #[arg(long)]
        graph: Option<PathBuf>,
    },
    Graphml {
        #[arg(long)]
        graph: Option<PathBuf>,
    },
    Neo4j {
        #[arg(long)]
        graph: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum PlatformCmd {
    /// Install graphify integration for this platform.
    Install,
    /// Uninstall graphify integration for this platform.
    Uninstall,
}

fn main() -> Result<()> {
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

fn dispatch(cmd: Command) -> Result<()> {
    match cmd {
        Command::Validate { path } => cmd_validate(&path),
        Command::Install { platform } => {
            let msg = install_platform_skill(&platform)?;
            println!("{msg}");
            Ok(())
        }
        Command::Uninstall { purge } => {
            let cwd = std::env::current_dir()?;
            let msg = uninstall_all(&cwd, purge)?;
            println!("{msg}");
            Ok(())
        }
        Command::Hook { cmd } => cmd_hook(&cmd),
        Command::Global { cmd } => cmd_global(cmd),
        Command::Benchmark { graph } => cmd_benchmark(graph.as_deref()),
        Command::Watch { path } => {
            eprintln!(
                "watching {} (debounce=1s, Ctrl-C to stop) ...",
                path.display()
            );
            graphify_watch::watch(&path, 1.0)?;
            Ok(())
        }
        Command::Update {
            path,
            force,
            no_cluster,
        } => cmd_update(&path, force, no_cluster),
        Command::ClusterOnly {
            path,
            no_viz,
            graph,
        } => cmd_cluster_only(&path, no_viz, graph.as_deref()),
        Command::Query {
            question,
            dfs,
            context,
            budget,
            graph,
        } => cmd_query(&question, dfs, &context, budget, graph.as_deref()),
        Command::Path { from, to, graph } => cmd_path(&from, &to, graph.as_deref()),
        Command::Explain { node, graph } => cmd_explain(&node, graph.as_deref()),
        Command::SaveResult {
            question,
            answer,
            query_type,
            nodes,
            memory_dir,
        } => cmd_save_result(&question, &answer, &query_type, &nodes, &memory_dir),
        Command::CheckUpdate { path } => cmd_check_update(&path),
        Command::Tree {
            graph,
            output,
            root,
            max_children: _,
            top_k_edges: _,
            label: _,
        } => cmd_tree(graph.as_deref(), output.as_deref(), root.as_deref()),
        Command::Extract {
            path,
            no_cluster,
            out,
            ..
        } => cmd_extract(&path, no_cluster, out.as_deref()),
        Command::Export { cmd } => cmd_export(cmd),
        Command::Add {
            url,
            author,
            contributor,
            dir,
        } => cmd_add(&url, author.as_deref(), contributor.as_deref(), &dir),
        Command::Clone { url, branch, out } => cmd_clone(&url, branch.as_deref(), out.as_deref()),
        Command::MergeDriver {
            base,
            current,
            other,
        } => cmd_merge_driver(&base, &current, &other),
        Command::MergeGraphs { graphs, out } => cmd_merge_graphs(&graphs, out.as_deref()),
        Command::Prs { repo, limit } => cmd_prs(repo.as_deref(), limit),
        Command::Serve { graph } => cmd_serve(graph.as_deref()),
        Command::HookCheck => Ok(()), // silent no-op until needs_update logic ports
        Command::Claude { cmd: c } => cmd_platform("claude", &c),
        Command::Gemini { cmd: c } => cmd_platform("gemini", &c),
        Command::Cursor { cmd: c } => cmd_platform("cursor", &c),
        Command::Vscode { cmd: c } => cmd_platform("vscode", &c),
        Command::Copilot { cmd: c } => cmd_platform("copilot", &c),
        Command::Kiro { cmd: c } => cmd_platform("kiro", &c),
        Command::Pi { cmd: c } => cmd_platform("pi", &c),
        Command::Antigravity { cmd: c } => cmd_platform("antigravity", &c),
        Command::Codex { cmd: c } => cmd_platform("codex", &c),
        Command::Opencode { cmd: c } => cmd_platform("opencode", &c),
        Command::Aider { cmd: c } => cmd_platform("aider", &c),
        Command::Claw { cmd: c } => cmd_platform("claw", &c),
        Command::Droid { cmd: c } => cmd_platform("droid", &c),
        Command::Trae { cmd: c } => cmd_platform("trae", &c),
    }
}

fn cmd_validate(path: &std::path::Path) -> Result<()> {
    let contents = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&contents)?;
    graphify_validate::assert_valid(&value)?;
    println!("OK: {}", path.display());
    Ok(())
}

fn cmd_hook(cmd: &HookCmd) -> Result<()> {
    let cwd = std::env::current_dir()?;
    match cmd {
        HookCmd::Install => {
            let msg = graphify_hooks::install(&cwd)?;
            println!("{msg}");
        }
        HookCmd::Uninstall => {
            let msg = graphify_hooks::uninstall(&cwd)?;
            println!("{msg}");
        }
        HookCmd::Status => {
            println!("{}", graphify_hooks::status(&cwd));
        }
    }
    Ok(())
}

fn cmd_global(cmd: GlobalCmd) -> Result<()> {
    match cmd {
        GlobalCmd::Add { graph, as_tag } => {
            let tag = as_tag.unwrap_or_else(|| {
                graph
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            });
            let manifest_path = graphify_global::global_manifest_path();
            let global_path = graphify_global::global_graph_path();
            let summary = graphify_global::global_add(&graph, &tag, &global_path, &manifest_path)?;
            println!(
                "added {} ({} nodes added, {} removed)",
                summary.repo_tag, summary.nodes_added, summary.nodes_removed
            );
        }
        GlobalCmd::Remove { tag } => {
            let manifest_path = graphify_global::global_manifest_path();
            let global_path = graphify_global::global_graph_path();
            let removed = graphify_global::global_remove(&tag, &global_path, &manifest_path)?;
            println!("removed {removed} nodes for tag '{tag}'");
        }
        GlobalCmd::List => {
            let manifest_path = graphify_global::global_manifest_path();
            let entries = graphify_global::global_list(&manifest_path);
            for (tag, entry) in &entries {
                println!(
                    "{tag}\t{} nodes\t{} edges\t{}",
                    entry.node_count, entry.edge_count, entry.added_at
                );
            }
        }
        GlobalCmd::Path => {
            println!("{}", graphify_global::global_graph_path().display());
        }
    }
    Ok(())
}

fn cmd_benchmark(graph: Option<&std::path::Path>) -> Result<()> {
    let default_path = std::path::PathBuf::from("graphify-out/graph.json");
    let path = graph.unwrap_or(default_path.as_path());
    eprintln!("benchmarking against {} ...", path.display());
    let start = std::time::Instant::now();
    let result = graphify_benchmark::run_benchmark(path, None, None)?;
    eprintln!("done in {:.1}s", start.elapsed().as_secs_f64());
    println!("{}", graphify_benchmark::format_benchmark(result.as_ref()));
    Ok(())
}

fn cmd_check_update(path: &std::path::Path) -> Result<()> {
    if graphify_watch::check_update(path) {
        std::process::exit(0);
    }
    std::process::exit(1);
}

fn cmd_clone(url: &str, branch: Option<&str>, out: Option<&std::path::Path>) -> Result<()> {
    use std::process::Command as Proc;
    eprintln!("cloning {url} ...");
    let target = out.map_or_else(
        || {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            let safe = url
                .replace("https://", "")
                .replace("http://", "")
                .replace('/', "_");
            PathBuf::from(home)
                .join(".graphify")
                .join("repos")
                .join(safe)
        },
        std::path::Path::to_path_buf,
    );
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut cmd = Proc::new("git");
    cmd.arg("clone");
    if let Some(b) = branch {
        cmd.arg("--branch").arg(b);
    }
    cmd.arg(url).arg(&target);
    let status = cmd.status()?;
    if !status.success() {
        anyhow::bail!("git clone failed with status {status}");
    }
    println!("{}", target.display());
    Ok(())
}

fn cmd_prs(repo: Option<&str>, _limit: usize) -> Result<()> {
    eprintln!(
        "fetching PRs{} via gh CLI ...",
        repo.map(|r| format!(" for {r}")).unwrap_or_default()
    );
    let args = graphify_prs::PrsArgs {
        repo: repo.map(str::to_string),
        ..Default::default()
    };
    graphify_prs::run_cmd_prs(
        &graphify_prs::gh::ProcessGhClient,
        &graphify_prs::git::ProcessGitClient,
        &graphify_prs::triage::NoOpTriageBackend,
        &args,
    )?;
    Ok(())
}

fn cmd_platform(platform: &str, cmd: &PlatformCmd) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let msg = match (platform, cmd) {
        ("claude", PlatformCmd::Install) => claude_install(&cwd)?,
        ("claude", PlatformCmd::Uninstall) => claude_uninstall(&cwd)?,
        ("gemini", PlatformCmd::Install) => gemini_install(&cwd)?,
        ("gemini", PlatformCmd::Uninstall) => gemini_uninstall(&cwd)?,
        ("vscode", PlatformCmd::Install) => vscode_install(&cwd)?,
        ("vscode", PlatformCmd::Uninstall) => vscode_uninstall(&cwd)?,
        ("copilot", PlatformCmd::Install) => copilot_install()?,
        ("copilot", PlatformCmd::Uninstall) => copilot_uninstall()?,
        ("kiro", PlatformCmd::Install) => kiro_install(&cwd)?,
        ("kiro", PlatformCmd::Uninstall) => kiro_uninstall(&cwd)?,
        ("pi", PlatformCmd::Install) => pi_install()?,
        ("pi", PlatformCmd::Uninstall) => pi_uninstall()?,
        ("antigravity", PlatformCmd::Install) => antigravity_install(&cwd)?,
        ("antigravity", PlatformCmd::Uninstall) => antigravity_uninstall(&cwd)?,
        ("cursor", PlatformCmd::Install) => cursor_install(&cwd)?,
        ("cursor", PlatformCmd::Uninstall) => cursor_uninstall(&cwd)?,
        (p, PlatformCmd::Install) => agents_install(&cwd, p)?,
        (p, PlatformCmd::Uninstall) => agents_uninstall(&cwd, p)?,
    };
    println!("{msg}");
    Ok(())
}

fn load_graph(path: &std::path::Path) -> Result<graphify_build::Graph> {
    let contents = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&contents)?;
    let graph = graphify_build::build_from_json(value, true, None)?;
    Ok(graph)
}

fn default_graph_path() -> std::path::PathBuf {
    std::path::PathBuf::from("graphify-out/graph.json")
}

fn cmd_export(cmd: ExportCmd) -> Result<()> {
    match cmd {
        ExportCmd::Graphml { graph } => {
            let path = graph.unwrap_or_else(default_graph_path);
            eprintln!("loading {} ...", path.display());
            let g = load_graph(&path)?;
            let communities = indexmap::IndexMap::new();
            let out = path.with_file_name("graph.graphml");
            eprintln!(
                "exporting GraphML ({} nodes, {} edges) ...",
                g.node_count(),
                g.edge_count()
            );
            graphify_export::to_graphml(&g, &communities, &out)?;
            eprintln!("wrote {}", out.display());
        }
        ExportCmd::Svg { graph } => {
            let path = graph.unwrap_or_else(default_graph_path);
            eprintln!("loading {} ...", path.display());
            let g = load_graph(&path)?;
            let communities = indexmap::IndexMap::new();
            let out = path.with_file_name("graph.svg");
            eprintln!(
                "computing spring layout + rendering SVG for {} nodes ...",
                g.node_count()
            );
            graphify_export::to_svg(&g, &communities, &out, None, (16, 12))?;
            eprintln!("wrote {}", out.display());
        }
        ExportCmd::Html { graph } => {
            let path = graph.unwrap_or_else(default_graph_path);
            eprintln!("loading {} ...", path.display());
            let g = load_graph(&path)?;
            let communities = indexmap::IndexMap::new();
            let out = path.with_file_name("graph.html");
            eprintln!("rendering HTML viz ({} nodes) ...", g.node_count());
            graphify_export::to_html(&g, &communities, &out, None, None, None)?;
            eprintln!("wrote {}", out.display());
        }
        ExportCmd::Obsidian { graph, out } => {
            let path = graph.unwrap_or_else(default_graph_path);
            eprintln!("loading {} ...", path.display());
            let g = load_graph(&path)?;
            let communities = indexmap::IndexMap::new();
            let out_dir = out.unwrap_or_else(|| std::path::PathBuf::from("graphify-out/obsidian"));
            eprintln!(
                "rendering Obsidian vault ({} nodes) to {} ...",
                g.node_count(),
                out_dir.display()
            );
            let count = graphify_export::to_obsidian(&g, &communities, &out_dir, None, None)?;
            eprintln!("wrote {count} notes to {}", out_dir.display());
        }
        ExportCmd::Wiki { graph: _ } => {
            anyhow::bail!("export wiki: not yet wired to graphify-wiki (needs cluster output)")
        }
        ExportCmd::CallflowHtml { graph, output } => {
            eprintln!("rendering Mermaid call-flow HTML ...");
            let opts = graphify_html::callflow::CallflowOptions {
                graph,
                output,
                ..Default::default()
            };
            let written = graphify_html::callflow::write_callflow_html(&opts)?;
            eprintln!("wrote {}", written.display());
        }
        ExportCmd::Neo4j { graph: _ } => {
            anyhow::bail!("export neo4j: requires neo4rs integration (deferred)")
        }
    }
    Ok(())
}

fn cmd_add(
    url: &str,
    author: Option<&str>,
    contributor: Option<&str>,
    dir: &std::path::Path,
) -> Result<()> {
    eprintln!("fetching {url} ...");
    let path = graphify_ingest::ingest(url, dir, author, contributor)?;
    eprintln!("wrote {}", path.display());
    Ok(())
}

fn cmd_tree(
    graph: Option<&std::path::Path>,
    output: Option<&std::path::Path>,
    root: Option<&std::path::Path>,
) -> Result<()> {
    let graph_path = graph.map_or_else(default_graph_path, std::path::Path::to_path_buf);
    eprintln!("loading {} ...", graph_path.display());
    let g = load_graph(&graph_path)?;
    let default_root = std::env::current_dir()?;
    let root_path = root.unwrap_or(default_root.as_path());
    let default_output = graph_path.with_file_name("GRAPH_TREE.html");
    let out = output.unwrap_or(default_output.as_path());
    eprintln!(
        "rendering tree HTML for {} nodes rooted at {} ...",
        g.node_count(),
        root_path.display()
    );
    graphify_html::tree::write_tree_html(&g, root_path, out)?;
    eprintln!("wrote {}", out.display());
    Ok(())
}

const MERGE_MAX_BYTES: u64 = 50 * 1024 * 1024;
const MERGE_MAX_NODES: usize = 100_000;

fn read_graph_capped(path: &std::path::Path) -> Result<serde_json::Value> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| anyhow::anyhow!("cannot stat {}: {e}", path.display()))?;
    if metadata.len() > MERGE_MAX_BYTES {
        anyhow::bail!(
            "graph.json {} is {} bytes, exceeds {}-byte cap",
            path.display(),
            metadata.len(),
            MERGE_MAX_BYTES
        );
    }
    let contents = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&contents)?;
    Ok(value)
}

fn merge_two_graphs(a: serde_json::Value, b: serde_json::Value) -> Result<serde_json::Value> {
    use serde_json::Value;
    let mut nodes_by_id: indexmap::IndexMap<String, Value> = indexmap::IndexMap::new();
    let mut edges: Vec<Value> = Vec::new();
    let mut hyperedges: Vec<Value> = Vec::new();
    for graph in [a, b] {
        let obj = graph
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("graph is not a JSON object"))?
            .clone();
        let nodes = obj
            .get("nodes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for node in nodes {
            let id = node
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if !id.is_empty() {
                nodes_by_id.insert(id, node);
            }
        }
        let edge_arr = obj
            .get("edges")
            .or_else(|| obj.get("links"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        edges.extend(edge_arr);
        let hyper = obj
            .get("hyperedges")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        hyperedges.extend(hyper);
    }
    let mut result = serde_json::Map::new();
    result.insert(
        "nodes".to_string(),
        Value::Array(nodes_by_id.into_values().collect()),
    );
    result.insert("edges".to_string(), Value::Array(edges));
    result.insert("hyperedges".to_string(), Value::Array(hyperedges));
    Ok(Value::Object(result))
}

fn count_nodes(graph: &serde_json::Value) -> usize {
    graph
        .get("nodes")
        .and_then(serde_json::Value::as_array)
        .map_or(0, std::vec::Vec::len)
}

fn cmd_merge_driver(
    _base: &std::path::Path,
    current: &std::path::Path,
    other: &std::path::Path,
) -> Result<()> {
    let cur = read_graph_capped(current)?;
    let oth = read_graph_capped(other)?;
    let merged = merge_two_graphs(cur, oth)?;
    if count_nodes(&merged) > MERGE_MAX_NODES {
        anyhow::bail!("merged graph exceeds {MERGE_MAX_NODES}-node cap; aborting merge");
    }
    let out = serde_json::to_string_pretty(&merged)?;
    std::fs::write(current, out)?;
    Ok(())
}

fn cmd_merge_graphs(graphs: &[std::path::PathBuf], out: Option<&std::path::Path>) -> Result<()> {
    if graphs.len() < 2 {
        anyhow::bail!("merge-graphs requires at least 2 graph files");
    }
    let mut merged = read_graph_capped(&graphs[0])?;
    for g in &graphs[1..] {
        let next = read_graph_capped(g)?;
        merged = merge_two_graphs(merged, next)?;
    }
    let default_out = std::path::PathBuf::from("graphify-out/merged-graph.json");
    let out_path = out.unwrap_or(default_out.as_path());
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(&merged)?;
    std::fs::write(out_path, body)?;
    println!("wrote {}", out_path.display());
    Ok(())
}

fn cmd_update(path: &std::path::Path, force: bool, no_cluster: bool) -> Result<()> {
    cmd_extract(path, no_cluster, None)?;
    let _ = force;
    Ok(())
}

fn cmd_cluster_only(
    path: &std::path::Path,
    no_viz: bool,
    graph: Option<&std::path::Path>,
) -> Result<()> {
    let start = std::time::Instant::now();
    let graph_path = graph.map_or_else(
        || path.join("graphify-out").join("graph.json"),
        std::path::Path::to_path_buf,
    );
    eprintln!("[1/4] loading {} ...", graph_path.display());
    let g = load_graph(&graph_path)?;
    eprintln!(
        "      loaded {} nodes, {} edges",
        g.node_count(),
        g.edge_count()
    );

    eprintln!("[2/4] clustering (Louvain, resolution=1.0) ...");
    let cluster_start = std::time::Instant::now();
    let communities = graphify_cluster::cluster(&g, 1.0, None);
    eprintln!(
        "      found {} communities in {:.1}s",
        communities.len(),
        cluster_start.elapsed().as_secs_f64()
    );

    eprintln!("[3/4] writing report ...");
    let analysis = build_analysis(&g, &communities, path);
    let report_path = graph_path.with_file_name("GRAPH_REPORT.md");
    graphify_report::write_report(&g, &analysis, &report_path)?;
    eprintln!("      wrote {}", report_path.display());

    if no_viz {
        eprintln!("[4/4] HTML viz: skipped (--no-viz)");
    } else {
        eprintln!("[4/4] rendering HTML viz ...");
        let html_path = graph_path.with_file_name("graph.html");
        match graphify_export::to_html(&g, &communities, &html_path, None, None, None) {
            Ok(()) => eprintln!("      wrote {}", html_path.display()),
            Err(e) => eprintln!("      skipped ({e})"),
        }
    }
    eprintln!("done in {:.1}s", start.elapsed().as_secs_f64());
    Ok(())
}

fn cmd_extract(
    path: &std::path::Path,
    no_cluster: bool,
    out: Option<&std::path::Path>,
) -> Result<()> {
    let start = std::time::Instant::now();

    eprintln!("[1/6] detecting files in {} ...", path.display());
    let detect = graphify_detect::detect(path, None, None);
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut by_kind: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for (kind, paths) in &detect.files {
        by_kind.insert(kind.as_str(), paths.len());
        if kind == "code" || kind == "document" {
            for p in paths {
                files.push(path.join(p));
            }
        }
    }
    let kinds_summary = by_kind
        .iter()
        .map(|(k, n)| format!("{n} {k}"))
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!(
        "      detected {} files ({kinds_summary}); will extract from {}",
        detect.total_files,
        files.len()
    );

    eprintln!("[2/6] extracting AST from {} files ...", files.len());
    let extract_start = std::time::Instant::now();
    let extraction = graphify_extract::extract(&files, Some(path));
    eprintln!(
        "      extracted {} nodes, {} edges in {:.1}s",
        extraction.nodes.len(),
        extraction.edges.len(),
        extract_start.elapsed().as_secs_f64()
    );

    let extraction_json = serde_json::json!({
        "nodes": extraction.nodes,
        "edges": extraction.edges,
        "hyperedges": [],
    });
    let out_dir = out.map_or_else(|| path.join("graphify-out"), std::path::Path::to_path_buf);
    std::fs::create_dir_all(&out_dir)?;
    let extraction_path = out_dir.join("stage_02_extract.json");
    std::fs::write(
        &extraction_path,
        serde_json::to_string_pretty(&extraction_json)?,
    )?;
    eprintln!("      wrote {}", extraction_path.display());

    eprintln!("[3/6] building graph ...");
    let graph = graphify_build::build_from_json(extraction_json, true, Some(path))?;
    eprintln!(
        "      built graph: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );
    let graph_path = out_dir.join("graph.json");

    let communities = if no_cluster {
        eprintln!("[4/6] clustering: skipped (--no-cluster)");
        indexmap::IndexMap::new()
    } else {
        eprintln!(
            "[4/6] clustering (Louvain, resolution=1.0) on {} nodes ...",
            graph.node_count()
        );
        let cluster_start = std::time::Instant::now();
        let c = graphify_cluster::cluster(&graph, 1.0, None);
        eprintln!(
            "      found {} communities in {:.1}s",
            c.len(),
            cluster_start.elapsed().as_secs_f64()
        );
        c
    };
    graphify_export::to_json(&graph, &communities, &graph_path, true, None)?;
    eprintln!("      wrote {}", graph_path.display());

    if no_cluster {
        eprintln!("done in {:.1}s", start.elapsed().as_secs_f64());
        return Ok(());
    }

    eprintln!("[5/6] analyzing (god nodes, surprising connections, suggested questions) ...");
    let analyze_start = std::time::Instant::now();
    let analysis = build_analysis(&graph, &communities, path);
    eprintln!(
        "      analysis done in {:.1}s",
        analyze_start.elapsed().as_secs_f64()
    );
    let report_path = out_dir.join("GRAPH_REPORT.md");
    graphify_report::write_report(&graph, &analysis, &report_path)?;
    eprintln!("      wrote {}", report_path.display());

    eprintln!("[6/6] rendering HTML viz ...");
    let html_path = out_dir.join("graph.html");
    match graphify_export::to_html(&graph, &communities, &html_path, None, None, None) {
        Ok(()) => eprintln!("      wrote {}", html_path.display()),
        Err(e) => eprintln!("      skipped ({e})"),
    }

    eprintln!("done in {:.1}s", start.elapsed().as_secs_f64());
    Ok(())
}

/// Build the analysis JSON consumed by `graphify_report::write_report`.
///
/// Mirrors the shape produced by Python's `analyze.generate(...)` for the
/// minimum set of fields the report renderer reads.
fn build_analysis(
    graph: &graphify_build::Graph,
    communities: &indexmap::IndexMap<i64, Vec<String>>,
    root: &std::path::Path,
) -> serde_json::Value {
    let mut communities_json = serde_json::Map::new();
    for (cid, members) in communities {
        communities_json.insert(
            cid.to_string(),
            serde_json::Value::Array(
                members
                    .iter()
                    .map(|m| serde_json::Value::String(m.clone()))
                    .collect(),
            ),
        );
    }
    let god_nodes = graphify_analyze::god_nodes(graph, 12);
    let surprising = graphify_analyze::surprising_connections(graph, communities, 12);
    let empty_labels: indexmap::IndexMap<i64, String> = indexmap::IndexMap::new();
    let suggested = graphify_analyze::suggest_questions(graph, communities, &empty_labels, 8);
    serde_json::json!({
        "root": root.display().to_string(),
        "communities": serde_json::Value::Object(communities_json),
        "god_nodes": god_nodes,
        "surprising_connections": surprising,
        "suggested_questions": suggested,
        "min_community_size": 3,
    })
}

fn cmd_save_result(
    question: &str,
    answer: &str,
    query_type: &str,
    nodes: &[String],
    memory_dir: &std::path::Path,
) -> Result<()> {
    let source_nodes = if nodes.is_empty() { None } else { Some(nodes) };
    let path =
        graphify_ingest::save_query_result(question, answer, memory_dir, query_type, source_nodes)?;
    println!("wrote {}", path.display());
    Ok(())
}

fn cmd_query(
    question: &str,
    dfs: bool,
    context: &[String],
    budget: usize,
    graph: Option<&std::path::Path>,
) -> Result<()> {
    let path = graph.map_or_else(default_graph_path, std::path::Path::to_path_buf);
    eprintln!("loading {} ...", path.display());
    let g = load_graph(&path)?;
    eprintln!(
        "querying {} ({} nodes, mode={}) ...",
        path.display(),
        g.node_count(),
        if dfs { "dfs" } else { "bfs" }
    );
    let mode = if dfs { "dfs" } else { "bfs" };
    let context_filters: Option<&[String]> = if context.is_empty() {
        None
    } else {
        Some(context)
    };
    let mut idf_cache = std::collections::HashMap::new();
    let result = graphify_serve::graph::query_graph_text(
        &g,
        question,
        mode,
        2,
        budget,
        context_filters,
        &mut idf_cache,
    );
    println!("{result}");
    Ok(())
}

fn cmd_path(from: &str, to: &str, graph: Option<&std::path::Path>) -> Result<()> {
    let path = graph.map_or_else(default_graph_path, std::path::Path::to_path_buf);
    let g = load_graph(&path)?;
    let from_ids = graphify_serve::graph::find_node(&g, from);
    let to_ids = graphify_serve::graph::find_node(&g, to);
    let Some(src) = from_ids.first() else {
        anyhow::bail!("source node not found: {from}");
    };
    let Some(tgt) = to_ids.first() else {
        anyhow::bail!("target node not found: {to}");
    };
    match graphify_serve::graph::shortest_path(&g, src, tgt) {
        Some(p) => println!("{}", p.join(" -> ")),
        None => println!("no path from {from} to {to}"),
    }
    Ok(())
}

fn cmd_explain(node: &str, graph: Option<&std::path::Path>) -> Result<()> {
    let path = graph.map_or_else(default_graph_path, std::path::Path::to_path_buf);
    let g = load_graph(&path)?;
    let ids = graphify_serve::graph::find_node(&g, node);
    let Some(node_id) = ids.first() else {
        anyhow::bail!("node not found: {node}");
    };
    let neighbors = graphify_serve::graph::neighbors(&g, node_id);
    println!("{node_id}");
    for n in neighbors {
        println!("  - {n}");
    }
    Ok(())
}

fn cmd_serve(graph: Option<&std::path::Path>) -> Result<()> {
    let default_path = std::path::PathBuf::from("graphify-out/graph.json");
    let path = graph.unwrap_or(default_path.as_path());
    eprintln!(
        "serving MCP over stdio (graph={}, Ctrl-C to stop) ...",
        path.display()
    );
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(graphify_serve::serve(&path.to_string_lossy()))?;
    Ok(())
}
