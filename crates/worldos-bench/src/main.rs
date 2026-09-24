//! `worldos-bench` — run a WorldBench task corpus or a performance
//! baseline, writing evidence-rich JSON reports.
//!
//!   worldos-bench --tasks bench/tasks --out report.json
//!   worldos-bench --tasks bench/tasks            (report to stdout)
//!   worldos-bench perf --sizes 100,10000 --out perf.json

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "worldos-bench", about = "WorldBench deterministic task runner")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// Directory of *.yaml task files.
    #[arg(long, default_value = "bench/tasks")]
    tasks: PathBuf,
    /// Write the JSON report here instead of stdout.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Exit non-zero when any task fails.
    #[arg(long)]
    strict: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Performance baseline: create/save/reopen/query/undo at N objects.
    Perf {
        /// Comma-separated object counts, e.g. `100,10000`.
        #[arg(long, default_value = "100,10000")]
        sizes: String,
        /// Write the JSON report here instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn write_or_print(json: &str, out: Option<&PathBuf>, what: &str) {
    match out {
        Some(p) => {
            if let Err(e) = std::fs::write(p, json) {
                eprintln!("worldos-bench: cannot write {p:?}: {e}");
                std::process::exit(2);
            }
            eprintln!("worldos-bench: {what} — report at {}", p.display());
        }
        None => println!("{json}"),
    }
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Some(Cmd::Perf { sizes, out }) => {
            let sizes: Vec<usize> = match sizes
                .split(',')
                .map(|s| s.trim().parse::<usize>())
                .collect::<Result<_, _>>()
            {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("worldos-bench: bad --sizes: {e}");
                    std::process::exit(2);
                }
            };
            let report = match worldos_bench::run_perf(&sizes) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("worldos-bench: {e}");
                    std::process::exit(2);
                }
            };
            let json = serde_json::to_string_pretty(&report).expect("serialize report");
            write_or_print(&json, out.as_ref(), "perf baseline");
        }
        None => {
            let report = match worldos_bench::run(&cli.tasks) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("worldos-bench: {e}");
                    std::process::exit(2);
                }
            };
            let json = serde_json::to_string_pretty(&report).expect("serialize report");
            match &cli.out {
                Some(p) => {
                    if let Err(e) = std::fs::write(p, &json) {
                        eprintln!("worldos-bench: cannot write {p:?}: {e}");
                        std::process::exit(2);
                    }
                    eprintln!(
                        "worldos-bench: {} task(s), {} passed — report at {}",
                        report.summary["total"],
                        report.summary["passed"],
                        p.display()
                    );
                }
                None => println!("{json}"),
            }
            if cli.strict && report.summary["failed"].as_u64().unwrap_or(0) > 0 {
                std::process::exit(1);
            }
        }
    }
}
