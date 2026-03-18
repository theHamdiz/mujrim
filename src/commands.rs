//! CLI command implementations.

use std::io::{self, Write};
use std::time::Instant;

use comms::UciHandler;
#[cfg(feature = "xboard")]
use comms::XBoardHandler;
use search::SearchEngine;
use types::{Board, Color};

/// Runs the UCI protocol loop.
pub fn run_uci() {
    let mut handler = UciHandler::new();
    handler.run();
}

/// Runs the XBoard/CECP protocol loop.
#[cfg(feature = "xboard")]
pub fn run_xboard() {
    let mut handler = XBoardHandler::new();
    handler.run();
}

/// Interactive play mode against the engine.
pub fn run_play(depth: i32) {
    let mut engine = SearchEngine::new(64, 8);
    let mut board = Board::new();

    println!("╔══════════════════════════════════════╗");
    println!("║   KishMat Chess Engine v2.0.0        ║");
    println!("║   You play as White                  ║");
    println!("║   Enter moves in UCI format (e2e4)   ║");
    println!("║   Type 'quit' to exit                ║");
    println!("╚══════════════════════════════════════╝");
    println!("{board}");

    loop {
        if board.is_game_over() {
            if board.is_checkmate() {
                let winner = if board.side_to_move == Color::White { "Black" } else { "White" };
                println!("Checkmate! {winner} wins!");
            } else {
                println!("Game over — draw!");
            }
            break;
        }

        if board.side_to_move == Color::White {
            // Human's turn
            print!("\n  Your move: ");
            io::stdout().flush().ok();

            let mut input = String::new();
            if io::stdin().read_line(&mut input).is_err() {
                break;
            }
            let input = input.trim();

            if input == "quit" || input == "exit" {
                break;
            }

            // Parse and validate the move
            let legal_moves = board.generate_legal_moves();
            let parsed = legal_moves.iter().find(|m| m.to_uci() == input);

            match parsed {
                Some(mv) => {
                    board.make_move(*mv);
                    println!("{board}");
                }
                None => {
                    println!("  Illegal move! Legal moves: {}",
                        legal_moves.iter().map(|m| m.to_uci()).collect::<Vec<_>>().join(", "));
                    continue;
                }
            }
        } else {
            // Engine's turn
            println!("\n  Engine is thinking...");
            let start = Instant::now();
            let result = engine.search_depth(&mut board, depth);
            let elapsed = start.elapsed();
            let best_move = result.best_move;

            println!("  Engine plays: {} ({:.1}s)", best_move.to_uci(), elapsed.as_secs_f64());
            board.make_move(best_move);
            println!("{board}");
        }
    }
}

/// Analyze a FEN position.
pub fn run_analyze(fen: &str, depth: i32) {
    let mut board = match Board::from_fen(fen) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Invalid FEN: {e}");
            return;
        }
    };

    println!("Analyzing position:");
    println!("{board}");

    let mut engine = SearchEngine::new(64, 8);
    let result = engine.search_depth(&mut board, depth);

    println!("\nBest move: {}", result.best_move.to_uci());
    println!("Evaluation: {} cp", result.score);
}

/// Run perft test.
pub fn run_perft(depth: u32, fen: Option<&str>) {
    let mut board = if let Some(fen) = fen {
        Board::from_fen(fen).unwrap_or_else(|e| {
            eprintln!("Invalid FEN: {e}");
            std::process::exit(1);
        })
    } else {
        Board::new()
    };

    println!("Perft test:");
    println!("{board}");

    for d in 1..=depth {
        let start = Instant::now();
        let nodes = board.perft(d);
        let elapsed = start.elapsed();
        let nps = if elapsed.as_secs() > 0 {
            nodes / elapsed.as_secs()
        } else {
            nodes
        };
        println!("  Perft({d}) = {nodes:>12} nodes  ({:.3}s, ~{nps} nps)", elapsed.as_secs_f64());
    }
}

/// Bratko-Kopec test suite: (FEN, best_move_uci).
const BK_POSITIONS: &[(&str, &str)] = &[
    ("1k1r4/pp1b1R2/3q2pp/4p3/2B5/4Q3/PPP2B2/2K5 b - - 0 1", "d6d1"),
    ("3r1k2/4npp1/1ppr3p/p6P/P2PPPP1/1NR5/5K2/2R5 w - - 0 1", "d4d5"),
    ("2q1rr1k/3bbnnp/p2p1pp1/2pPp3/PpP1P1P1/1P2BNNP/2BQ1PRK/7R b - - 0 1", "f6f5"),
    ("rnbqkb1r/p3pppp/1p6/2ppP3/3N4/2P5/PPP1QPPP/R1B1KB1R w KQkq - 0 1", "e5e6"),
    ("r1b2rk1/2q1b1pp/p2ppn2/1p6/3QP3/1BN1B3/PPP3PP/R4RK1 w - - 0 1", "a2a4"),
    ("2r3k1/pppR1pp1/4p3/4P1P1/5P2/1P4K1/P1P5/8 w - - 0 1", "g5g6"),
    ("1nk1r1r1/pp2n1pp/4p3/q2pPp1N/b1pP1P2/B1P2R2/2P1B1PP/R2Q2K1 w - - 0 1", "h5f6"),
    ("4b3/p3kp2/6p1/3pP2p/2pP1P2/2P1K1P1/P7/8 w - - 0 1", "f4f5"),
    ("2kr1bnr/pbpq4/2n1pp2/3p3p/3P1P1B/2N2N1P/PPP3P1/2KR1B1R w - - 0 1", "f4f5"),
    ("3rr1k1/pp3pp1/1qn2np1/8/3p4/PP1R1P2/2P1NQPP/R1B3K1 b - - 0 1", "c6e5"),
    ("2r1nrk1/p2q1ppp/bp1p4/n1pPp3/P1P1P3/2PBB1N1/4QPPP/R4RK1 w - - 0 1", "f2f4"),
    ("r3r1k1/ppqb1ppp/8/4p1NQ/8/2P5/PP3PPP/R3R1K1 w - - 0 1", "g5f7"),
    ("r2q1rk1/4bppp/p2p4/2pP4/3pP3/3Q4/PP1B1PPP/R3R1K1 w - - 0 1", "b2b4"),
    ("rnb2r1k/pp2p2p/2pp2p1/q2P1p2/8/1Pb2NP1/PB2PPBP/R2Q1RK1 w - - 0 1", "d1d2"),
    ("2r3k1/1p2q1pp/2b1pr2/p1pp4/6Q1/1P1PP3/P1RB1PPP/1K3B1R w - - 0 1", "g4g7"),
    ("r1bqkb1r/4npp1/p1p4p/1p1pP1B1/8/1B6/PPPN1PPP/R2Q1RK1 w kq - 0 1", "d2e4"),
    ("r2q1rk1/1ppnbppp/p2p1nb1/3Pp3/2P1P1p1/2N2N1P/PPB1QPP1/R1BR2K1 b - - 0 1", "g6h5"),
    ("r1bq1rk1/pp2ppbp/2np2p1/2n5/P3PP2/3pBN2/1PP1B1PP/RN1Q1RK1 b - - 0 1", "c5b3"),
    ("3rr3/2pq2pk/p2p1pnp/8/2QBPP2/1P6/P5PP/4RRK1 b - - 0 1", "e8e4"),
    ("r4k2/pb2bp1r/1p1qp2p/3pNp2/3P1P2/2N3P1/PPP1Q2P/2KRR3 w - - 0 1", "g3g4"),
    ("3rn2k/ppb2rpp/2ppqp2/5N2/2P1P3/1P5Q/PB3PPP/3RR1K1 w - - 0 1", "f5h6"),
    ("2r2rk1/1bqnbpp1/1p1ppn1p/pP6/N1P1P3/P2B1N1P/1B2QPP1/R2R2K1 b - - 0 1", "b7e4"),
    ("r1bqk2r/pp2bppp/2p5/3pP3/P2Q1P2/2N1B3/1PP3PP/R4RK1 b kq - 0 1", "f7f6"),
    ("r2qnrnk/p2b2b1/1p1p2pp/2pPpp2/1PP1P3/PRNBB3/3QNPPP/5RK1 w - - 0 1", "f2f4"),
];

/// Formats a number with human-readable suffixes (K, M, B).
fn format_nps(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.2}B", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{n}")
    }
}

/// Run the ELO estimation benchmark.
pub fn run_bench(depth: i32, time_ms: Option<u64>) {
    use search::SearchEngine;
    use std::time::Duration;

    println!("╔══════════════════════════════════════════════╗");
    println!("║       KishMat Benchmark Suite v2.0.0        ║");
    println!("║       Bratko-Kopec Test (24 positions)      ║");
    println!("╚══════════════════════════════════════════════╝");
    println!();

    // ── Hardware Detection ──────────────────────────────────────
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    // Reserve 2 cores for the OS / background tasks, but always use at least 1
    let num_threads = cpu_cores.saturating_sub(2).max(1);

    println!("  Hardware Detection:");

    // CPU info
    #[cfg(target_arch = "aarch64")]
    println!("    CPU arch:   aarch64 (ARM64)");
    #[cfg(target_arch = "x86_64")]
    println!("    CPU arch:   x86_64");
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    println!("    CPU arch:   {}", std::env::consts::ARCH);

    println!("    CPU cores:  {} (using {} for bench)", cpu_cores, num_threads);

    // SIMD features (compile-time detection via target-cpu=native)
    let mut simd_features = Vec::new();
    #[cfg(target_arch = "aarch64")]
    {
        simd_features.push("NEON");
        #[cfg(target_feature = "dotprod")]
        simd_features.push("DotProd");
        #[cfg(target_feature = "fp16")]
        simd_features.push("FP16");
        #[cfg(target_feature = "crc")]
        simd_features.push("CRC32");
        #[cfg(target_feature = "aes")]
        simd_features.push("AES");
        #[cfg(target_feature = "sha2")]
        simd_features.push("SHA2");
    }
    #[cfg(target_arch = "x86_64")]
    {
        #[cfg(target_feature = "avx2")]
        simd_features.push("AVX2");
        #[cfg(target_feature = "avx")]
        simd_features.push("AVX");
        #[cfg(target_feature = "sse4.2")]
        simd_features.push("SSE4.2");
        #[cfg(target_feature = "sse4.1")]
        simd_features.push("SSE4.1");
        #[cfg(target_feature = "popcnt")]
        simd_features.push("POPCNT");
        #[cfg(target_feature = "bmi2")]
        simd_features.push("BMI2");
    }
    if simd_features.is_empty() {
        println!("    SIMD:       (none detected at compile time)");
    } else {
        println!("    SIMD:       {}", simd_features.join(", "));
    }

    // GPU detection
    #[cfg(target_os = "macos")]
    {
        let gpu_info = std::process::Command::new("system_profiler")
            .args(["SPDisplaysDataType"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();

        let metal_support = gpu_info.lines()
            .find(|l| l.contains("Metal Support"))
            .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
            .unwrap_or_else(|| "Not detected".to_string());
        let gpu_model = gpu_info.lines()
            .find(|l| l.contains("Chipset Model"))
            .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
            .unwrap_or_else(|| "Unknown".to_string());

        println!("    GPU:        {} ({})", gpu_model, metal_support);

        // NPU (Apple Neural Engine) — present on Apple Silicon
        #[cfg(target_arch = "aarch64")]
        println!("    NPU:        Apple Neural Engine (available via CoreML)");
    }
    #[cfg(target_os = "linux")]
    {
        let gpu_detected = detect_linux_gpu();
        if gpu_detected.is_empty() {
            println!("    GPU:        N/A (not detected)");
        } else {
            println!("    GPU:        {}", gpu_detected);
        }
        println!("    NPU:        N/A");
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        println!("    GPU:        N/A (no detection for this platform)");
        println!("    NPU:        N/A");
    }

    // Note about compute usage
    println!();
    println!("    Threads:  {} (Lazy SMP)", num_threads);
    println!("    Engine:   CPU SIMD only (no GPU acceleration)");

    // Bench hash: 4GB to match UCI default and fully utilize available RAM
    let hash_mb = 4096;
    println!("    Hash:     {}MB", hash_mb);
    println!();

    let mut engine = SearchEngine::new(hash_mb, num_threads);
    let mut correct = 0;
    let mut total_positions = 0;
    let mut total_nodes = 0u64;
    let mut total_time_ms = 0u64;

    for (i, (fen, expected)) in BK_POSITIONS.iter().enumerate() {
        let mut board = match Board::from_fen(fen) {
            Ok(b) => b,
            Err(e) => {
                println!("  [{:>2}] SKIP -- bad FEN: {e}", i + 1);
                continue;
            }
        };

        total_positions += 1;
        engine.clear();

        let result = if let Some(ms) = time_ms {
            engine.search_time(&mut board, Duration::from_millis(ms), 64)
        } else {
            // Use search_time with a generous 5-minute per-position limit.
            // This ensures SMP threads have a time-based stop signal while
            // giving enough time to reach depth 14-18+ on most positions.
            engine.search_time(&mut board, Duration::from_secs(300), depth)
        };

        let found = result.best_move.to_uci();
        let ok = found == *expected;
        if ok { correct += 1; }
        total_nodes += result.nodes;
        total_time_ms += result.elapsed.as_millis() as u64;

        let marker = if ok { "OK" } else { "--" };
        let pos_nps = if result.elapsed.as_millis() > 0 {
            result.nodes * 1000 / result.elapsed.as_millis() as u64
        } else {
            result.nodes
        };
        println!(
            "  [{:>2}] {marker} found={:<6} expected={:<6} score={:>6}cp  {} NPS ({:.0}ms)",
            i + 1, found, expected, result.score, format_nps(pos_nps),
            result.elapsed.as_millis()
        );
    }

    println!();

    // NPS measurement on startpos — time-limited to 5 seconds so it never hangs
    let mut startpos = Board::new();
    engine.clear();
    let nps_result = engine.search_time(&mut startpos, Duration::from_secs(5), 64);
    let nps = if nps_result.elapsed.as_millis() > 0 {
        nps_result.nodes * 1000 / nps_result.elapsed.as_millis() as u64
    } else {
        nps_result.nodes
    };

    let accuracy = (correct as f64) / (total_positions as f64) * 100.0;
    let elo = estimate_elo(accuracy);

    println!("╔══════════════════════════════════════════════╗");
    println!("║                  RESULTS                    ║");
    println!("╠══════════════════════════════════════════════╣");
    println!("║  Accuracy:    {:>2}/{:<2} ({:>5.1}%)                ║", correct, total_positions, accuracy);
    println!("║  Est. ELO:    ~{:<30}║", elo);
    println!("║  NPS:         {:<31}║", format!("{} (5s, startpos)", format_nps(nps)));
    println!("║  Total nodes: {:<31}║", format_nps(total_nodes));
    println!("║  Total time:  {:<31}║", format!("{total_time_ms}ms"));
    println!("╚══════════════════════════════════════════════╝");
}

/// Maps BK accuracy (0–100) to an approximate ELO rating.
fn estimate_elo(accuracy: f64) -> i32 {
    // Piecewise linear approximation based on engine testing data
    let elo = if accuracy <= 10.0 {
        800.0 + accuracy * 40.0
    } else if accuracy <= 30.0 {
        1200.0 + (accuracy - 10.0) * 20.0
    } else if accuracy <= 50.0 {
        1600.0 + (accuracy - 30.0) * 15.0
    } else if accuracy <= 70.0 {
        1900.0 + (accuracy - 50.0) * 15.0
    } else if accuracy <= 90.0 {
        2200.0 + (accuracy - 70.0) * 15.0
    } else {
        2500.0 + (accuracy - 90.0) * 30.0
    };
    elo.round() as i32
}

/// Detect GPU on Linux using `lspci` (works for AMD, NVIDIA, Intel).
#[cfg(target_os = "linux")]
fn detect_linux_gpu() -> String {
    // Try lspci first (most reliable)
    if let Ok(output) = std::process::Command::new("lspci").output() {
        if let Ok(stdout) = String::from_utf8(output.stdout) {
            let mut gpus = Vec::new();
            for line in stdout.lines() {
                let lower = line.to_lowercase();
                if lower.contains("vga") || lower.contains("3d") || lower.contains("display") {
                    // Extract the device description (everything after the first ': ')
                    if let Some(desc) = line.split(": ").nth(1) {
                        gpus.push(desc.trim().to_string());
                    }
                }
            }
            if !gpus.is_empty() {
                return gpus.join("; ");
            }
        }
    }

    // Fallback: check /sys/class/drm
    if let Ok(entries) = std::fs::read_dir("/sys/class/drm") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("card") && !name.contains('-') {
                let vendor_path = entry.path().join("device/vendor");
                if let Ok(vendor) = std::fs::read_to_string(&vendor_path) {
                    let vendor = vendor.trim();
                    let vendor_name = match vendor {
                        "0x1002" => "AMD/ATI",
                        "0x10de" => "NVIDIA",
                        "0x8086" => "Intel",
                        _ => vendor,
                    };
                    return format!("{} (via sysfs)", vendor_name);
                }
            }
        }
    }

    String::new()
}
