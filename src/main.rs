//! KishMat Chess Engine — CLI entry point.
//!
//! Supports multiple modes:
//! - `uci`: Standard UCI protocol (default when no subcommand)
//! - `xboard`: XBoard/CECP protocol for WinBoard and XBoard GUIs
//! - `play`: Interactive play against the engine
//! - `analyze`: Analyze a FEN position
//! - `bench`: Run ELO estimation benchmark
//! - `perft`: Run perft test

#[cfg(feature = "mimalloc")]
use mimalloc::MiMalloc;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

mod commands;

use clap::{Arg, Command};

fn main() {
    types::init();

    let matches = Command::new("KishMat Chess Engine")
        .version("2.0.0")
        .author("Ahmad Hamdi <contact@hamdiz.me>")
        .about("A high-performance chess engine with NNUE-enhanced evaluation")
        .subcommand(
            Command::new("uci")
                .about("Run in UCI protocol mode (for chess GUIs)")
        )
        .subcommand(
            Command::new("xboard")
                .about("Run in XBoard/CECP protocol mode (for WinBoard and XBoard GUIs)")
        )
        .subcommand(
            Command::new("play")
                .about("Play interactively against the engine")
                .arg(
                    Arg::new("depth")
                        .short('d')
                        .long("depth")
                        .value_name("DEPTH")
                        .default_value("5")
                        .help("Search depth for the engine"),
                ),
        )
        .subcommand(
            Command::new("analyze")
                .about("Analyze a given position")
                .arg(
                    Arg::new("fen")
                        .short('f')
                        .long("fen")
                        .value_name("FEN")
                        .required(true)
                        .help("FEN string of the position to analyze"),
                )
                .arg(
                    Arg::new("depth")
                        .short('d')
                        .long("depth")
                        .value_name("DEPTH")
                        .default_value("10")
                        .help("Search depth"),
                ),
        )
        .subcommand(
            Command::new("perft")
                .about("Run perft test on starting position")
                .arg(
                    Arg::new("depth")
                        .short('d')
                        .long("depth")
                        .value_name("DEPTH")
                        .default_value("5")
                        .help("Perft depth"),
                )
                .arg(
                    Arg::new("fen")
                        .short('f')
                        .long("fen")
                        .value_name("FEN")
                        .help("Optional FEN string"),
                ),
        )
        .subcommand(
            Command::new("bench")
                .about("Run ELO estimation benchmark suite")
                .arg(
                    Arg::new("depth")
                        .short('d')
                        .long("depth")
                        .value_name("DEPTH")
                        .default_value("18")
                        .help("Search depth for benchmark"),
                )
                .arg(
                    Arg::new("time")
                        .short('t')
                        .long("time")
                        .value_name("MS")
                        .help("Time per position in ms (overrides depth)"),
                ),
        )
        .get_matches();

    match matches.subcommand() {
        Some(("uci", _)) => {
            commands::run_uci();
        }
        Some(("xboard", _)) => {
            #[cfg(feature = "xboard")]
            commands::run_xboard();
            #[cfg(not(feature = "xboard"))]
            eprintln!("XBoard protocol support disabled (compile with --features xboard)");
        }
        Some(("play", sub)) => {
            let depth: i32 = sub.get_one::<String>("depth")
                .unwrap().parse().unwrap_or(5);
            commands::run_play(depth);
        }
        Some(("analyze", sub)) => {
            let fen = sub.get_one::<String>("fen").unwrap();
            let depth: i32 = sub.get_one::<String>("depth")
                .unwrap().parse().unwrap_or(10);
            commands::run_analyze(fen, depth);
        }
        Some(("perft", sub)) => {
            let depth: u32 = sub.get_one::<String>("depth")
                .unwrap().parse().unwrap_or(5);
            let fen = sub.get_one::<String>("fen").map(|s| s.as_str());
            commands::run_perft(depth, fen);
        }
        Some(("bench", sub)) => {
            let depth: i32 = sub.get_one::<String>("depth")
                .unwrap().parse().unwrap_or(10);
            let time_ms: Option<u64> = sub.get_one::<String>("time")
                .and_then(|s| s.parse().ok());
            commands::run_bench(depth, time_ms);
        }
        _ => {
            commands::run_uci();
        }
    }
}
