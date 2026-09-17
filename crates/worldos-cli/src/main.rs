//! `worldos` — the WorldOS command line interface.
//!
//! Every subcommand drives the same `Engine` the desktop app uses; the
//! CLI is just one more interface onto the governed command layer.

mod cmd;
mod out;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "worldos",
    version,
    about = "WorldOS — the open AI-native operating system for creating the digital and physical world"
)]
struct Cli {
    /// Emit machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a new project file.
    New {
        name: String,
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Open a project and print a summary.
    Open { file: PathBuf },
    /// Inspect the project or a single object (--object name/id).
    Inspect {
        file: PathBuf,
        #[arg(short, long)]
        object: Option<String>,
    },
    /// Print the project graph (objects + relations).
    Graph { file: PathBuf },
    /// Run all validators.
    Validate { file: PathBuf },
    /// Execute one command: `worldos command p.worldos object.create '{"type":"core:note","name":"x"}'`
    Command {
        file: PathBuf,
        command: String,
        input: Option<String>,
    },
    /// Run a JSON list of commands inside ONE transaction.
    Batch { file: PathBuf, script: PathBuf },
    /// List command schemas.
    Commands { file: PathBuf },
    /// List capability descriptors.
    Capabilities { file: PathBuf },
    /// Show transaction history.
    History {
        file: PathBuf,
        #[arg(short, long, default_value = "25")]
        limit: usize,
    },
    /// Undo the latest transaction.
    Undo { file: PathBuf },
    /// Redo the latest undone transaction.
    Redo { file: PathBuf },
    /// Semantic diff between two project files.
    Diff { a: PathBuf, b: PathBuf },
    /// Run the agent on a goal.
    Agent {
        file: PathBuf,
        goal: String,
        #[arg(long, default_value = "assistant")]
        agent: String,
    },
    /// Export the project model to JSON.
    Export { file: PathBuf, out: PathBuf },
    /// Start the MCP server (stdio) bound to a project.
    Mcp { file: PathBuf },
    /// Raw JSON-RPC stdio endpoint bound to a project (used by SDKs).
    Rpc { file: PathBuf },
    /// WebSocket JSON-RPC server for SDK/desktop clients.
    Serve {
        file: PathBuf,
        #[arg(short, long, default_value = "7799")]
        port: u16,
    },
    /// Environment diagnostics.
    Doctor,
    /// List discovered plugins, or run one inside a project session.
    Plugin {
        #[command(subcommand)]
        sub: PluginCmd,
    },
    /// Reference plugin: speaks the JSON-RPC plugin protocol on stdio.
    /// Used by tests and as a documented example for plugin authors —
    /// `worldos plugin run <file> -- <path-to-worldos> plugin-shim`.
    #[command(hide = true)]
    PluginShim {
        /// Exit non-zero mid-session to exercise rollback.
        #[arg(long)]
        fail: bool,
    },
    /// Print version.
    Version,
}

#[derive(Subcommand)]
enum PluginCmd {
    /// Discover `worldos-plugin-*` executables in plugin dirs.
    List {
        /// Project file — its sibling `plugins/` dir is searched too.
        file: Option<PathBuf>,
    },
    /// Run a plugin: subprocess gets a JSON-RPC channel into the project;
    /// all its commands commit as one `plugin:<name>` transaction.
    Run {
        /// Project file to open.
        file: PathBuf,
        /// Plugin name (`foo` → `worldos-plugin-foo`) or path.
        plugin: String,
        /// Arguments passed to the plugin.
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
}

fn main() {
    let cli = Cli::parse();
    init_tracing();
    let code = match cmd::run(cli.cmd, cli.json) {
        Ok(()) => 0,
        Err(e) => {
            if cli.json {
                println!("{}", serde_json::json!({"error": e.to_string()}));
            } else {
                eprintln!("error: {e}");
            }
            1
        }
    };
    std::process::exit(code);
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("WORLDOS_LOG").unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
