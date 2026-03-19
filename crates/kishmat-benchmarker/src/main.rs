//! KishMat Benchmarker — CLI entry point.
//!
//! # Usage
//!
//! ```bash
//! # Internal BK benchmark (default)
//! kishmat-benchmarker bench
//!
//! # With options
//! kishmat-benchmarker bench --depth 18 --threads 8 --hash 256
//!
//! # Custom FEN file
//! kishmat-benchmarker bench --positions custom.fen
//!
//! # TUI mode
//! kishmat-benchmarker bench --tui
//!
//! # External UCI engine
//! kishmat-benchmarker uci ./stockfish --depth 16
//!
//! # Show engine info
//! kishmat-benchmarker info
//! ```

use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};

use kishmat_benchmarker::{
    engine_info::{self, NnueInfo},
    external::{self, ExternalBenchConfig},
    hardware::HardwareInfo,
    internal::{self, InternalBenchConfig},
    suite::{self, format_nps},
};

#[derive(Parser)]
#[command(
    name = "kishmat-benchmarker",
    about = "Benchmark suite for KishMat and external UCI chess engines",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run internal BK benchmark using KishMat's search engine.
    Bench {
        /// Search depth.
        #[arg(short, long, default_value_t = 16)]
        depth: i32,

        /// Number of search threads.
        #[arg(short = 't', long)]
        threads: Option<usize>,

        /// Hash table size in MB.
        #[arg(long, default_value_t = 128)]
        hash: usize,

        /// Per-position time limit in seconds.
        #[arg(long, default_value_t = 120)]
        time: u64,

        /// Path to a custom FEN file (default: built-in BK suite).
        #[arg(short, long)]
        positions: Option<PathBuf>,

        /// Enable TUI mode with live progress display.
        #[arg(long)]
        tui: bool,
    },

    /// Benchmark an external UCI engine binary.
    Uci {
        /// Path to the UCI engine binary.
        engine: PathBuf,

        /// Search depth.
        #[arg(short, long, default_value_t = 16)]
        depth: i32,

        /// Hash table size in MB.
        #[arg(long, default_value_t = 128)]
        hash: usize,

        /// Number of engine threads.
        #[arg(short = 't', long, default_value_t = 1)]
        threads: usize,

        /// Per-position time limit in seconds.
        #[arg(long, default_value_t = 120)]
        time: u64,

        /// Path to a custom FEN file (default: built-in BK suite).
        #[arg(short, long)]
        positions: Option<PathBuf>,
    },

    /// Display NNUE network and search technique information.
    Info,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Bench {
            depth,
            threads,
            hash,
            time,
            positions,
            tui,
        } => run_bench(depth, threads, hash, time, positions, tui),
        Commands::Uci {
            engine,
            depth,
            hash,
            threads,
            time,
            positions,
        } => run_uci(engine, depth, hash, threads, time, positions),
        Commands::Info => run_info(),
    }
}

fn run_info() {
    let hw = HardwareInfo::detect();
    let nnue = NnueInfo::detect();
    let params = search::search_params::SearchParams::default();

    println!("╔══════════════════════════════════════════════╗");
    println!("║           KISHMAT ENGINE INFO               ║");
    println!("╠══════════════════════════════════════════════╣");
    println!();
    println!("  ── NNUE Network ──");
    for line in nnue.display_lines() {
        println!("{line}");
    }
    println!();
    println!("  ── Hardware ──");
    for line in hw.display_lines() {
        println!("{line}");
    }
    println!();
    println!("  ── Search Techniques ──");
    print!("{}", engine_info::format_techniques(&params));
    println!();
    println!("╚══════════════════════════════════════════════╝");
}

fn run_bench(
    depth: i32,
    threads: Option<usize>,
    hash: usize,
    time: u64,
    positions: Option<PathBuf>,
    _tui: bool,
) {
    let hw = HardwareInfo::detect();
    let nnue = NnueInfo::detect();
    let params = search::search_params::SearchParams::default();

    let thread_count = threads.unwrap_or(hw.bench_threads());

    // Print header
    println!("╔══════════════════════════════════════════════╗");
    println!("║           KISHMAT BENCHMARKER               ║");
    println!("╠══════════════════════════════════════════════╣");
    println!();
    println!("  ── NNUE ──");
    for line in nnue.display_lines() {
        println!("{line}");
    }
    println!();
    println!("  ── Hardware ──");
    for line in hw.display_lines() {
        println!("{line}");
    }
    println!();
    println!("  ── Config ──");
    println!("    Depth:      {depth}");
    println!("    Threads:    {thread_count}");
    println!("    Hash:       {hash} MB");
    println!("    Time/pos:   {time}s");
    println!();

    // Load positions
    let suite = if let Some(path) = positions {
        match suite::load_custom_positions(&path) {
            Ok(s) => {
                println!("  Loaded {} positions from {}", s.len(), path.display());
                s
            }
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    } else {
        println!("  Using Bratko-Kopec test suite (24 positions)");
        suite::bk_suite()
    };
    println!();

    // Show enabled techniques summary
    let techs = engine_info::detect_techniques(&params);
    let enabled_count = techs.iter().filter(|t| t.enabled).count();
    println!("  {enabled_count} search techniques enabled");
    println!();

    // Run
    let config = InternalBenchConfig {
        depth,
        threads: thread_count,
        hash_mb: hash,
        time_per_position: Duration::from_secs(time),
        suite_name: "BK".into(),
    };

    #[cfg(feature = "tui")]
    if _tui {
        let mut header_lines = vec![format!("NNUE: {} ({})", nnue.name, nnue.architecture)];
        header_lines.extend(hw.display_lines());
        header_lines.push(format!(
            "Depth: {depth}  Threads: {thread_count}  Hash: {hash}MB"
        ));

        match kishmat_benchmarker::tui::BenchTui::new(suite.len(), header_lines) {
            Ok(mut tui_state) => {
                let summary = internal::run_internal_bench(
                    &suite,
                    &config,
                    Some(Box::new(move |_i, _total, _result| {
                        // TUI callback would update here, but we need mutable access
                        // For simplicity, we'll use the non-TUI path and show TUI summary
                    })),
                );
                tui_state.show_summary(&summary);
                return;
            }
            Err(e) => {
                eprintln!("TUI init failed, falling back to plain output: {e}");
            }
        }
    }

    let summary = internal::run_internal_bench(&suite, &config, None);
    println!();

    // Print NPS from startpos
    let nps_5s = measure_nps(thread_count, hash);
    println!("{summary}");
    println!("  NPS (5s startpos): {}", format_nps(nps_5s));
}

fn run_uci(
    engine: PathBuf,
    depth: i32,
    hash: usize,
    threads: usize,
    time: u64,
    positions: Option<PathBuf>,
) {
    let suite = if let Some(path) = positions {
        match suite::load_custom_positions(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    } else {
        suite::bk_suite()
    };

    println!("Benchmarking UCI engine: {}", engine.display());
    println!("Depth: {depth}  Hash: {hash}MB  Threads: {threads}  Time/pos: {time}s");
    println!();

    let config = ExternalBenchConfig {
        engine_path: engine,
        depth,
        hash_mb: hash,
        threads,
        time_per_position: Duration::from_secs(time),
    };

    match external::run_external_bench(&suite, &config) {
        Ok(summary) => {
            println!();
            println!("{summary}");
        }
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}

/// Measure NPS from startpos in 5 seconds.
fn measure_nps(threads: usize, hash_mb: usize) -> u64 {
    use search::engine::SearchEngine;
    use types::Board;

    let mut engine = SearchEngine::new(hash_mb, threads);
    let mut board = Board::new();
    let result = engine.search_time(&mut board, Duration::from_secs(5), 64);
    if result.elapsed.as_millis() > 0 {
        result.nodes * 1000 / result.elapsed.as_millis() as u64
    } else {
        result.nodes
    }
}
