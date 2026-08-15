#![cfg_attr(all(target_os = "windows", not(test)), windows_subsystem = "windows")]

//! Mujrim Chess Engine — CLI entry point.
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
use std::path::PathBuf;

const EXTERNAL_BACKENDS: &[&str] = &[
    "stockfish",
    "plentychess",
    "obsidian",
    "reckless",
    "ethereal",
    "lc0",
    "viridithas",
    "hobbes",
    "integral",
    "velvet",
];
const MUJRIM_ADAPTERS: &[&str] = &[
    "mujrim-elite",
    "mujrim-external",
    "mujrim-v60",
    "mujrim-ak",
    "mujrim-viri",
    "mujrim-obs",
    "mujrim-plenty",
    "mujrim-ateed",
    "mujrim-lc0",
];
const V60_PASSTHROUGH_MARKER: &str = "MUJRIM_V60_PASSTHROUGH_ACTIVE";

#[derive(Clone, Copy)]
struct NativeSearchStackProfile {
    engine_id: &'static str,
    display_name: &'static str,
    authors: &'static str,
    memory_limit_bytes: u64,
    max_hash_mb: usize,
    max_threads: usize,
}

fn search_stack_profile(engine_id: &'static str) -> NativeSearchStackProfile {
    match engine_id {
        "stockfish" => NativeSearchStackProfile {
            engine_id,
            display_name: "Mujrim Elite 1.0.0",
            authors: "Ahmad Hamdi Emara (Egypt)",
            memory_limit_bytes: 1536 * 1024 * 1024,
            max_hash_mb: 1024,
            max_threads: 12,
        },
        "reckless" => NativeSearchStackProfile {
            engine_id,
            display_name: "Mujrim v60 1.0.0",
            authors: "Ahmad Hamdi Emara (Egypt)",
            memory_limit_bytes: 1024 * 1024 * 1024,
            max_hash_mb: 768,
            max_threads: 8,
        },
        _ => NativeSearchStackProfile {
            engine_id,
            display_name: "Mujrim External Search Adapter 1.0.0",
            authors: "Ahmad Hamdi Emara (Egypt) / upstream engine authors",
            memory_limit_bytes: 512 * 1024 * 1024,
            max_hash_mb: 384,
            max_threads: 8,
        },
    }
}

fn resolve_backend_engine_id(backend: &str) -> Option<&'static str> {
    match backend {
        "v60" => Some("mujrim-v60"),
        "v10" | "elite" => Some("mujrim-elite"),
        "akimbo" | "ak" => Some("mujrim-ak"),
        "viridithas" | "viri" | "mujrim-viri" => Some("viridithas"),
        "obsidian" | "obs" | "mujrim-obs" => Some("obsidian"),
        "plentychess" | "plenty" | "mujrim-plenty" => Some("plentychess"),
        "ateed" | "mujrim-ateed" => Some("ateed"),
        "lc0" | "mujrim-lc0" => Some("lc0"),
        "external" => Some("mujrim-external"),
        other => EXTERNAL_BACKENDS
            .iter()
            .copied()
            .find(|candidate| *candidate == other),
    }
}

fn passthrough_engine_id(
    backend: &str,
    uci_mode: bool,
    v60_passthrough_active: bool,
    explicit_path: bool,
) -> Option<&'static str> {
    if !uci_mode
        || matches!(backend, "universal" | "mujrim-hce")
        || (backend == "v60" && v60_passthrough_active)
    {
        return None;
    }
    if matches!(
        backend,
        "viridithas"
            | "viri"
            | "mujrim-viri"
            | "obsidian"
            | "obs"
            | "mujrim-obs"
            | "plentychess"
            | "plenty"
            | "mujrim-plenty"
            | "ateed"
            | "mujrim-ateed"
    ) && !explicit_path
    {
        return None;
    }
    resolve_backend_engine_id(backend)
}

fn product_adapter_from_exe_stem(stem: &str) -> Option<&'static str> {
    match stem {
        "mujrim-viri" | "mujrim-viridithas" => Some("viridithas"),
        "mujrim-obs" | "mujrim-obsidian" => Some("obsidian"),
        "mujrim-plenty" | "mujrim-plentychess" => Some("plentychess"),
        "mujrim-ateed" => Some("ateed"),
        "mujrim-lc0" | "mujrim-leela" => Some("lc0"),
        "mujrim-elite" | "mujrim-embedded" => Some("stockfish"),
        "mujrim-ak" | "mujrim-akimbo" => Some("akimbo"),
        "mujrim-v60" | "mujrim-v60-embedded" => Some("reckless"),
        _ => None,
    }
}

fn fallback_engine_id(backend: &str, explicit_path: bool) -> Option<&'static str> {
    if explicit_path {
        return None;
    }
    match backend {
        "stockfish" => Some("mujrim-elite"),
        "reckless" => Some("mujrim-v60"),
        _ => None,
    }
}

fn is_mujrim_adapter(engine_id: &str) -> bool {
    MUJRIM_ADAPTERS.contains(&engine_id)
}

fn run_external_backend(engine_id: &str, explicit_path: Option<&PathBuf>) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to locate current executable: {error}"))?;
    let current_dir = std::env::current_dir()
        .map_err(|error| format!("failed to locate current directory: {error}"))?;
    let engine = mujrim_protocols::catalog::discover_engine(
        engine_id,
        &executable,
        &current_dir,
        explicit_path.map(PathBuf::as_path),
    )?;
    // Never passthrough to ourselves — that re-enters the same stockfish/elite fallback loop.
    if let (Ok(self_path), Ok(engine_path)) = (
        std::fs::canonicalize(&executable),
        std::fs::canonicalize(&engine),
    ) && self_path == engine_path
    {
        return Err(format!(
            "refusing to passthrough '{engine_id}' to the current executable ({})",
            executable.display()
        ));
    }
    let environment: &[(&str, &str)] = if engine_id == "mujrim-v60" {
        &[(V60_PASSTHROUGH_MARKER, "1")]
    } else {
        &[]
    };
    let (engine, extra_args) = if engine_id == "lc0" {
        let device = mujrim_protocols::detect_device_kind();
        let launch = mujrim_protocols::plan_launch(&engine, device);
        let extra_args = launch.argv();
        (launch.binary, extra_args)
    } else {
        (engine, Vec::new())
    };
    let status = if is_mujrim_adapter(engine_id) {
        let memory_limit = match engine_id {
            "mujrim-elite" | "mujrim-v10" => Some(1536 * 1024 * 1024),
            "mujrim-v60" => Some(1024 * 1024 * 1024),
            _ => Some(512 * 1024 * 1024),
        };
        mujrim_protocols::run_passthrough_with_environment(
            &engine,
            &extra_args,
            environment,
            memory_limit,
        )?
    } else {
        let profile = search_stack_profile(
            EXTERNAL_BACKENDS
                .iter()
                .copied()
                .find(|candidate| *candidate == engine_id)
                .ok_or_else(|| format!("unsupported external search stack '{engine_id}'"))?,
        );
        debug_assert_eq!(profile.engine_id, engine_id);
        let adapter = mujrim_protocols::BoundedUciIdentityAdapter {
            identity: mujrim_protocols::UciIdentityAdapter {
                name: profile.display_name,
                author: profile.authors,
            },
            max_hash_mb: profile.max_hash_mb,
            max_threads: profile.max_threads,
        };
        mujrim_protocols::run_uci_search_stack_adapter(
            &engine,
            &extra_args,
            environment,
            Some(profile.memory_limit_bytes),
            &adapter,
        )?
    };
    if status.success() {
        Ok(())
    } else {
        Err(format!("'{}' exited with {status}", engine.display()))
    }
}

fn main() {
    types::init();

    let matches = Command::new("Mujrim Chess Engine")
        .version("1.0.0")
        .author("Ahmad Hamdi <contact@hamdiz.me>")
        .about("A high-performance chess engine with NNUE-enhanced evaluation")
        .arg(
            Arg::new("backend")
                .long("backend")
                .value_parser([
                    "v60",
                    "v10",
                    "universal",
                    "mujrim-hce",
                    "stockfish",
                    "plentychess",
                    "obsidian",
                    "viridithas",
                    "ateed",
                    "reckless",
                    "akimbo",
                    "ethereal",
                    "lc0",
                ])
                .default_value("universal")
                .global(true)
                .help(
                    "Search backend to expose over UCI (default: in-process universal; v60/v10/akimbo prefer Mujrim adapters; mujrim-hce is in-process HCE)",
                ),
        )
        .arg(
            Arg::new("engine-path")
                .long("engine-path")
                .value_name("PATH")
                .value_parser(clap::value_parser!(PathBuf))
                .global(true)
                .help("Explicit executable for the selected external backend"),
        )
        .subcommand(Command::new("uci").about("Run in UCI protocol mode (for chess GUIs)"))
        .subcommand(
            Command::new("xboard")
                .about("Run in XBoard/CECP protocol mode (for WinBoard and XBoard GUIs)"),
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
            Command::new("train")
                .about("NNUE training pipeline")
                .subcommand(
                    Command::new("emit-ateed")
                        .about("Write a well-formed zero Ateed MoE network")
                        .arg(
                            Arg::new("output")
                                .short('o')
                                .long("output")
                                .value_name("PATH")
                                .default_value("ateed_default.bin")
                                .help("Destination path for the Ateed payload"),
                        ),
                )
                .subcommand(
                    Command::new("datagen")
                        .about("Self-play dataset generation (append-only)")
                        .arg(
                            Arg::new("games")
                                .short('g')
                                .long("games")
                                .value_name("N")
                                .default_value("1")
                                .help("Number of self-play games"),
                        )
                        .arg(
                            Arg::new("depth")
                                .short('d')
                                .long("depth")
                                .value_name("DEPTH")
                                .default_value("6")
                                .help("Search depth per move"),
                        )
                        .arg(
                            Arg::new("output")
                                .short('o')
                                .long("output")
                                .value_name("PATH")
                                .default_value("data.txt")
                                .help("Append-only dataset path"),
                        )
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_name("text|plain|binpack")
                                .default_value("text")
                                .help("Write Mujrim text, Stockfish plain, or MJBP binpack"),
                        ),
                )
                .subcommand(
                    Command::new("ateed")
                        .about("Train an Ateed checkpoint from a text dataset")
                        .arg(
                            Arg::new("data")
                                .short('d')
                                .long("data")
                                .value_name("PATH[,PATH]")
                                .default_value("data.txt")
                                .help("One or more datasets (text, plain, MJBP, PGN, gz, zst)"),
                        )
                        .arg(
                            Arg::new("mix")
                                .long("mix")
                                .value_name("W[,W]")
                                .help("Per-source mix weights; interleaves instead of concatenating"),
                        )
                        .arg(
                            Arg::new("seed")
                                .long("seed")
                                .value_name("N")
                                .default_value("1")
                                .help("Shuffle seed after the weighted mix"),
                        )
                        .arg(
                            Arg::new("output")
                                .short('o')
                                .long("output")
                                .value_name("PATH")
                                .default_value("ateed_default.bin")
                                .help("Destination Ateed payload"),
                        )
                        .arg(
                            Arg::new("epochs")
                                .short('e')
                                .long("epochs")
                                .value_name("N")
                                .default_value("8")
                                .help("Training epochs"),
                        )
                        .arg(
                            Arg::new("lr")
                                .long("lr")
                                .value_name("RATE")
                                .default_value("1.0")
                                .help("SGD learning rate"),
                        )
                        .arg(
                            Arg::new("wdl-weight")
                                .long("wdl-weight")
                                .value_name("W")
                                .default_value("0.25")
                                .help("Mixture weight for WDL cross-entropy"),
                        )
                        .arg(
                            Arg::new("scope")
                                .long("scope")
                                .value_name("heads|expert0|moe")
                                .default_value("heads")
                                .help("Train output biases, expert 0 + FT, or routed MoE heads"),
                        )
                        .arg(
                            Arg::new("base")
                                .long("base")
                                .value_name("PATH")
                                .help("Optional Ateed checkpoint to fine-tune"),
                        ),
                )
                .subcommand(
                    Command::new("fetch")
                        .about("Download a catalog or remote dataset with HTTP Range resume")
                        .arg(
                            Arg::new("id")
                                .long("id")
                                .value_name("ID")
                                .help("Catalog id from `mujrim train catalog`"),
                        )
                        .arg(
                            Arg::new("url")
                                .long("url")
                                .value_name("URL")
                                .help("http(s) file URL; required when the catalog id is a directory root"),
                        )
                        .arg(
                            Arg::new("output")
                                .short('o')
                                .long("output")
                                .value_name("PATH")
                                .help("Destination path; resumes into a .part file"),
                        ),
                )
                .subcommand(
                    Command::new("catalog")
                        .about("List Stockfish, Lc0, and self-play dataset ids"),
                )
                .subcommand(
                    Command::new("decode")
                        .about("Decompress and decode a dump into a training dataset")
                        .arg(
                            Arg::new("input")
                                .short('i')
                                .long("input")
                                .value_name("PATH")
                                .required(true)
                                .help("Source dump (gz, zst, plain, MJBP, PGN, text)"),
                        )
                        .arg(
                            Arg::new("output")
                                .short('o')
                                .long("output")
                                .value_name("PATH")
                                .default_value("data.txt")
                                .help("Decoded dataset path"),
                        )
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_name("text|plain|binpack")
                                .default_value("text")
                                .help("Output encoding"),
                        ),
                )
                .subcommand(
                    Command::new("merge")
                        .about("Weighted-interleave multiple decoded sources into one epoch")
                        .arg(
                            Arg::new("data")
                                .short('d')
                                .long("data")
                                .value_name("PATH[,PATH]")
                                .required(true)
                                .help("Comma-separated source paths"),
                        )
                        .arg(
                            Arg::new("mix")
                                .long("mix")
                                .value_name("W[,W]")
                                .help("Per-source mix weights (default 1 per source)"),
                        )
                        .arg(
                            Arg::new("seed")
                                .long("seed")
                                .value_name("N")
                                .default_value("1")
                                .help("Shuffle seed after the mix"),
                        )
                        .arg(
                            Arg::new("output")
                                .short('o')
                                .long("output")
                                .value_name("PATH")
                                .default_value("mix.txt")
                                .help("Merged dataset path"),
                        )
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .value_name("text|plain|binpack")
                                .default_value("text")
                                .help("Output encoding"),
                        ),
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
                        .default_value("16")
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

    let uci_mode =
        matches.subcommand().is_none() || matches!(matches.subcommand(), Some(("uci", _)));
    let requested_backend = matches
        .get_one::<String>("backend")
        .map_or("universal", String::as_str);
    let inferred = std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.file_stem()
                .and_then(|stem| stem.to_str().map(str::to_owned))
        })
        .and_then(|stem| product_adapter_from_exe_stem(&stem));
    let backend = if requested_backend == "universal" {
        inferred.unwrap_or("universal")
    } else {
        requested_backend
    };
    let passthrough_active = std::env::var_os(V60_PASSTHROUGH_MARKER).is_some();
    let uci_smoke = std::env::var_os("MUJRIM_UCI_SMOKE").is_some();
    let explicit_path = matches.get_one::<PathBuf>("engine-path");
    if !uci_smoke
        && let Some(engine_id) = passthrough_engine_id(
            requested_backend,
            uci_mode,
            passthrough_active,
            explicit_path.is_some(),
        )
        .or_else(|| (uci_mode && backend == "lc0" && explicit_path.is_none()).then_some("lc0"))
    {
        match run_external_backend(engine_id, explicit_path) {
            Ok(()) => return,
            Err(error) => {
                if engine_id == "lc0" && explicit_path.is_none() {
                    eprintln!(
                        "info string official lc0 unavailable; using in-process Lc0 adapter: {error}"
                    );
                } else if let Some(fallback) = fallback_engine_id(backend, explicit_path.is_some())
                {
                    eprintln!(
                        "info string {backend} backend unavailable; trying {fallback}: {error}"
                    );
                    match run_external_backend(fallback, None) {
                        Ok(()) => return,
                        Err(fallback_error) => eprintln!(
                            "info string {fallback} unavailable; using universal adapter: {fallback_error}"
                        ),
                    }
                } else {
                    eprintln!("{error}");
                    std::process::exit(2);
                }
            }
        }
    }

    match matches.subcommand() {
        Some(("uci", _)) => {
            commands::run_uci(backend);
        }
        Some(("xboard", _)) => {
            #[cfg(feature = "xboard")]
            commands::run_xboard();
            #[cfg(not(feature = "xboard"))]
            eprintln!("XBoard protocol support disabled (compile with --features xboard)");
        }
        Some(("play", sub)) => {
            let depth: i32 = sub.get_one::<String>("depth").unwrap().parse().unwrap_or(5);
            commands::run_play(depth);
        }
        Some(("analyze", sub)) => {
            let fen = sub.get_one::<String>("fen").unwrap();
            let depth: i32 = sub
                .get_one::<String>("depth")
                .unwrap()
                .parse()
                .unwrap_or(10);
            commands::run_analyze(fen, depth);
        }
        Some(("perft", sub)) => {
            let depth: u32 = sub.get_one::<String>("depth").unwrap().parse().unwrap_or(5);
            let fen = sub.get_one::<String>("fen").map(|s| s.as_str());
            commands::run_perft(depth, fen);
        }
        Some(("train", sub)) => {
            #[cfg(feature = "trainer")]
            match sub.subcommand() {
                Some(("emit-ateed", emit)) => {
                    let output = emit
                        .get_one::<String>("output")
                        .map_or("ateed_default.bin", String::as_str);
                    if let Err(error) =
                        trainer::ateed::emit_zero_network(std::path::Path::new(output))
                    {
                        eprintln!("failed to emit Ateed network: {error}");
                        std::process::exit(1);
                    }
                    println!("wrote Ateed zero network to {output}");
                }
                Some(("datagen", datagen)) => {
                    let config = trainer::config::DatagenConfig {
                        num_games: datagen
                            .get_one::<String>("games")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(1),
                        depth: datagen
                            .get_one::<String>("depth")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(6),
                        output_path: datagen
                            .get_one::<String>("output")
                            .cloned()
                            .unwrap_or_else(|| "data.txt".to_string()),
                        format: datagen
                            .get_one::<String>("format")
                            .cloned()
                            .unwrap_or_else(|| "text".to_string()),
                        ..Default::default()
                    };
                    if let Err(error) = trainer::datagen::generate_data(&config) {
                        eprintln!("datagen failed: {error}");
                        std::process::exit(1);
                    }
                }
                Some(("ateed", train)) => {
                    let config = trainer::config::TrainingConfig {
                        data_path: train
                            .get_one::<String>("data")
                            .cloned()
                            .unwrap_or_else(|| "data.txt".to_string()),
                        output_path: train
                            .get_one::<String>("output")
                            .cloned()
                            .unwrap_or_else(|| "ateed_default.bin".to_string()),
                        epochs: train
                            .get_one::<String>("epochs")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(8),
                        learning_rate: train
                            .get_one::<String>("lr")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(1.0),
                        wdl_weight: train
                            .get_one::<String>("wdl-weight")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(0.25),
                        base_network: train.get_one::<String>("base").cloned(),
                        mix_weights: train.get_one::<String>("mix").cloned().unwrap_or_default(),
                        mix_seed: train
                            .get_one::<String>("seed")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(1),
                        ..Default::default()
                    };
                    let scope = match trainer::train::AteedTrainScope::parse(
                        train
                            .get_one::<String>("scope")
                            .map_or("heads", String::as_str),
                    ) {
                        Ok(scope) => scope,
                        Err(error) => {
                            eprintln!("{error}");
                            std::process::exit(2);
                        }
                    };
                    if let Err(error) = trainer::train::train_ateed_to_file(&config, scope) {
                        eprintln!("Ateed training failed: {error}");
                        std::process::exit(1);
                    }
                    println!("wrote Ateed checkpoint to {}", config.output_path);
                }
                Some(("fetch", fetch)) => {
                    let id = fetch.get_one::<String>("id").map(String::as_str);
                    let url = fetch.get_one::<String>("url").map_or("", String::as_str);
                    let (resolved, filename) = match trainer::catalog::resolve_fetch_url(id, url) {
                        Ok(resolved) => resolved,
                        Err(error) => {
                            eprintln!("{error}");
                            std::process::exit(2);
                        }
                    };
                    let output = fetch
                        .get_one::<String>("output")
                        .cloned()
                        .unwrap_or(filename);
                    if let Err(error) =
                        trainer::dataset::fetch_dataset(&resolved, std::path::Path::new(&output))
                    {
                        eprintln!("dataset fetch failed: {error}");
                        std::process::exit(1);
                    }
                    println!("wrote dataset to {output}");
                }
                Some(("catalog", _)) => {
                    for offer in trainer::catalog::DATASETS {
                        println!(
                            "{}\t{}\t{}\t{}",
                            offer.id,
                            offer.kind.as_str(),
                            offer.filename,
                            offer.notes
                        );
                    }
                }
                Some(("decode", decode)) => {
                    let input = decode.get_one::<String>("input").map_or("", String::as_str);
                    let output = decode
                        .get_one::<String>("output")
                        .map_or("data.txt", String::as_str);
                    let format = match trainer::formats::DatasetFormat::parse(
                        decode
                            .get_one::<String>("format")
                            .map_or("text", String::as_str),
                    ) {
                        Ok(format) => format,
                        Err(error) => {
                            eprintln!("{error}");
                            std::process::exit(2);
                        }
                    };
                    let positions = match trainer::formats::load_positions_from_path(
                        std::path::Path::new(input),
                    ) {
                        Ok(positions) => positions,
                        Err(error) => {
                            eprintln!("decode failed: {error}");
                            std::process::exit(1);
                        }
                    };
                    if let Err(error) = trainer::formats::write_positions(
                        std::path::Path::new(output),
                        &positions,
                        format,
                    ) {
                        eprintln!("decode write failed: {error}");
                        std::process::exit(1);
                    }
                    println!("decoded {} positions to {output}", positions.len());
                }
                Some(("merge", merge)) => {
                    let data = merge.get_one::<String>("data").map_or("", String::as_str);
                    let mix = merge.get_one::<String>("mix").map_or("", String::as_str);
                    let seed = merge
                        .get_one::<String>("seed")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(1);
                    let output = merge
                        .get_one::<String>("output")
                        .map_or("mix.txt", String::as_str);
                    let format = match trainer::formats::DatasetFormat::parse(
                        merge
                            .get_one::<String>("format")
                            .map_or("text", String::as_str),
                    ) {
                        Ok(format) => format,
                        Err(error) => {
                            eprintln!("{error}");
                            std::process::exit(2);
                        }
                    };
                    let positions = match trainer::dataset::load_mixed_positions(data, mix, seed) {
                        Ok(positions) => positions,
                        Err(error) => {
                            eprintln!("merge failed: {error}");
                            std::process::exit(1);
                        }
                    };
                    if let Err(error) = trainer::formats::write_positions(
                        std::path::Path::new(output),
                        &positions,
                        format,
                    ) {
                        eprintln!("merge write failed: {error}");
                        std::process::exit(1);
                    }
                    println!("merged {} positions to {output}", positions.len());
                }
                _ => {
                    eprintln!(
                        "usage: mujrim train <emit-ateed|datagen|ateed|fetch|catalog|decode|merge> [options]"
                    );
                    std::process::exit(2);
                }
            }
            #[cfg(not(feature = "trainer"))]
            {
                let _ = sub;
                eprintln!("training support disabled (compile with --features trainer)");
            }
        }
        Some(("bench", sub)) => {
            let depth: i32 = sub
                .get_one::<String>("depth")
                .unwrap()
                .parse()
                .unwrap_or(10);
            let time_ms: Option<u64> = sub.get_one::<String>("time").and_then(|s| s.parse().ok());
            commands::run_bench(depth, time_ms);
        }
        _ => {
            commands::run_uci(backend);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EXTERNAL_BACKENDS, MUJRIM_ADAPTERS, fallback_engine_id, is_mujrim_adapter,
        passthrough_engine_id, product_adapter_from_exe_stem, search_stack_profile,
    };

    #[test]
    fn every_bundled_engine_is_available_as_a_backend() {
        assert_eq!(EXTERNAL_BACKENDS.len(), 10);
        for engine in mujrim_protocols::catalog::BUNDLED_ENGINES {
            assert!(
                matches!(
                    engine.0,
                    "mujrim-elite"
                        | "mujrim-external"
                        | "mujrim-v60"
                        | "mujrim-ak"
                        | "mujrim-viri"
                        | "mujrim-obs"
                        | "mujrim-plenty"
                        | "mujrim-ateed"
                        | "mujrim-lc0"
                        | "akimbo"
                ) || EXTERNAL_BACKENDS.contains(&engine.0)
            );
        }
    }

    #[test]
    fn uci_backends_prefer_mujrim_adapter_aliases() {
        assert_eq!(
            passthrough_engine_id("v60", true, false, false),
            Some("mujrim-v60")
        );
        assert_eq!(
            passthrough_engine_id("v10", true, false, false),
            Some("mujrim-elite")
        );
        assert_eq!(
            passthrough_engine_id("akimbo", true, false, false),
            Some("mujrim-ak")
        );
        assert_eq!(
            passthrough_engine_id("stockfish", true, false, false),
            Some("stockfish")
        );
        assert_eq!(
            passthrough_engine_id("reckless", true, false, false),
            Some("reckless")
        );
        assert_eq!(passthrough_engine_id("universal", true, false, false), None);
        assert_eq!(
            passthrough_engine_id("mujrim-hce", true, false, false),
            None
        );
        assert_eq!(passthrough_engine_id("v60", true, true, false), None);
        assert_eq!(
            passthrough_engine_id("viridithas", true, false, false),
            None
        );
        assert_eq!(passthrough_engine_id("obsidian", true, false, false), None);
        assert_eq!(
            passthrough_engine_id("plentychess", true, false, false),
            None
        );
        assert_eq!(
            passthrough_engine_id("plentychess", true, false, true),
            Some("plentychess")
        );
        assert_eq!(passthrough_engine_id("ateed", true, false, false), None);
        assert_eq!(
            passthrough_engine_id("ateed", true, false, true),
            Some("ateed")
        );
        assert_eq!(
            passthrough_engine_id("lc0", true, false, false),
            Some("lc0")
        );
        assert_eq!(
            passthrough_engine_id("viridithas", true, false, true),
            Some("viridithas")
        );
        assert_eq!(
            passthrough_engine_id("obsidian", true, false, true),
            Some("obsidian")
        );
        assert_eq!(
            product_adapter_from_exe_stem("mujrim-viri"),
            Some("viridithas")
        );
        assert_eq!(
            product_adapter_from_exe_stem("mujrim-obs"),
            Some("obsidian")
        );
        assert_eq!(
            product_adapter_from_exe_stem("mujrim-plenty"),
            Some("plentychess")
        );
        assert_eq!(product_adapter_from_exe_stem("mujrim-ateed"), Some("ateed"));
        assert_eq!(product_adapter_from_exe_stem("mujrim-lc0"), Some("lc0"));
        assert!(MUJRIM_ADAPTERS.iter().all(|id| is_mujrim_adapter(id)));
    }

    #[test]
    fn native_backend_alias_is_removed() {
        assert_eq!(passthrough_engine_id("native", true, false, false), None);
        assert_eq!(fallback_engine_id("native", false), None);
    }

    #[test]
    fn non_uci_commands_keep_the_in_process_implementation() {
        assert_eq!(passthrough_engine_id("v60", false, false, false), None);
        assert_eq!(
            passthrough_engine_id("stockfish", false, false, false),
            None
        );
    }

    #[test]
    fn packaged_default_falls_back_without_masking_explicit_path_errors() {
        assert_eq!(fallback_engine_id("stockfish", false), Some("mujrim-elite"));
        assert_eq!(fallback_engine_id("reckless", false), Some("mujrim-v60"));
        assert_eq!(fallback_engine_id("mujrim-hce", false), None);
        assert_eq!(fallback_engine_id("akimbo", false), None);
        assert_eq!(fallback_engine_id("stockfish", true), None);
    }

    #[test]
    fn default_uci_backend_is_in_process_universal() {
        assert_eq!(passthrough_engine_id("universal", true, false, false), None);
    }

    #[test]
    fn external_stack_profiles_keep_search_and_evaluation_paired() {
        let stockfish = search_stack_profile("stockfish");
        let reckless = search_stack_profile("reckless");

        assert_eq!(stockfish.engine_id, "stockfish");
        assert_eq!(stockfish.display_name, "Mujrim Elite 1.0.0");
        assert_eq!(reckless.engine_id, "reckless");
        assert_eq!(reckless.display_name, "Mujrim v60 1.0.0");
    }
}
