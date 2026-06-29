//! Clap definitions for the `graphify` CLI.
//!
//! Ports the `argparse` setup in `graphify-py/graphify/__main__.py`.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// Semantic-extraction modes for `graphify extract --mode`.
///
/// Mirrors graphify-py's `_VALID_MODES`. `deep` biases the LLM toward richer
/// `INFERRED` architectural edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum ExtractMode {
    /// Aggressive INFERRED-edge semantic extraction.
    Deep,
}

/// MCP transport for the `serve` command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum ServeTransport {
    /// Line-delimited JSON-RPC over stdio (the default, per-developer transport).
    Stdio,
    /// Streamable HTTP (MCP spec 2025-03-26); requires the `http` build feature.
    Http,
}

/// Root CLI struct parsed from `argv` by clap.
#[derive(Debug, Parser)]
#[command(
    name = "graphify",
    version = concat!(env!("CARGO_PKG_VERSION"), "-", env!("GIT_SHORT_SHA")),
    about = "Turn any folder of code, docs, papers, images, or videos into a queryable knowledge graph"
)]
pub(crate) struct Cli {
    /// The subcommand to run. When absent, a brief help hint is printed.
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

/// All top-level subcommands exposed by the `graphify` binary.
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
        /// trae, trae-cn, gemini, cursor, antigravity, hermes, kiro, pi, devin).
        ///
        /// Accepts either `graphify install --platform <name>` or the
        /// positional shorthand `graphify install <name>` for parity with
        /// Python's argv-parsing fallback (see `__main__.py:1358`).
        #[arg(long)]
        platform: Option<String>,
        /// Optional positional platform (mutually exclusive with `--platform`).
        platform_positional: Option<String>,
        /// Install the skill under the current project (./.{platform}/skills/...)
        /// instead of the user-global home directory. Mirrors the Python
        /// `--project` flag (#931).
        #[arg(long)]
        project: bool,
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
        /// Keep `Community N` placeholders (skip LLM community naming).
        #[arg(long = "no-label")]
        no_label: bool,
        /// Backend to use for community naming (default: auto-detect).
        #[arg(long)]
        backend: Option<String>,
        /// Model to use for community naming (default: backend default).
        #[arg(long)]
        model: Option<String>,
        /// Max community-label batches sent concurrently (#1390).
        #[arg(long = "max-concurrency", default_value_t = 4)]
        max_concurrency: usize,
        /// Communities per LLM labeling call (#1390).
        #[arg(long = "batch-size", default_value_t = 100)]
        batch_size: usize,
        /// Print per-stage wall-clock timings to stderr (#1490).
        #[arg(long)]
        timing: bool,
        /// Only (re)name communities that are unnamed or hold a `Community N`
        /// placeholder, preserving existing labels (#1481).
        #[arg(long = "missing-only")]
        missing_only: bool,
    },

    /// (Re)name communities with the configured LLM backend, regenerate report.
    ///
    /// Equivalent to `cluster-only` but always refreshes community names even
    /// when a `.graphify_labels.json` already exists.
    Label {
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
        /// Backend to use (default: auto-detect from API keys).
        #[arg(long)]
        backend: Option<String>,
        /// Model to use for community naming (default: backend default).
        #[arg(long)]
        model: Option<String>,
        /// Max community-label batches sent concurrently (#1390).
        #[arg(long = "max-concurrency", default_value_t = 4)]
        max_concurrency: usize,
        /// Communities per LLM labeling call (#1390).
        #[arg(long = "batch-size", default_value_t = 100)]
        batch_size: usize,
        /// Print per-stage wall-clock timings to stderr (#1490).
        #[arg(long)]
        timing: bool,
        /// Only (re)name communities that are unnamed or hold a `Community N`
        /// placeholder, preserving existing labels (#1481).
        #[arg(long = "missing-only")]
        missing_only: bool,
    },

    /// Manage custom LLM providers (`graphify provider <add|list|show|remove>`).
    Provider {
        #[command(subcommand)]
        cmd: ProviderCommand,
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
        /// Work-memory signal: useful | `dead_end` | corrected (#1441).
        #[arg(long)]
        outcome: Option<String>,
        /// What the right answer was (pairs with `--outcome corrected`).
        #[arg(long)]
        correction: Option<String>,
    },

    /// Aggregate graphify-out/memory/ outcomes into a deterministic lessons doc.
    Reflect {
        /// Memory directory (default: `<GRAPHIFY_OUT>/memory`).
        #[arg(long = "memory-dir")]
        memory_dir: Option<PathBuf>,
        /// Output lessons file (default: `<GRAPHIFY_OUT>/reflections/LESSONS.md`).
        #[arg(long)]
        out: Option<PathBuf>,
        /// graph.json for community grouping (default: auto-detect under `<GRAPHIFY_OUT>`).
        #[arg(long)]
        graph: Option<PathBuf>,
        /// `.graphify_analysis.json` override (default: sibling of the graph).
        #[arg(long)]
        analysis: Option<PathBuf>,
        /// `.graphify_labels.json` override (default: sibling of the graph).
        #[arg(long)]
        labels: Option<PathBuf>,
        /// Signal weight halves every N days.
        #[arg(long = "half-life-days", default_value_t = graphify_reflect::DEFAULT_HALF_LIFE_DAYS)]
        half_life_days: f64,
        /// Distinct useful results to promote a node to preferred.
        #[arg(long = "min-corroboration", default_value_t = graphify_reflect::DEFAULT_MIN_CORROBORATION)]
        min_corroboration: usize,
        /// Skip when LESSONS.md is already newer than every input.
        #[arg(long = "if-stale")]
        if_stale: bool,
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
        /// LLM backend for semantic extraction:
        /// `gemini|kimi|claude|openai|deepseek|ollama` (default: whichever API
        /// key is set).
        ///
        /// `openai` also reaches self-hosted `OpenAI`-compatible servers
        /// (`llama.cpp`, `vLLM`, LM Studio): set `OPENAI_BASE_URL`
        /// (e.g. `http://localhost:8080/v1`) and `OPENAI_MODEL` to the model name
        /// your server serves. `claude` also reaches custom Anthropic-compatible
        /// endpoints (`LiteLLM` proxy, gateways): set `ANTHROPIC_BASE_URL` and
        /// `ANTHROPIC_MODEL`.
        #[arg(long)]
        backend: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Semantic-extraction mode. `deep` enables aggressive INFERRED-edge extraction.
        #[arg(long, value_enum)]
        mode: Option<ExtractMode>,
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
        /// Run LLM-driven dedup tiebreak after clustering.
        #[arg(long = "dedup-llm")]
        dedup_llm: bool,
        /// Also extract crate -> crate dependency edges from `Cargo.toml`.
        #[arg(long)]
        cargo: bool,
        /// Also extract schema from a live Postgres database at this DSN.
        #[arg(long, value_name = "DSN")]
        postgres: Option<String>,
        /// Print per-stage wall-clock timings to stderr (#1490).
        #[arg(long)]
        timing: bool,
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
        /// Cap the number of PRs fetched from GitHub. Defaults to 50 (matches
        /// Python `fetch_prs(limit=50)`).
        #[arg(long, default_value_t = 50)]
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

    /// MCP server (stdio by default, or Streamable HTTP).
    Serve {
        #[arg(long)]
        graph: Option<PathBuf>,
        /// Transport to serve on.
        #[arg(long, value_enum, default_value_t = ServeTransport::Stdio)]
        transport: ServeTransport,
        /// HTTP bind host (http transport).
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// HTTP bind port (http transport).
        #[arg(long, default_value_t = 8080)]
        port: u16,
        /// Require this key on the HTTP transport (env: `GRAPHIFY_API_KEY`).
        #[arg(long = "api-key", env = "GRAPHIFY_API_KEY")]
        api_key: Option<String>,
        /// HTTP mount path (http transport).
        #[arg(long, default_value = "/mcp")]
        path: String,
        /// Return plain JSON responses instead of SSE streams (http transport).
        #[arg(long = "json-response")]
        json_response: bool,
        /// Run without per-session state (load-balanced / CI deployments).
        #[arg(long)]
        stateless: bool,
        /// Reap stateful sessions idle this many seconds (0 disables).
        #[arg(long = "session-timeout", default_value_t = 3600.0)]
        session_timeout: f64,
    },

    /// Reverse-traversal impact analysis (`graphify affected <query>`).
    Affected {
        /// Node label, ID, or source file substring to use as the seed.
        query: String,
        /// Edge relation to follow (repeatable). Defaults to the canonical
        /// impact relations (calls/references/imports/...).
        #[arg(long = "relation")]
        relations: Vec<String>,
        /// Maximum BFS depth.
        #[arg(long, default_value_t = 2)]
        depth: usize,
        /// Path to graph.json (default `graphify-out/graph.json`).
        #[arg(long)]
        graph: Option<PathBuf>,
    },

    /// Diagnose graph health (`graphify diagnose <subcommand>`).
    Diagnose {
        #[command(subcommand)]
        cmd: DiagnoseCmd,
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
    /// Install or uninstall graphify integration for Claude.
    Claude {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    // `CodeBuddy` is the product's own camel-case spelling; clap renders this
    // doc comment as user-facing `--help` text, so backticks (which clippy's
    // `doc_markdown` would otherwise demand) must not appear here.
    #[allow(clippy::doc_markdown)]
    /// Install or uninstall graphify integration for CodeBuddy.
    Codebuddy {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for Gemini.
    Gemini {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for Cursor.
    Cursor {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for VS Code.
    Vscode {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for GitHub Copilot.
    Copilot {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for Kiro.
    Kiro {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for Kilo Code.
    Kilo {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for Pi.
    Pi {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for Antigravity.
    Antigravity {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for Codex.
    Codex {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for Amp.
    Amp {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for Opencode.
    Opencode {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for Aider.
    Aider {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for Claw.
    Claw {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for Droid.
    Droid {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for Trae.
    Trae {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for Trae CN.
    #[command(name = "trae-cn")]
    TraeCn {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for Hermes.
    Hermes {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall graphify integration for Devin CLI.
    Devin {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Install or uninstall the generic cross-framework Agent-Skills integration.
    Agents {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
    /// Alias for `agents` (the Agent-Skills ecosystem calls them "skills").
    Skills {
        #[command(subcommand)]
        cmd: PlatformCmd,
    },
}

/// Subcommands for the `hook` command group.
#[derive(Debug, Subcommand)]
pub(crate) enum HookCmd {
    /// Install post-commit/post-checkout git hooks.
    Install,
    /// Remove git hooks.
    Uninstall,
    /// Check if git hooks are installed.
    Status,
}

/// Subcommands for the `provider` command group (custom LLM providers, #1084).
#[derive(Debug, Subcommand)]
pub(crate) enum ProviderCommand {
    /// Register a custom OpenAI-compatible provider in `~/.graphify/providers.json`.
    Add {
        /// Provider name (the `--backend` selector).
        name: String,
        #[arg(long = "base-url")]
        base_url: Option<String>,
        #[arg(long = "default-model")]
        default_model: Option<String>,
        #[arg(long = "env-key")]
        env_key: Option<String>,
        #[arg(long = "pricing-input")]
        pricing_input: Option<f64>,
        #[arg(long = "pricing-output")]
        pricing_output: Option<f64>,
    },
    /// List registered custom providers.
    List,
    /// Print one provider's full configuration as JSON.
    Show { name: String },
    /// Remove a custom provider.
    Remove { name: String },
}

/// Subcommands for the `global` command group.
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

/// Subcommands for the `export` command group.
#[derive(Debug, Subcommand)]
pub(crate) enum ExportCmd {
    /// Mermaid-based architecture/call-flow HTML.
    #[command(name = "callflow-html")]
    CallflowHtml {
        /// Optional positional graph path or directory, mirroring Python's
        /// `export callflow-html [GRAPH|DIR]`. A `*.json` path is used directly;
        /// a directory resolves to `<dir>/graph.json` or `<dir>/graphify-out/graph.json`.
        /// Ignored when `--graph` is supplied.
        path: Option<PathBuf>,
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
    /// Render an interactive D3 force-graph HTML file from `graph.json`.
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
    /// Export graph nodes and edges as an Obsidian markdown vault.
    Obsidian {
        #[arg(long)]
        graph: Option<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
        /// Path to community labels JSON (default: `<graph_dir>/.graphify_labels.json`).
        #[arg(long)]
        labels: Option<PathBuf>,
    },
    /// Export graph as a Markdown wiki with one article per community.
    Wiki {
        #[arg(long)]
        graph: Option<PathBuf>,
        /// Path to community labels JSON (default: `<graph_dir>/.graphify_labels.json`).
        #[arg(long)]
        labels: Option<PathBuf>,
    },
    /// Render a static SVG force-directed graph image.
    Svg {
        #[arg(long)]
        graph: Option<PathBuf>,
        /// Path to community labels JSON (default: `<graph_dir>/.graphify_labels.json`).
        #[arg(long)]
        labels: Option<PathBuf>,
    },
    /// Export graph in `GraphML` format for Gephi or yEd.
    Graphml {
        #[arg(long)]
        graph: Option<PathBuf>,
    },
    /// Export graph as Cypher statements or push directly to a Neo4j instance.
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
    /// Export Cypher statements or push directly to a `FalkorDB` instance.
    ///
    /// `FalkorDB` is `OpenCypher`-compatible; without `--push` this writes the same
    /// `cypher.txt` as the `neo4j` export.
    Falkordb {
        #[arg(long)]
        graph: Option<PathBuf>,
        /// Push directly to a live `FalkorDB` instance (e.g. `falkordb://localhost:6379`).
        #[arg(long)]
        push: Option<String>,
        /// `FalkorDB` username for `--push` (auth is optional; anonymous by default).
        #[arg(long)]
        user: Option<String>,
        /// `FalkorDB` password for `--push` (or set `FALKORDB_PASSWORD`).
        #[arg(long)]
        password: Option<String>,
    },
}

/// Install/uninstall subcommand shared by every per-platform command group.
#[derive(Debug, Subcommand)]
pub(crate) enum PlatformCmd {
    /// Install graphify integration for this platform.
    Install {
        /// Install under the current project instead of the home directory.
        #[arg(long)]
        project: bool,
    },
    /// Uninstall graphify integration for this platform.
    Uninstall {
        /// Uninstall only the project-scoped install (leave user-global
        /// untouched).
        #[arg(long)]
        project: bool,
    },
}

/// Subcommands for the `diagnose` command group.
#[derive(Debug, Subcommand)]
pub(crate) enum DiagnoseCmd {
    /// `MultiDiGraph` edge-collapse risk report.
    Multigraph {
        /// Path to graph.json (default `graphify-out/graph.json`).
        #[arg(long)]
        graph: Option<PathBuf>,
        /// Emit a JSON envelope instead of the line-by-line text report.
        #[arg(long)]
        json: bool,
        /// Maximum number of edge-group examples to include.
        #[arg(long = "max-examples", default_value_t = 5)]
        max_examples: usize,
        /// Force directed analysis (overrides the JSON's `directed` flag).
        #[arg(long, conflicts_with = "undirected")]
        directed: bool,
        /// Force undirected analysis (overrides the JSON's `directed` flag).
        #[arg(long, conflicts_with = "directed")]
        undirected: bool,
        /// Path to the Python extractor file scanned for `seen_*` producer
        /// suppression sites. Optional — when omitted the producer-suppression
        /// section is empty.
        #[arg(long = "extract-path")]
        extract_path: Option<PathBuf>,
    },
}
