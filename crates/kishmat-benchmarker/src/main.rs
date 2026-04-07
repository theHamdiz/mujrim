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
use eval::nnue::load_network;
use kishmat_protocols::ProtocolKind;

use kishmat_benchmarker::{
    engine_info::{self, NnueInfo},
    external::{self, ExternalBenchConfig},
    hardware::HardwareInfo,
    internal::{self, InternalBenchConfig},
    iterate::{self, EloIterateConfig},
    suite::{self, format_nps},
};

#[derive(Parser)]
#[command(
    name = "kishmat-benchmarker",
    about = "Benchmark suite for KishMat and external UCI/XBoard chess engines",
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
        #[arg(short, long, default_value_t = 20)]
        depth: i32,

        /// Number of search threads.
        #[arg(short = 't', long)]
        threads: Option<usize>,

        /// Hash table size in MB.
        #[arg(long, default_value_t = 128)]
        hash: usize,

        /// Per-position time limit in seconds.
        #[arg(long, default_value_t = 30)]
        time: u64,

        /// Path to a custom FEN file (default: built-in BK suite).
        #[arg(short, long)]
        positions: Option<PathBuf>,

        /// Enable TUI mode with live progress display.
        #[arg(long)]
        tui: bool,

        /// Runtime NNUE preset (`auto`, `akimbo`, `stockfish`).
        #[arg(
            long,
            default_value = "auto",
            value_parser = ["auto", "akimbo", "stockfish"]
        )]
        eval_preset: String,

        /// Optional runtime network file path (same semantics as UCI EvalFile).
        #[arg(long)]
        eval_file: Option<PathBuf>,

        /// Suppress per-position lines and auto-detect stderr (machine-friendly).
        #[arg(long, default_value_t = false)]
        quiet: bool,

        /// Print one JSON object with summary + NPS (implies `--quiet` for the bench run).
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// Repeat BK benchmarks until the CCRL 40/15 proxy reaches `--target` (or limits stop).
    Iterate {
        /// Stop when BK accuracy maps to at least this **approx. CCRL 40/15** (proxy cap ~2750).
        #[arg(long, default_value_t = 2750)]
        target_elo: i32,

        /// Maximum benchmark rounds (each round is one full BK pass, optional `--between` first).
        #[arg(long, default_value_t = 100)]
        max_rounds: u32,

        /// Exit after this many rounds with no improvement in the CCRL proxy.
        #[arg(long, default_value_t = 20)]
        stagnation_limit: u32,

        /// Shell command run before each round (e.g. `cargo build --release -p foo`); skipped before round 1.
        #[arg(long)]
        between: Option<String>,

        /// Print one JSON line per round; final line is the outcome object when not using `--json-progress-only`.
        #[arg(long, default_value_t = false)]
        json: bool,

        /// Only NDJSON round rows + stagnation/exit events (no final wrapper object).
        #[arg(long, default_value_t = false)]
        json_progress_only: bool,

        /// Skip the extra 5s startpos NPS sample each round.
        #[arg(long, default_value_t = false)]
        no_nps: bool,

        /// Search depth.
        #[arg(short, long, default_value_t = 20)]
        depth: i32,

        /// Number of search threads.
        #[arg(short = 't', long)]
        threads: Option<usize>,

        /// Hash table size in MB.
        #[arg(long, default_value_t = 128)]
        hash: usize,

        /// Per-position time limit in seconds.
        #[arg(long, default_value_t = 30)]
        time: u64,

        /// Path to a custom FEN file (default: built-in BK suite).
        #[arg(short, long)]
        positions: Option<PathBuf>,

        /// Runtime NNUE preset (`auto`, `akimbo`, `stockfish`).
        #[arg(
            long,
            default_value = "auto",
            value_parser = ["auto", "akimbo", "stockfish"]
        )]
        eval_preset: String,

        /// Optional runtime network file path (same semantics as UCI EvalFile).
        #[arg(long)]
        eval_file: Option<PathBuf>,
    },

    /// Benchmark an external UCI engine binary.
    Uci {
        /// Path to the UCI engine binary.
        engine: PathBuf,

        /// Extra CLI arg passed to the engine process (repeatable).
        #[arg(long = "arg")]
        engine_args: Vec<String>,

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
        #[arg(long, default_value_t = 30)]
        time: u64,

        /// Path to a custom FEN file (default: built-in BK suite).
        #[arg(short, long)]
        positions: Option<PathBuf>,
    },

    /// Benchmark an external XBoard/CECP engine binary.
    Xboard {
        /// Path to the XBoard engine binary.
        engine: PathBuf,

        /// Extra CLI arg passed to the engine process (repeatable).
        #[arg(long = "arg")]
        engine_args: Vec<String>,

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
        #[arg(long, default_value_t = 30)]
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
            eval_preset,
            eval_file,
            quiet,
            json,
        } => run_bench(
            depth,
            threads,
            hash,
            time,
            positions,
            tui,
            eval_preset,
            eval_file,
            quiet,
            json,
        ),
        Commands::Iterate {
            target_elo,
            max_rounds,
            stagnation_limit,
            between,
            json,
            json_progress_only,
            no_nps,
            depth,
            threads,
            hash,
            time,
            positions,
            eval_preset,
            eval_file,
        } => run_iterate(
            target_elo,
            max_rounds,
            stagnation_limit,
            between,
            json,
            json_progress_only,
            no_nps,
            depth,
            threads,
            hash,
            time,
            positions,
            eval_preset,
            eval_file,
        ),
        Commands::Uci {
            engine,
            engine_args,
            depth,
            hash,
            threads,
            time,
            positions,
        } => run_external(
            engine,
            engine_args,
            ProtocolKind::Uci,
            depth,
            hash,
            threads,
            time,
            positions,
        ),
        Commands::Xboard {
            engine,
            engine_args,
            depth,
            hash,
            threads,
            time,
            positions,
        } => run_external(
            engine,
            engine_args,
            ProtocolKind::Xboard,
            depth,
            hash,
            threads,
            time,
            positions,
        ),
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
    eval_preset: String,
    eval_file: Option<PathBuf>,
    quiet: bool,
    json_output: bool,
) {
    let bench_quiet = quiet || json_output;
    let hw = HardwareInfo::detect();
    let nnue = if let Some(path) = &eval_file {
        match load_network(path) {
            Ok(network) => NnueInfo::from_runtime(network.info()),
            Err(e) => {
                eprintln!(
                    "info string EvalFile load failed for '{}': {e} (showing embedded info)",
                    path.display()
                );
                NnueInfo::detect()
            }
        }
    } else {
        NnueInfo::detect()
    };
    let params = search::search_params::SearchParams::default();

    let thread_count = threads.unwrap_or(hw.bench_threads());

    if !bench_quiet {
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
        println!("    EvalPreset: {eval_preset}");
        if let Some(path) = &eval_file {
            println!("    EvalFile:   {}", path.display());
        }
        println!();
    }

    // Load positions
    let suite = if let Some(path) = positions {
        match suite::load_custom_positions(&path) {
            Ok(s) => {
                if !bench_quiet {
                    println!("  Loaded {} positions from {}", s.len(), path.display());
                }
                s
            }
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    } else {
        if !bench_quiet {
            println!("  Using Bratko-Kopec test suite (24 positions)");
        }
        suite::bk_suite()
    };
    if !bench_quiet {
        println!();

        // Show enabled techniques summary
        let techs = engine_info::detect_techniques(&params);
        let enabled_count = techs.iter().filter(|t| t.enabled).count();
        println!("  {enabled_count} search techniques enabled");
        println!();
    }

    // Run
    let config = InternalBenchConfig {
        depth,
        threads: thread_count,
        hash_mb: hash,
        time_per_position: Duration::from_secs(time),
        suite_name: "BK".into(),
        eval_preset: eval_preset.clone(),
        eval_file: eval_file.clone(),
        quiet: bench_quiet,
    };

    #[cfg(feature = "tui")]
    if _tui {
        let mut header_lines = vec![format!("NNUE: {} ({})", nnue.name, nnue.architecture)];
        header_lines.extend(hw.display_lines());
        header_lines.push(format!(
            "Depth: {depth}  Threads: {thread_count}  Hash: {hash}MB"
        ));

        let mut tui_config = config.clone();
        tui_config.quiet = false;

        match kishmat_benchmarker::tui::BenchTui::new(suite.len(), header_lines) {
            Ok(mut tui_state) => {
                let summary = internal::run_internal_bench(
                    &suite,
                    &tui_config,
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
    if !bench_quiet {
        println!();
    }

    let nps_5s = internal::measure_startpos_nps(
        thread_count,
        hash,
        &eval_preset,
        eval_file.as_deref(),
        bench_quiet,
    );
    if json_output {
        println!(
            "{}",
            serde_json::json!({
                "summary": summary.to_json_value(),
                "nps_startpos_5s": nps_5s,
            })
        );
    } else {
        println!("{summary}");
        println!("  NPS (5s startpos): {}", format_nps(nps_5s));
    }
}

fn run_iterate(
    target_elo: i32,
    max_rounds: u32,
    stagnation_limit: u32,
    between: Option<String>,
    json: bool,
    json_progress_only: bool,
    no_nps: bool,
    depth: i32,
    threads: Option<usize>,
    hash: usize,
    time: u64,
    positions: Option<PathBuf>,
    eval_preset: String,
    eval_file: Option<PathBuf>,
) {
    let hw = HardwareInfo::detect();
    let thread_count = threads.unwrap_or(hw.bench_threads());

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

    let json_progress = json || json_progress_only;
    let print_final_json = json && !json_progress_only;

    let iter_cfg = EloIterateConfig {
        target_elo,
        max_rounds,
        stagnation_limit,
        between_shell: between,
        json_progress,
        quiet: json_progress,
        bench: InternalBenchConfig {
            depth,
            threads: thread_count,
            hash_mb: hash,
            time_per_position: Duration::from_secs(time),
            suite_name: "BK".into(),
            eval_preset,
            eval_file,
            quiet: true,
        },
        measure_nps: !no_nps,
    };

    if !json_progress {
        eprintln!(
            "CCRL 40/15 proxy iterate: target ~{target_elo}  rounds≤{max_rounds}  stagnation≤{stagnation_limit}  threads={thread_count}"
        );
    }

    let outcome = iterate::run_elo_iterate(&suite, &iter_cfg);

    if print_final_json {
        println!("{}", outcome.to_json_value());
    } else if !json_progress && !outcome.success {
        eprintln!(
            "Stopped: {} (best approx. CCRL 40/15 {} on round {}, final {})",
            outcome.reason,
            outcome.best_elo,
            outcome.best_round,
            outcome.final_round.summary.approx_ccrl_40_15
        );
    } else if !json_progress && outcome.success {
        eprintln!(
            "Target reached: approx. CCRL 40/15 ~{} (round {})",
            outcome.final_round.summary.approx_ccrl_40_15, outcome.rounds_run
        );
    }

    if !outcome.success {
        std::process::exit(1);
    }
}

fn run_external(
    engine: PathBuf,
    engine_args: Vec<String>,
    protocol: ProtocolKind,
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

    println!("Benchmarking {} engine: {}", protocol, engine.display());
    if !engine_args.is_empty() {
        println!("Engine args: {}", engine_args.join(" "));
    }
    println!("Depth: {depth}  Hash: {hash}MB  Threads: {threads}  Time/pos: {time}s");
    println!();

    let config = ExternalBenchConfig {
        engine_path: engine,
        engine_args,
        protocol,
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
