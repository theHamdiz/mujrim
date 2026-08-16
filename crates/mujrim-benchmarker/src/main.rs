#![cfg_attr(all(target_os = "windows", not(test)), windows_subsystem = "windows")]

//! Mujrim Benchmarker — CLI entry point.
//!
//! # Usage
//!
//! ```bash
//! # Internal BK benchmark (default)
//! mujrim-benchmarker bench
//!
//! # With options
//! mujrim-benchmarker bench --depth 18 --threads 8 --hash 256
//!
//! # Custom FEN file
//! mujrim-benchmarker bench --positions custom.fen
//!
//! # TUI mode
//! mujrim-benchmarker bench --tui
//!
//! # External UCI engine
//! mujrim-benchmarker uci ./stockfish --depth 16
//!
//! # Show engine info
//! mujrim-benchmarker info
//! ```

use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use eval::nnue::{NnueNetworkSource, load_network, load_network_for_preset};
use mujrim_protocols::ProtocolKind;

use mujrim_benchmarker::{
    compare,
    engine_info::{self, NnueInfo},
    external::{self, ExternalBenchConfig},
    hardware::HardwareInfo,
    internal::{self, InternalBenchConfig},
    iterate::{self, EloIterateConfig},
    nnue_bench::{self, NnueBenchConfig},
    replay,
    strength::{
        EngineSpec, MatchConfig, TournamentConfig, TournamentEngine,
        openings::load_openings,
        run_match, run_tournament,
        stats::{Sprt, SprtDecision},
    },
    suite::{self, format_nps},
};

#[derive(Parser)]
#[command(
    name = "mujrim-benchmarker",
    about = "Benchmark suite for Mujrim and external UCI/XBoard chess engines",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

fn parse_uci_option(input: &str) -> Result<(String, String), String> {
    let (name, value) = input
        .split_once('=')
        .ok_or_else(|| "expected NAME=VALUE".to_string())?;
    let name = name.trim();
    let value = value.trim();
    if name.is_empty() {
        return Err("option name cannot be empty".to_string());
    }
    if name.chars().any(char::is_control) || value.chars().any(char::is_control) {
        return Err("option names and values cannot contain control characters".to_string());
    }
    Ok((name.to_string(), value.to_string()))
}

fn parse_engine_rating(input: &str) -> Result<(String, f64), String> {
    let (name, rating) = input
        .split_once('=')
        .ok_or_else(|| "expected ENGINE=ELO".to_owned())?;
    let name = name.trim();
    if name.is_empty() {
        return Err("engine name cannot be empty".to_owned());
    }
    let rating = rating
        .trim()
        .parse::<f64>()
        .map_err(|error| format!("invalid Elo rating: {error}"))?;
    if !rating.is_finite() || !(0.0..=5_000.0).contains(&rating) {
        return Err("Elo must be a finite value from 0 to 5000".to_owned());
    }
    Ok((name.to_owned(), rating))
}

#[derive(Subcommand)]
enum Commands {
    /// Run internal BK benchmark using Mujrim's search engine.
    Bench {
        /// Search depth.
        #[arg(short, long, default_value_t = 20)]
        depth: i32,

        /// Number of search threads.
        #[arg(short = 't', long)]
        threads: Option<usize>,

        /// Hash table size in MB.
        #[arg(long, default_value_t = 256)]
        hash: usize,

        /// Per-position time limit in seconds.
        #[arg(long, default_value_t = 90)]
        time: u64,

        /// Path to a custom FEN file (default: built-in BK suite).
        #[arg(short, long)]
        positions: Option<PathBuf>,

        /// Enable TUI mode with live progress display.
        #[arg(long)]
        tui: bool,

        /// Runtime eval adapter (`auto`, `akimbo`, `stockfish`, `reckless`, `viridithas`, `obsidian`, `plentychess`, `ateed`, `lc0`, `mujrim-hce`).
        #[arg(
            long,
            default_value = "auto",
            value_parser = ["auto", "akimbo", "stockfish", "reckless", "viridithas", "obsidian", "plentychess", "ateed", "lc0", "mujrim-hce"]
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

    /// Measure the embedded NNUE evaluator without search or subprocess noise.
    Nnue {
        /// Timed evaluations for each workload.
        #[arg(long, default_value_t = 100_000)]
        iterations: u64,

        /// Untimed evaluations used to warm code and network pages.
        #[arg(long, default_value_t = 1_000)]
        warmup: u64,

        /// Optional network file; omit to benchmark the official embedded network.
        #[arg(long)]
        eval_file: Option<PathBuf>,

        /// Emit one machine-readable JSON object.
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// Isolated adapter eval card and V2-shaped adapter-vs-native matches.
    Gauntlet {
        /// Timed evaluations for each adapter preset that has a network.
        #[arg(long, default_value_t = 8_000)]
        iterations: u64,

        /// Untimed evaluations used to warm code and network pages.
        #[arg(long, default_value_t = 200)]
        warmup: u64,

        /// Optional single preset (`akimbo`, `viridithas`, …). Omit to try every V2 pair.
        #[arg(long)]
        preset: Option<String>,

        /// Play nodes-equal and/or 3+2 cards against the native binary.
        #[arg(long, default_value_t = false)]
        play: bool,

        /// Color-swapped opening pairs per played card.
        #[arg(long, default_value_t = 4)]
        pairs: usize,

        /// Node budget for the nodes-equal card.
        #[arg(long, default_value_t = 20_000)]
        nodes: u64,

        /// Skip the nodes-equal card when `--play` is set.
        #[arg(long, default_value_t = false)]
        skip_nodes: bool,

        /// Skip the V2 3+2 clock card when `--play` is set.
        #[arg(long, default_value_t = false)]
        skip_clock: bool,

        /// Workspace root used to resolve `dist/<os-arch>/engines` binaries.
        #[arg(long)]
        root: Option<PathBuf>,

        /// Emit one machine-readable JSON object.
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// Repeat BK benchmarks until the CCRL 40/15 proxy reaches `--target`, `--min-bk` hits, or limits stop.
    Iterate {
        /// Stop when BK accuracy maps to at least this **approx. CCRL 40/15** (100% BK suite → 3500 on this proxy).
        #[arg(long, default_value_t = 3500)]
        target_elo: i32,

        /// Stop when at least this many positions match (24-pos BK suite). Use 0 to disable.
        #[arg(long, default_value_t = 20)]
        min_bk: usize,

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
        #[arg(long, default_value_t = 256)]
        hash: usize,

        /// Per-position time limit in seconds.
        #[arg(long, default_value_t = 90)]
        time: u64,

        /// Path to a custom FEN file (default: built-in BK suite).
        #[arg(short, long)]
        positions: Option<PathBuf>,

        /// Runtime eval adapter (`auto`, `akimbo`, `stockfish`, `reckless`, `viridithas`, `obsidian`, `plentychess`, `ateed`, `lc0`, `mujrim-hce`).
        #[arg(
            long,
            default_value = "auto",
            value_parser = ["auto", "akimbo", "stockfish", "reckless", "viridithas", "obsidian", "plentychess", "ateed", "lc0", "mujrim-hce"]
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

        /// UCI option as NAME=VALUE (repeatable).
        #[arg(long = "option", value_parser = parse_uci_option)]
        uci_options: Vec<(String, String)>,

        /// Search depth.
        #[arg(short, long, default_value_t = 16)]
        depth: i32,

        /// Fixed node budget per position; overrides depth and time limits.
        #[arg(long)]
        nodes: Option<u64>,

        /// Hash table size in MB.
        #[arg(long, default_value_t = 64)]
        hash: usize,

        /// Number of engine threads.
        #[arg(short = 't', long, default_value_t = 1)]
        threads: usize,

        /// Maximum external-engine working set in MiB.
        #[arg(long, default_value_t = 256)]
        memory_limit: usize,

        /// Optional per-position time limit in seconds; overrides depth.
        #[arg(long)]
        time: Option<u64>,

        /// Path to a custom FEN file (default: built-in BK suite).
        #[arg(short, long)]
        positions: Option<PathBuf>,

        /// Emit one machine-readable JSON object.
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// Compare candidate and reference UCI binaries in alternating fixed-node runs.
    Compare {
        /// Candidate engine binary.
        candidate: PathBuf,

        /// Accepted reference engine binary.
        reference: PathBuf,

        /// Extra argument passed to the candidate (repeatable).
        #[arg(long = "candidate-arg")]
        candidate_args: Vec<String>,

        /// UCI option for the candidate as NAME=VALUE (repeatable).
        #[arg(long = "candidate-option", value_parser = parse_uci_option)]
        candidate_options: Vec<(String, String)>,

        /// Extra argument passed to the reference (repeatable).
        #[arg(long = "reference-arg")]
        reference_args: Vec<String>,

        /// UCI option for the reference as NAME=VALUE (repeatable).
        #[arg(long = "reference-option", value_parser = parse_uci_option)]
        reference_options: Vec<(String, String)>,

        /// Alternating run count for each binary. Use one for rapid triage.
        #[arg(long, default_value_t = 2)]
        rounds: usize,

        /// Fixed node budget per position.
        #[arg(long, default_value_t = 250_000)]
        nodes: u64,

        /// Hash table size for the active engine process in MB.
        #[arg(long, default_value_t = 128)]
        hash: usize,

        /// Search threads for the active engine process.
        #[arg(short = 't', long, default_value_t = 1)]
        threads: usize,

        /// Hard working-set limit for the active engine process in MiB.
        #[arg(long, default_value_t = 256)]
        memory_limit: usize,

        /// Minimum speed gain needed to accept behaviorally equivalent output.
        #[arg(long, default_value_t = 2.0)]
        minimum_speedup: f64,

        /// Path to a custom FEN file (default: built-in BK suite).
        #[arg(short, long)]
        positions: Option<PathBuf>,

        /// Emit one machine-readable JSON object.
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// Compare two engines at an exact FEN and UCI move history.
    Replay {
        /// Candidate engine binary.
        candidate: PathBuf,

        /// Reference engine binary.
        reference: PathBuf,

        /// Initial FEN before applying `--move` values.
        #[arg(long, default_value = mujrim_benchmarker::strength::openings::START_FEN)]
        fen: String,

        /// UCI move from the game history (repeat in played order).
        #[arg(long = "move")]
        moves: Vec<String>,

        /// Extra argument passed to the candidate (repeatable).
        #[arg(long = "candidate-arg")]
        candidate_args: Vec<String>,

        /// UCI option for the candidate as NAME=VALUE (repeatable).
        #[arg(long = "candidate-option", value_parser = parse_uci_option)]
        candidate_options: Vec<(String, String)>,

        /// Extra argument passed to the reference (repeatable).
        #[arg(long = "reference-arg")]
        reference_args: Vec<String>,

        /// UCI option for the reference as NAME=VALUE (repeatable).
        #[arg(long = "reference-option", value_parser = parse_uci_option)]
        reference_options: Vec<(String, String)>,

        /// Fixed search budget for each engine.
        #[arg(long, default_value_t = 250_000)]
        nodes: u64,

        /// Hash table size for the active engine in MiB.
        #[arg(long, default_value_t = 128)]
        hash: usize,

        /// Search threads for the active engine.
        #[arg(short = 't', long, default_value_t = 1)]
        threads: usize,

        /// Hard working-set limit for the active engine in MiB.
        #[arg(long, default_value_t = 256)]
        memory_limit: usize,

        /// Emit one machine-readable JSON object.
        #[arg(long, default_value_t = false)]
        json: bool,
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

        /// Maximum external-engine working set in MiB.
        #[arg(long, default_value_t = 256)]
        memory_limit: usize,

        /// Per-position time limit in seconds.
        #[arg(long, default_value_t = 30)]
        time: u64,

        /// Path to a custom FEN file (default: built-in BK suite).
        #[arg(short, long)]
        positions: Option<PathBuf>,

        /// Emit one machine-readable JSON object.
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// Run a fast paired UCI strength match with fixed node limits and SPRT.
    Duel {
        /// Candidate engine binary.
        candidate: PathBuf,

        /// Reference engine binary.
        reference: PathBuf,

        /// Extra argument passed to the candidate (repeatable).
        #[arg(long = "candidate-arg")]
        candidate_args: Vec<String>,

        /// UCI option for the candidate as NAME=VALUE (repeatable).
        #[arg(long = "candidate-option", value_parser = parse_uci_option)]
        candidate_options: Vec<(String, String)>,

        /// Extra argument passed to the reference (repeatable).
        #[arg(long = "reference-arg")]
        reference_args: Vec<String>,

        /// UCI option for the reference as NAME=VALUE (repeatable).
        #[arg(long = "reference-option", value_parser = parse_uci_option)]
        reference_options: Vec<(String, String)>,

        /// Maximum opening pairs; each pair swaps colors.
        #[arg(long, default_value_t = 32)]
        pairs: usize,

        /// Concurrent pairs. Increase explicitly only on a monitored test host.
        #[arg(short, long, default_value_t = 1)]
        concurrency: usize,

        /// Deterministic node budget for every move.
        #[arg(long, default_value_t = 20_000)]
        nodes: u64,

        /// Fixed thinking time per move in milliseconds; when set, overrides `--nodes`.
        #[arg(long)]
        move_time_ms: Option<u64>,

        /// Hash size for each engine process in MB.
        #[arg(long, default_value_t = 32)]
        hash: usize,

        /// Search threads per engine process.
        #[arg(short = 't', long, default_value_t = 1)]
        threads: usize,

        /// Hard working-set limit for each engine process in MB.
        #[arg(long, default_value_t = 384)]
        max_engine_memory: usize,

        /// Aggregate ceiling implied by all concurrently reserved engine limits.
        #[arg(long, default_value_t = 768)]
        max_match_memory: usize,

        /// Color-swapped pairs played before recycling both engine processes.
        #[arg(long, default_value_t = 1)]
        session_pairs: usize,

        /// Durable JSONL checkpoint; an existing matching file is resumed.
        #[arg(long)]
        checkpoint: Option<PathBuf>,

        /// Optional opening file (`startpos moves ...` or `fen ... moves ...`).
        #[arg(long)]
        openings: Option<PathBuf>,

        /// Zero-based opening index at which to start (useful for resumable runs).
        #[arg(long, default_value_t = 0)]
        opening_offset: usize,

        /// Reference engine's established rating, used for an absolute estimate.
        #[arg(long)]
        reference_elo: Option<f64>,

        /// SPRT null-hypothesis Elo delta.
        #[arg(long, default_value_t = -3.0, allow_hyphen_values = true)]
        elo0: f64,

        /// SPRT alternative-hypothesis Elo delta.
        #[arg(long, default_value_t = 3.0, allow_hyphen_values = true)]
        elo1: f64,

        /// Maximum plies before adjudicating a draw.
        #[arg(long, default_value_t = 300)]
        max_plies: usize,

        /// Emit one machine-readable JSON object.
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// Run a resource-bounded paired round-robin tournament. With no paths,
    /// all compatible engines in `dist/<os-arch>/engines` are discovered automatically.
    Tournament {
        /// UCI engine executables. Omit to use the bundled engine catalog.
        #[arg(value_name = "ENGINE", num_args = 0..)]
        engines: Vec<PathBuf>,

        /// Established rating as ENGINE=ELO, used for performance estimates.
        #[arg(long = "rating", value_parser = parse_engine_rating)]
        ratings: Vec<(String, f64)>,

        /// Color-swapped opening pairs per engine matchup.
        #[arg(long, default_value_t = 4)]
        pairs: usize,

        /// Deterministic node budget for every move.
        #[arg(long, default_value_t = 20_000)]
        nodes: u64,

        /// Hash size for each engine process in MB.
        #[arg(long, default_value_t = 32)]
        hash: usize,

        /// Search threads per engine process.
        #[arg(short = 't', long, default_value_t = 1)]
        threads: usize,

        /// Hard working-set limit for each engine process in MB.
        #[arg(long, default_value_t = 384)]
        max_engine_memory: usize,

        /// Aggregate process-memory ceiling in MB.
        #[arg(long, default_value_t = 768)]
        max_match_memory: usize,

        /// Directory for independently resumable match checkpoints.
        #[arg(long)]
        checkpoint_directory: Option<PathBuf>,

        /// Emit one machine-readable JSON object.
        #[arg(long, default_value_t = false)]
        json: bool,
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
        } => run_bench(BenchRunConfig {
            depth,
            threads,
            hash,
            time,
            positions,
            tui,
            eval_preset,
            eval_file,
            quiet,
            json_output: json,
        }),
        Commands::Nnue {
            iterations,
            warmup,
            eval_file,
            json,
        } => run_nnue_bench(iterations, warmup, eval_file.as_deref(), json),
        Commands::Gauntlet {
            iterations,
            warmup,
            preset,
            play,
            pairs,
            nodes,
            skip_nodes,
            skip_clock,
            root,
            json,
        } => run_adapter_gauntlet(GauntletRunConfig {
            iterations,
            warmup,
            preset,
            play,
            pairs,
            nodes,
            play_nodes: !skip_nodes,
            play_clock: !skip_clock,
            root,
            json_output: json,
        }),
        Commands::Iterate {
            target_elo,
            min_bk,
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
        } => run_iterate(IterateRunConfig {
            target_elo,
            min_bk,
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
        }),
        Commands::Uci {
            engine,
            engine_args,
            uci_options,
            depth,
            nodes,
            hash,
            threads,
            memory_limit,
            time,
            positions,
            json,
        } => run_external(ExternalRunConfig {
            engine,
            engine_args,
            uci_options,
            protocol: ProtocolKind::Uci,
            depth,
            nodes,
            hash,
            threads,
            memory_limit,
            time,
            positions,
            json,
        }),
        Commands::Compare {
            candidate,
            reference,
            candidate_args,
            candidate_options,
            reference_args,
            reference_options,
            rounds,
            nodes,
            hash,
            threads,
            memory_limit,
            minimum_speedup,
            positions,
            json,
        } => run_compare(CompareRunConfig {
            candidate,
            reference,
            candidate_args,
            candidate_options,
            reference_args,
            reference_options,
            rounds,
            nodes,
            hash,
            threads,
            memory_limit,
            minimum_speedup,
            positions,
            json,
        }),
        Commands::Replay {
            candidate,
            reference,
            fen,
            moves,
            candidate_args,
            candidate_options,
            reference_args,
            reference_options,
            nodes,
            hash,
            threads,
            memory_limit,
            json,
        } => run_replay(ReplayRunConfig {
            candidate,
            reference,
            fen,
            moves,
            candidate_args,
            candidate_options,
            reference_args,
            reference_options,
            nodes,
            hash,
            threads,
            memory_limit,
            json,
        }),
        Commands::Xboard {
            engine,
            engine_args,
            depth,
            hash,
            threads,
            memory_limit,
            time,
            positions,
            json,
        } => run_external(ExternalRunConfig {
            engine,
            engine_args,
            uci_options: Vec::new(),
            protocol: ProtocolKind::Xboard,
            depth,
            nodes: None,
            hash,
            threads,
            memory_limit,
            time: Some(time),
            positions,
            json,
        }),
        Commands::Duel {
            candidate,
            reference,
            candidate_args,
            reference_args,
            candidate_options,
            reference_options,
            pairs,
            concurrency,
            nodes,
            move_time_ms,
            hash,
            threads,
            max_engine_memory,
            max_match_memory,
            session_pairs,
            checkpoint,
            openings,
            opening_offset,
            reference_elo,
            elo0,
            elo1,
            max_plies,
            json,
        } => run_duel(DuelRunConfig {
            candidate_path: candidate,
            reference_path: reference,
            candidate_args,
            reference_args,
            candidate_options,
            reference_options,
            pairs,
            concurrency,
            nodes,
            move_time_ms,
            hash,
            threads,
            max_engine_memory,
            max_match_memory,
            session_pairs,
            checkpoint_path: checkpoint,
            openings_path: openings,
            opening_offset,
            reference_elo,
            elo0,
            elo1,
            max_plies,
            json,
        }),
        Commands::Tournament {
            engines,
            ratings,
            pairs,
            nodes,
            hash,
            threads,
            max_engine_memory,
            max_match_memory,
            checkpoint_directory,
            json,
        } => run_round_robin(TournamentRunConfig {
            engines,
            ratings,
            pairs,
            nodes,
            hash,
            threads,
            max_engine_memory,
            max_match_memory,
            checkpoint_directory,
            json,
        }),
        Commands::Info => run_info(),
    }
}

fn run_nnue_bench(
    iterations: u64,
    warmup: u64,
    eval_file: Option<&std::path::Path>,
    json_output: bool,
) {
    let config = NnueBenchConfig { iterations, warmup };
    let result = if let Some(path) = eval_file {
        let network = load_network(path).unwrap_or_else(|error| {
            eprintln!("NNUE load failed: {error}");
            std::process::exit(2);
        });
        nnue_bench::run_with_network(config, network)
    } else {
        nnue_bench::run(config)
    }
    .unwrap_or_else(|error| {
        eprintln!("NNUE benchmark failed: {error}");
        std::process::exit(2);
    });

    if json_output {
        println!("{}", result.to_json_value());
        return;
    }

    println!("NNUE network: {}", result.network);
    println!("Evaluations per workload: {}", result.iterations);
    println!(
        "Hot position: {:.0} eval/s ({:.1} ns/eval)",
        result.hot_evals_per_second(),
        result.hot_ns_per_eval()
    );
    println!(
        "Incremental path: {:.0} eval/s ({:.1} ns/eval)",
        result.incremental_evals_per_second(),
        result.incremental_ns_per_eval()
    );
    println!(
        "BK position cycle: {:.0} eval/s ({:.1} ns/eval)",
        result.suite_evals_per_second(),
        result.suite_ns_per_eval()
    );
    println!("Checksum: {}", result.checksum);
}

struct GauntletRunConfig {
    iterations: u64,
    warmup: u64,
    preset: Option<String>,
    play: bool,
    pairs: usize,
    nodes: u64,
    play_nodes: bool,
    play_clock: bool,
    root: Option<PathBuf>,
    json_output: bool,
}

fn run_adapter_gauntlet(config: GauntletRunConfig) {
    use mujrim_benchmarker::adapter_gauntlet::{
        GauntletTargets, ValidationRequest, run_validation,
    };

    let root = config
        .root
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let payload = match run_validation(&ValidationRequest {
        root,
        preset: config.preset,
        play: config.play,
        play_clock: config.play_clock,
        play_nodes: config.play_nodes,
        pairs: config.pairs.max(1),
        nodes: config.nodes.max(1),
        eval: NnueBenchConfig {
            iterations: config.iterations,
            warmup: config.warmup,
        },
    }) {
        Ok(payload) => payload,
        Err(error) => {
            eprintln!("Error: {error}");
            std::process::exit(1);
        }
    };

    if config.json_output {
        println!("{payload}");
        return;
    }

    let targets = GauntletTargets::v2();
    println!("Adapter gauntlet targets:");
    println!(
        "  NPS ≥ {:.0}% of native · clock H2H ≥ {:.0}% · nodes-equal H2H ≥ {:.0}%",
        targets.nps_ratio * 100.0,
        targets.clock_h2h * 100.0,
        targets.nodes_h2h * 100.0
    );
    println!(
        "  nodes-equal: {} pairs × {} nodes, 128 MB, 1 thread",
        payload["nodes_equal"]["pairs"], payload["nodes_equal"]["nodes"]
    );
    println!(
        "  clock: {} pairs × 3+2 (+3 after 40), 128 MB, 1 thread",
        payload["clock"]["pairs"]
    );
    if let Some(pairs) = payload["pairs"].as_array() {
        for pair in pairs {
            let adapter = pair["adapter_id"].as_str().unwrap_or("?");
            if let Some(eval) = pair["eval"].as_object() {
                println!(
                    "  {:<12} hot {} ns/eval  incr {} ns/eval  ({})",
                    adapter,
                    eval["hot_ns_per_eval"],
                    eval["incremental_ns_per_eval"],
                    eval["network"]
                );
            }
            if let Some(nodes) = pair["nodes_equal"].as_object() {
                println!(
                    "    nodes-equal  {:>5.1}%  nps {:.0}%  met={}",
                    nodes["score"].as_f64().unwrap_or(0.0) * 100.0,
                    nodes["nps_ratio"].as_f64().unwrap_or(0.0) * 100.0,
                    nodes["met"]
                );
            }
            if let Some(clock) = pair["clock"].as_object() {
                println!(
                    "    clock        {:>5.1}%  nps {:.0}%  flags {}/{} leftover {:.0}/{:.0} ms  met={}",
                    clock["score"].as_f64().unwrap_or(0.0) * 100.0,
                    clock["nps_ratio"].as_f64().unwrap_or(0.0) * 100.0,
                    clock["adapter_flags"].as_u64().unwrap_or(0),
                    clock["native_flags"].as_u64().unwrap_or(0),
                    clock["adapter_leftover_ms"].as_f64().unwrap_or(0.0),
                    clock["native_leftover_ms"].as_f64().unwrap_or(0.0),
                    clock["met"]
                );
            }
        }
    }
    if let Some(errors) = payload["errors"].as_array() {
        for error in errors {
            println!("  skipped {error}");
        }
    }
}

fn run_info() {
    let hw = HardwareInfo::detect();
    let nnue = NnueInfo::detect();
    let params = search::search_params::SearchParams::for_preset(nnue.search_profile.as_str());

    println!("╔══════════════════════════════════════════════╗");
    println!("║           MUJRIM ENGINE INFO               ║");
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

struct BenchRunConfig {
    depth: i32,
    threads: Option<usize>,
    hash: usize,
    time: u64,
    positions: Option<PathBuf>,
    tui: bool,
    eval_preset: String,
    eval_file: Option<PathBuf>,
    quiet: bool,
    json_output: bool,
}

fn run_bench(config: BenchRunConfig) {
    let BenchRunConfig {
        depth,
        threads,
        hash,
        time,
        positions,
        tui: _tui,
        eval_preset,
        eval_file,
        quiet,
        json_output,
    } = config;
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
    } else if let Ok(network) = load_network_for_preset(&eval_preset) {
        NnueInfo::from_runtime(network.info())
    } else {
        NnueInfo::detect()
    };
    let params = search::search_params::SearchParams::for_preset(nnue.search_profile.as_str());

    // Single-thread BK runs are slower but far more stable for suite scores (Lazy SMP noise).
    let thread_count = threads.unwrap_or(1);

    if !bench_quiet {
        // Print header
        println!("╔══════════════════════════════════════════════╗");
        println!("║           MUJRIM BENCHMARKER               ║");
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

        let tui_config = internal::InternalBenchConfig {
            quiet: false,
            ..config.clone()
        };

        match mujrim_benchmarker::tui::BenchTui::new(suite.len(), header_lines) {
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

struct IterateRunConfig {
    target_elo: i32,
    min_bk: usize,
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
}

fn run_iterate(config: IterateRunConfig) {
    let IterateRunConfig {
        target_elo,
        min_bk,
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
    } = config;
    let thread_count = threads.unwrap_or(1);

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
        min_bk_correct: (min_bk > 0).then_some(min_bk),
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
            "Target reached: {} — approx. CCRL 40/15 ~{}  BK {}/{} (round {})",
            outcome.reason,
            outcome.final_round.summary.approx_ccrl_40_15,
            outcome.final_round.summary.correct,
            outcome.final_round.summary.total,
            outcome.rounds_run
        );
    }

    if !outcome.success {
        std::process::exit(1);
    }
}

struct ExternalRunConfig {
    engine: PathBuf,
    engine_args: Vec<String>,
    uci_options: Vec<(String, String)>,
    protocol: ProtocolKind,
    depth: i32,
    nodes: Option<u64>,
    hash: usize,
    threads: usize,
    memory_limit: usize,
    time: Option<u64>,
    positions: Option<PathBuf>,
    json: bool,
}

fn run_external(config: ExternalRunConfig) {
    let ExternalRunConfig {
        engine,
        engine_args,
        uci_options,
        protocol,
        depth,
        nodes,
        hash,
        threads,
        memory_limit,
        time,
        positions,
        json,
    } = config;
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

    if !json {
        println!("Benchmarking {} engine: {}", protocol, engine.display());
        if !engine_args.is_empty() {
            println!("Engine args: {}", engine_args.join(" "));
        }
        if let Some(nodes) = nodes {
            println!("Nodes/pos: {nodes}  Hash: {hash}MB  Threads: {threads}");
        } else {
            if let Some(time) = time {
                println!("Time/pos: {time}s  Hash: {hash}MB  Threads: {threads}");
            } else {
                println!("Depth: {depth}  Hash: {hash}MB  Threads: {threads}");
            }
        }
        println!();
    }

    let config = ExternalBenchConfig {
        engine_path: engine,
        engine_args,
        protocol,
        depth,
        hash_mb: hash,
        threads,
        memory_limit_mb: memory_limit,
        uci_options,
        node_limit: nodes,
        time_per_position: time.map(Duration::from_secs),
        quiet: json,
    };

    match external::run_external_bench(&suite, &config) {
        Ok(summary) => {
            if json {
                println!("{}", summary.to_json_value());
            } else {
                println!();
                println!("{summary}");
            }
        }
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}

struct CompareRunConfig {
    candidate: PathBuf,
    reference: PathBuf,
    candidate_args: Vec<String>,
    candidate_options: Vec<(String, String)>,
    reference_args: Vec<String>,
    reference_options: Vec<(String, String)>,
    rounds: usize,
    nodes: u64,
    hash: usize,
    threads: usize,
    memory_limit: usize,
    minimum_speedup: f64,
    positions: Option<PathBuf>,
    json: bool,
}

fn comparison_engine_config(
    path: PathBuf,
    arguments: Vec<String>,
    options: Vec<(String, String)>,
    config: &CompareRunConfig,
) -> ExternalBenchConfig {
    ExternalBenchConfig {
        engine_path: path,
        engine_args: arguments,
        protocol: ProtocolKind::Uci,
        depth: i32::MAX,
        hash_mb: config.hash,
        threads: config.threads,
        memory_limit_mb: config.memory_limit,
        uci_options: options,
        node_limit: Some(config.nodes),
        time_per_position: None,
        quiet: true,
    }
}

fn comparison_warmup_nodes(measured_nodes: u64) -> u64 {
    measured_nodes.clamp(1, 50_000)
}

fn warm_comparison_engine(
    suite: &[suite::TestPosition],
    engine: &ExternalBenchConfig,
    measured_nodes: u64,
) -> Result<(), String> {
    let Some(first) = suite.first() else {
        return Err("comparison suite cannot be empty".to_owned());
    };
    let mut warmup = engine.clone();
    warmup.node_limit = Some(comparison_warmup_nodes(measured_nodes));
    external::run_external_bench(std::slice::from_ref(first), &warmup).map(|_| ())
}

fn run_compare(config: CompareRunConfig) {
    if config.rounds == 0
        || config.nodes == 0
        || config.hash == 0
        || config.threads == 0
        || config.memory_limit < config.hash.saturating_add(64)
        || !config.minimum_speedup.is_finite()
        || config.minimum_speedup < 0.0
    {
        eprintln!(
            "Error: invalid comparison limits; require non-zero rounds/nodes/hash/threads, memory >= hash + 64 MiB, and a finite non-negative speed threshold"
        );
        std::process::exit(2);
    }

    let suite = if let Some(path) = &config.positions {
        suite::load_custom_positions(path).unwrap_or_else(|error| {
            eprintln!("Error: {error}");
            std::process::exit(2);
        })
    } else {
        suite::bk_suite()
    };
    let candidate_config = comparison_engine_config(
        config.candidate.clone(),
        config.candidate_args.clone(),
        config.candidate_options.clone(),
        &config,
    );
    let reference_config = comparison_engine_config(
        config.reference.clone(),
        config.reference_args.clone(),
        config.reference_options.clone(),
        &config,
    );
    let run = |engine: &ExternalBenchConfig| {
        external::run_external_bench(&suite, engine).unwrap_or_else(|error| {
            eprintln!("Error: {error}");
            std::process::exit(2);
        })
    };

    let mut candidate_runs = Vec::with_capacity(config.rounds);
    let mut reference_runs = Vec::with_capacity(config.rounds);
    // Page in both executables and embedded networks before collecting timing.
    // Warmup results are excluded from all tactical and NPS aggregates.
    warm_comparison_engine(&suite, &reference_config, config.nodes)
        .and_then(|()| warm_comparison_engine(&suite, &candidate_config, config.nodes))
        .unwrap_or_else(|error| {
            eprintln!("Error: comparison warmup failed: {error}");
            std::process::exit(2);
        });
    for round in 0..config.rounds {
        if round % 2 == 0 {
            candidate_runs.push(run(&candidate_config));
            reference_runs.push(run(&reference_config));
        } else {
            reference_runs.push(run(&reference_config));
            candidate_runs.push(run(&candidate_config));
        }
    }

    let gate = compare::compare_runs(&candidate_runs, &reference_runs, config.minimum_speedup)
        .unwrap_or_else(|error| {
            eprintln!("Error: {error}");
            std::process::exit(2);
        });

    if config.json {
        println!(
            "{}",
            serde_json::json!({
                "candidate_path": config.candidate,
                "reference_path": config.reference,
                "positions": suite.len(),
                "nodes_per_position": config.nodes,
                "hash_mb": config.hash,
                "threads": config.threads,
                "memory_limit_mb": config.memory_limit,
                "minimum_speedup_percent": config.minimum_speedup,
                "warmup_nodes_per_engine": comparison_warmup_nodes(config.nodes),
                "gate": gate.to_json_value(),
            })
        );
    } else {
        println!("Decision: {} ({})", gate.decision.as_str(), gate.reason);
        println!(
            "Candidate: {}/{} tactical hits, {} NPS",
            gate.candidate_correct,
            suite.len() * config.rounds,
            format_nps(gate.candidate_nps)
        );
        println!(
            "Reference: {}/{} tactical hits, {} NPS",
            gate.reference_correct,
            suite.len() * config.rounds,
            format_nps(gate.reference_nps)
        );
        println!("Speed delta: {:+.2}%", gate.speedup_percent);
        println!("Same moves: {}", gate.same_moves);
    }
}

struct ReplayRunConfig {
    candidate: PathBuf,
    reference: PathBuf,
    fen: String,
    moves: Vec<String>,
    candidate_args: Vec<String>,
    candidate_options: Vec<(String, String)>,
    reference_args: Vec<String>,
    reference_options: Vec<(String, String)>,
    nodes: u64,
    hash: usize,
    threads: usize,
    memory_limit: usize,
    json: bool,
}

fn validate_replay_position(fen: &str, moves: &[String]) -> Result<(), String> {
    if fen.is_empty() || fen.chars().any(char::is_control) {
        return Err("FEN must be non-empty and contain no control characters".to_owned());
    }
    if let Some(invalid) = moves.iter().find(|mv| types::Move::from_uci(mv).is_none()) {
        return Err(format!("invalid UCI move in replay history: {invalid}"));
    }
    Ok(())
}

fn replay_engine_config(
    path: PathBuf,
    arguments: Vec<String>,
    options: Vec<(String, String)>,
    config: &ReplayRunConfig,
) -> ExternalBenchConfig {
    ExternalBenchConfig {
        engine_path: path,
        engine_args: arguments,
        protocol: ProtocolKind::Uci,
        depth: i32::MAX,
        hash_mb: config.hash,
        threads: config.threads,
        memory_limit_mb: config.memory_limit,
        uci_options: options,
        node_limit: Some(config.nodes),
        time_per_position: None,
        quiet: true,
    }
}

fn run_replay(config: ReplayRunConfig) {
    if config.nodes == 0
        || config.hash == 0
        || config.threads == 0
        || config.memory_limit < config.hash.saturating_add(64)
    {
        eprintln!(
            "Error: invalid replay limits; require non-zero nodes/hash/threads and memory >= hash + 64 MiB"
        );
        std::process::exit(2);
    }
    validate_replay_position(&config.fen, &config.moves).unwrap_or_else(|error| {
        eprintln!("Error: {error}");
        std::process::exit(2);
    });

    let candidate_config = replay_engine_config(
        config.candidate.clone(),
        config.candidate_args.clone(),
        config.candidate_options.clone(),
        &config,
    );
    let reference_config = replay_engine_config(
        config.reference.clone(),
        config.reference_args.clone(),
        config.reference_options.clone(),
        &config,
    );
    let result = replay::compare_position(
        &candidate_config,
        &reference_config,
        &config.fen,
        &config.moves,
    )
    .unwrap_or_else(|error| {
        eprintln!("Error: {error}");
        std::process::exit(2);
    });

    if config.json {
        println!(
            "{}",
            result.to_json_value(&config.fen, &config.moves, config.nodes)
        );
        return;
    }

    println!("Position: {}", config.fen);
    println!("Moves: {}", config.moves.join(" "));
    for (label, probe) in [
        ("Candidate", &result.candidate),
        ("Reference", &result.reference),
    ] {
        println!(
            "{label}: {} score {:+} depth {}/{} nodes {} nps {} pv {}",
            probe.info.best_move,
            probe.info.score,
            probe.info.depth,
            probe.info.seldepth,
            probe.info.nodes,
            format_nps(probe.info.nps),
            probe.info.pv.join(" ")
        );
    }
    println!("Same best move: {}", result.same_best_move());
    println!("Score delta: {:+} cp", result.score_delta());
}

struct DuelRunConfig {
    candidate_path: PathBuf,
    reference_path: PathBuf,
    candidate_args: Vec<String>,
    reference_args: Vec<String>,
    candidate_options: Vec<(String, String)>,
    reference_options: Vec<(String, String)>,
    pairs: usize,
    concurrency: usize,
    nodes: u64,
    move_time_ms: Option<u64>,
    hash: usize,
    threads: usize,
    max_engine_memory: usize,
    max_match_memory: usize,
    session_pairs: usize,
    checkpoint_path: Option<PathBuf>,
    openings_path: Option<PathBuf>,
    opening_offset: usize,
    reference_elo: Option<f64>,
    elo0: f64,
    elo1: f64,
    max_plies: usize,
    json: bool,
}

struct TournamentRunConfig {
    engines: Vec<PathBuf>,
    ratings: Vec<(String, f64)>,
    pairs: usize,
    nodes: u64,
    hash: usize,
    threads: usize,
    max_engine_memory: usize,
    max_match_memory: usize,
    checkpoint_directory: Option<PathBuf>,
    json: bool,
}

fn run_round_robin(config: TournamentRunConfig) {
    if config.pairs == 0
        || config.nodes == 0
        || config.hash == 0
        || config.threads == 0
        || config.max_engine_memory == 0
        || config.max_match_memory < config.max_engine_memory.saturating_mul(2)
    {
        eprintln!(
            "Error: tournament limits must be positive and aggregate memory must cover two engines"
        );
        std::process::exit(2);
    }
    let mut specs = if config.engines.is_empty() {
        mujrim_protocols::catalog::discover_bundled_engines_from_environment()
            .unwrap_or_else(|error| {
                eprintln!("Error: {error}");
                std::process::exit(2);
            })
            .into_iter()
            .map(|engine| {
                let mut spec = EngineSpec::new(engine.path);
                spec.name = engine.display_name.to_owned();
                (spec, engine.search_limits)
            })
            .collect::<Vec<_>>()
    } else {
        config
            .engines
            .into_iter()
            .map(|path| {
                (
                    EngineSpec::new(path),
                    mujrim_protocols::catalog::SearchLimitSupport::STANDARD,
                )
            })
            .collect()
    };
    if specs.len() < 2 {
        eprintln!("Error: a tournament requires at least two compatible engines");
        std::process::exit(2);
    }
    specs.sort_by(|left, right| left.0.name.cmp(&right.0.name));
    let engines = specs
        .into_iter()
        .map(|(engine, search_limits)| {
            let established_elo = config
                .ratings
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(&engine.name))
                .map(|(_, rating)| *rating);
            TournamentEngine {
                engine,
                established_elo,
                search_limits,
            }
        })
        .collect();
    let match_config = MatchConfig {
        pairs: config.pairs,
        concurrency: 1,
        nodes_per_move: config.nodes,
        hash_mb: config.hash,
        engine_threads: config.threads,
        max_engine_memory_mb: config.max_engine_memory,
        max_match_memory_mb: config.max_match_memory,
        session_pairs: 1,
        early_stop: false,
        checkpoint_path: None,
        ..MatchConfig::default()
    };
    let summary = run_tournament(
        engines,
        TournamentConfig {
            match_config,
            checkpoint_directory: config.checkpoint_directory,
            ..TournamentConfig::default()
        },
    );
    if config.json {
        println!("{}", summary.to_json_value());
    } else {
        println!("Paired round-robin standings");
        for (rank, standing) in summary.standings.iter().enumerate() {
            let engine = &summary.engines[standing.entrant];
            let performance = standing.performance.map_or_else(
                || "unrated".to_owned(),
                |rating| format!("{:.0} Elo", rating.elo),
            );
            println!(
                "{:>2}. {:<20} {:>5.1} points  {}-{}-{}  {performance}",
                rank + 1,
                engine.engine.name,
                standing.points,
                standing.wins,
                standing.draws,
                standing.losses,
            );
        }
        if let Some(error) = summary.error {
            eprintln!("Tournament stopped: {error}");
        }
    }
}

fn run_duel(config: DuelRunConfig) {
    let DuelRunConfig {
        candidate_path,
        reference_path,
        candidate_args,
        reference_args,
        candidate_options,
        reference_options,
        pairs,
        concurrency,
        nodes,
        move_time_ms,
        hash,
        threads,
        max_engine_memory,
        max_match_memory,
        session_pairs,
        checkpoint_path,
        openings_path,
        opening_offset,
        reference_elo,
        elo0,
        elo1,
        max_plies,
        json,
    } = config;
    if pairs == 0
        || concurrency == 0
        || (nodes == 0 && move_time_ms.is_none())
        || move_time_ms == Some(0)
        || threads == 0
        || max_engine_memory == 0
        || max_match_memory == 0
        || session_pairs == 0
        || elo0 >= elo1
    {
        eprintln!(
            "Error: pairs, concurrency, search limit, threads, memory limits, and session pairs must be positive, and elo0 must be below elo1"
        );
        std::process::exit(2);
    }
    let openings = openings_path.map(|path| {
        load_openings(&path).unwrap_or_else(|error| {
            eprintln!("Error: {error}");
            std::process::exit(2);
        })
    });
    let mut candidate = EngineSpec::new(candidate_path);
    candidate.args = candidate_args;
    candidate.uci_options = candidate_options;
    let mut reference = EngineSpec::new(reference_path);
    reference.args = reference_args;
    reference.uci_options = reference_options;
    let defaults = MatchConfig::default();
    let config = MatchConfig {
        pairs,
        opening_offset,
        concurrency,
        nodes_per_move: if move_time_ms.is_some() { 0 } else { nodes },
        move_time: move_time_ms.map(std::time::Duration::from_millis),
        hash_mb: hash,
        engine_threads: threads,
        max_engine_memory_mb: max_engine_memory,
        max_match_memory_mb: max_match_memory,
        session_pairs,
        checkpoint_path,
        max_plies,
        sprt: Sprt {
            elo0,
            elo1,
            ..Sprt::default()
        },
        reference_elo,
        ..defaults
    };
    let summary = run_match(candidate, reference, openings, config);

    if json {
        println!("{}", summary.to_json_value());
    } else {
        println!(
            "{} vs {}: {}-{}-{} ({:.2}%), {:+.1} Elo [{:+.1}, {:+.1}]",
            summary.candidate,
            summary.reference,
            summary.scores.wins,
            summary.scores.draws,
            summary.scores.losses,
            summary.scores.score_rate() * 100.0,
            summary.elo_delta,
            summary.elo_low,
            summary.elo_high,
        );
        println!(
            "{} pairs / {} games, LLR {:.3}, SPRT {:?}, {} nodes in {:.2}s",
            summary.pairs.len(),
            summary.scores.games(),
            summary.llr,
            summary.sprt_decision,
            summary.total_nodes,
            summary.elapsed.as_secs_f64(),
        );
        if summary.resumed_pairs > 0 {
            println!(
                "Resumed {} completed pairs from checkpoint",
                summary.resumed_pairs
            );
        }
        if let Some(elo) = summary.candidate_elo() {
            println!("Anchored candidate estimate: {elo:.1} Elo");
        }
    }

    if let Some(error) = summary.error {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
    if summary.sprt_decision == SprtDecision::AcceptH0 {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod cli_tests {
    use super::{
        Cli, Commands, comparison_warmup_nodes, parse_engine_rating, parse_uci_option,
        validate_replay_position,
    };
    use clap::Parser;

    #[test]
    fn parses_uci_option_with_equals_in_value() {
        assert_eq!(
            parse_uci_option("EvalFile=C:\\nets\\v60=release.nnue").unwrap(),
            (
                "EvalFile".to_string(),
                "C:\\nets\\v60=release.nnue".to_string()
            )
        );
    }

    #[test]
    fn rejects_malformed_uci_option() {
        assert!(parse_uci_option("EvalFile").is_err());
        assert!(parse_uci_option("=network.nnue").is_err());
    }

    #[test]
    fn external_uci_benchmark_accepts_repeatable_options() {
        let cli = Cli::try_parse_from([
            "mujrim-benchmarker",
            "uci",
            "engine.exe",
            "--option",
            "EvalFile=v60.nnue",
            "--option",
            "Move Overhead=25",
            "--nodes",
            "12345",
            "--json",
        ])
        .unwrap();

        let Commands::Uci {
            uci_options,
            nodes,
            json,
            ..
        } = cli.command
        else {
            panic!("expected UCI command");
        };
        assert_eq!(nodes, Some(12_345));
        assert!(json);
        assert_eq!(
            uci_options,
            vec![
                ("EvalFile".to_string(), "v60.nnue".to_string()),
                ("Move Overhead".to_string(), "25".to_string()),
            ]
        );
    }

    #[test]
    fn comparison_gate_parses_bounded_triage_settings() {
        let cli = Cli::try_parse_from([
            "mujrim-benchmarker",
            "compare",
            "candidate.exe",
            "reference.exe",
            "--rounds",
            "1",
            "--nodes",
            "50000",
            "--memory-limit",
            "256",
            "--json",
        ])
        .unwrap();

        let Commands::Compare {
            rounds,
            nodes,
            memory_limit,
            minimum_speedup,
            json,
            ..
        } = cli.command
        else {
            panic!("expected compare command");
        };
        assert_eq!(rounds, 1);
        assert_eq!(nodes, 50_000);
        assert_eq!(memory_limit, 256);
        assert_eq!(minimum_speedup, 2.0);
        assert!(json);
    }

    #[test]
    fn comparison_warmup_is_bounded_and_never_zero() {
        assert_eq!(comparison_warmup_nodes(0), 1);
        assert_eq!(comparison_warmup_nodes(25_000), 25_000);
        assert_eq!(comparison_warmup_nodes(4_000_000), 50_000);
    }

    #[test]
    fn duel_accepts_fixed_move_time_as_the_search_limit() {
        let cli = Cli::try_parse_from([
            "mujrim-benchmarker",
            "duel",
            "candidate.exe",
            "reference.exe",
            "--move-time-ms",
            "25",
            "--nodes",
            "0",
        ])
        .unwrap();

        let Commands::Duel {
            move_time_ms,
            nodes,
            ..
        } = cli.command
        else {
            panic!("expected duel command");
        };
        assert_eq!(move_time_ms, Some(25));
        assert_eq!(nodes, 0);
    }

    #[test]
    fn replay_parses_ordered_moves_and_resource_limits() {
        let cli = Cli::try_parse_from([
            "mujrim-benchmarker",
            "replay",
            "candidate.exe",
            "reference.exe",
            "--move",
            "e2e4",
            "--move",
            "e7e5",
            "--nodes",
            "100000",
            "--memory-limit",
            "256",
            "--json",
        ])
        .unwrap();

        let Commands::Replay {
            moves,
            nodes,
            memory_limit,
            json,
            ..
        } = cli.command
        else {
            panic!("expected replay command");
        };
        assert_eq!(moves, ["e2e4", "e7e5"]);
        assert_eq!(nodes, 100_000);
        assert_eq!(memory_limit, 256);
        assert!(json);
    }

    #[test]
    fn replay_position_rejects_protocol_injection() {
        assert!(validate_replay_position("", &[]).is_err());
        assert!(validate_replay_position("start\nquit", &[]).is_err());
        assert!(validate_replay_position("fixture", &["e2e4".to_owned()]).is_ok());
        assert!(validate_replay_position("fixture", &["e2e4\nquit".to_owned()]).is_err());
    }

    #[test]
    fn gauntlet_command_defaults_to_eval_card() {
        let cli =
            Cli::try_parse_from(["mujrim-benchmarker", "gauntlet", "--preset", "akimbo"]).unwrap();
        let Commands::Gauntlet {
            iterations,
            warmup,
            preset,
            play,
            pairs,
            nodes,
            skip_nodes,
            skip_clock,
            root,
            json,
        } = cli.command
        else {
            panic!("expected gauntlet command");
        };
        assert_eq!(iterations, 8_000);
        assert_eq!(warmup, 200);
        assert_eq!(preset.as_deref(), Some("akimbo"));
        assert!(!play);
        assert_eq!(pairs, 4);
        assert_eq!(nodes, 20_000);
        assert!(!skip_nodes);
        assert!(!skip_clock);
        assert!(root.is_none());
        assert!(!json);
    }

    #[test]
    fn gauntlet_play_flags_select_cards() {
        let cli = Cli::try_parse_from([
            "mujrim-benchmarker",
            "gauntlet",
            "--play",
            "--pairs",
            "1",
            "--skip-clock",
            "--preset",
            "akimbo",
        ])
        .unwrap();
        let Commands::Gauntlet {
            play,
            pairs,
            skip_clock,
            skip_nodes,
            preset,
            ..
        } = cli.command
        else {
            panic!("expected gauntlet command");
        };
        assert!(play);
        assert_eq!(pairs, 1);
        assert!(skip_clock);
        assert!(!skip_nodes);
        assert_eq!(preset.as_deref(), Some("akimbo"));
    }

    #[test]
    fn tournament_can_auto_discover_engines_and_parse_ratings() {
        let cli = Cli::try_parse_from([
            "mujrim-benchmarker",
            "tournament",
            "--rating",
            "Stockfish=3651",
            "--pairs",
            "2",
        ])
        .unwrap();
        let Commands::Tournament {
            engines,
            ratings,
            pairs,
            ..
        } = cli.command
        else {
            panic!("expected tournament command");
        };
        assert!(engines.is_empty());
        assert_eq!(ratings, vec![("Stockfish".to_owned(), 3651.0)]);
        assert_eq!(pairs, 2);
    }

    #[test]
    fn engine_rating_parser_rejects_invalid_ranges() {
        assert_eq!(
            parse_engine_rating("Reckless=3634").unwrap(),
            ("Reckless".to_owned(), 3634.0)
        );
        assert!(parse_engine_rating("Reckless").is_err());
        assert!(parse_engine_rating("Reckless=9000").is_err());
    }
}
