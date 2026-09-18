//! `worldos-bench` — run a WorldBench task corpus and write an
//! evidence-rich JSON report.
//!
//!   worldos-bench --tasks bench/tasks --out report.json
//!   worldos-bench --tasks bench/tasks            (report to stdout)

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(name = "worldos-bench", about = "WorldBench deterministic task runner")]
struct Cli {
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

fn main() {
    let cli = Cli::parse();
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
