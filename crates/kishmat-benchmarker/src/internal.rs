//! Internal Benchmark Runner — runs KishMat's own search engine.
//!
//! Uses the `SearchEngine` from `kishmat-search` directly, with
//! shared TT and Lazy SMP threads. This is the "dog-fooding" benchmark.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eval::nnue::load_network;
use search::engine::SearchEngine;
use types::Board;

use crate::suite::{BenchSummary, PositionResult, TestPosition, format_nps};

/// Configuration for an internal benchmark run.
#[derive(Clone, Debug)]
pub struct InternalBenchConfig {
    pub depth: i32,
    pub threads: usize,
    pub hash_mb: usize,
    pub time_per_position: Duration,
    pub suite_name: String,
    /// Runtime NNUE preset (`auto`, `akimbo`, `stockfish`).
    pub eval_preset: String,
    /// Optional runtime network file path.
    pub eval_file: Option<PathBuf>,
    /// Suppress per-position `println` (for JSON / agent pipelines).
    pub quiet: bool,
}

impl Default for InternalBenchConfig {
    fn default() -> Self {
        Self {
            depth: 16,
            threads: std::thread::available_parallelism()
                .map(|n| n.get().saturating_sub(2).max(1))
                .unwrap_or(1),
            hash_mb: 128,
            time_per_position: Duration::from_secs(30),
            suite_name: "Bratko-Kopec".to_string(),
            eval_preset: "auto".to_string(),
            eval_file: None,
            quiet: false,
        }
    }
}

/// Callback for live progress reporting.
pub type ProgressCallback = Box<dyn Fn(usize, usize, &PositionResult) + Send>;

/// Run an internal benchmark against the given positions.
///
/// Returns a `BenchSummary` with per-position results. If `on_progress` is
/// provided, it is called after each position completes.
pub fn run_internal_bench(
    positions: &[TestPosition],
    config: &InternalBenchConfig,
    on_progress: Option<ProgressCallback>,
) -> BenchSummary {
    let mut engine = SearchEngine::new(config.hash_mb, config.threads);
    if let Some(path) = &config.eval_file {
        // Explicit --eval-file: load that specific network.
        match load_network(path) {
            Ok(network) => {
                engine.set_nnue_network(network);
            }
            Err(e) => {
                eprintln!(
                    "info string EvalFile load failed for '{}': {e} (using embedded net)",
                    path.display()
                );
            }
        }
    } else {
        // No --eval-file: auto-detect the strongest available net.
        let nnue_dir = std::path::Path::new("nnue");
        let (auto_net, msg) = eval::nnue::auto_detect_network(nnue_dir);
        if let Some(net) = auto_net {
            if !config.quiet {
                eprintln!("{msg}");
            }
            engine.set_nnue_network(net);
        }
    }
    // If the user explicitly requested a preset, override the auto-applied one.
    if config.eval_preset != "auto" {
        engine.set_params_for_preset(&config.eval_preset);
    }

    let mut results = Vec::with_capacity(positions.len());

    for (i, pos) in positions.iter().enumerate() {
        // Parse FEN
        let mut board = match Board::from_fen(&pos.fen) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[{}] SKIP bad FEN: {} — {}", i + 1, pos.fen, e);
                continue;
            }
        };

        // Clear TT between positions for isolation
        engine.clear();

        let start = Instant::now();
        let result = engine.search_time_hard(&mut board, config.time_per_position, config.depth);
        let elapsed = start.elapsed();

        let found_move = result.best_move.to_uci();
        let correct = !pos.expected_move.is_empty() && found_move == pos.expected_move;
        let nps = if elapsed.as_millis() > 0 {
            result.nodes * 1000 / elapsed.as_millis() as u64
        } else {
            result.nodes
        };

        let pos_result = PositionResult {
            index: i,
            fen: pos.fen.clone(),
            expected_move: pos.expected_move.clone(),
            found_move: found_move.clone(),
            correct,
            score: result.score,
            depth: result.depth,
            nodes: result.nodes,
            nps,
            elapsed,
        };

        // Print inline progress
        let status = if pos.expected_move.is_empty() {
            "  ".to_string()
        } else if correct {
            "OK".to_string()
        } else {
            "--".to_string()
        };
        if !config.quiet {
            println!(
                "[{:>2}] {} found={:<8} expected={:<8} score={:>5}cp  {} NPS ({}ms)",
                i + 1,
                status,
                found_move,
                if pos.expected_move.is_empty() {
                    "N/A"
                } else {
                    &pos.expected_move
                },
                result.score,
                format_nps(nps),
                elapsed.as_millis(),
            );
        }

        if let Some(ref cb) = on_progress {
            cb(i, positions.len(), &pos_result);
        }

        results.push(pos_result);
    }

    BenchSummary::from_results("KishMat", results)
}

/// NPS from the starting position using a short wall-clock search (same setup as CLI `info` NPS line).
pub fn measure_startpos_nps(
    threads: usize,
    hash_mb: usize,
    eval_preset: &str,
    eval_file: Option<&std::path::Path>,
    quiet: bool,
) -> u64 {
    use search::engine::SearchEngine;
    use types::Board;

    let mut engine = SearchEngine::new(hash_mb, threads);
    if let Some(path) = eval_file {
        if let Ok(network) = load_network(path) {
            engine.set_nnue_network(network);
        }
    } else {
        let nnue_dir = std::path::Path::new("nnue");
        let (auto_net, msg) = eval::nnue::auto_detect_network(nnue_dir);
        if let Some(net) = auto_net {
            if !quiet {
                eprintln!("{msg}");
            }
            engine.set_nnue_network(net);
        }
    }
    if eval_preset != "auto" {
        engine.set_params_for_preset(eval_preset);
    }
    let mut board = Board::new();
    let result = engine.search_time(&mut board, Duration::from_secs(5), 64);
    if result.elapsed.as_millis() > 0 {
        result.nodes * 1000 / result.elapsed.as_millis() as u64
    } else {
        result.nodes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::bk_suite;

    #[test]
    fn test_internal_bench_runs_one_position() {
        let positions = vec![bk_suite()[0].clone()];
        let config = InternalBenchConfig {
            depth: 4,
            threads: 1,
            hash_mb: 4,
            time_per_position: Duration::from_secs(5),
            suite_name: "Test".into(),
            eval_preset: "auto".into(),
            eval_file: None,
            quiet: true,
        };
        let summary = run_internal_bench(&positions, &config, None);
        assert_eq!(summary.total, 1);
        assert!(summary.total_nodes > 0);
    }

    #[test]
    fn test_internal_bench_invalid_eval_file_falls_back_to_embedded() {
        let positions = vec![bk_suite()[0].clone()];
        let config = InternalBenchConfig {
            depth: 3,
            threads: 1,
            hash_mb: 4,
            time_per_position: Duration::from_secs(2),
            suite_name: "Test".into(),
            eval_preset: "auto".into(),
            eval_file: Some(PathBuf::from("/nonexistent/kishmat/net.bin")),
            quiet: true,
        };
        let summary = run_internal_bench(&positions, &config, None);
        assert_eq!(summary.total, 1);
    }
}
