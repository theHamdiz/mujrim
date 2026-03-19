//! Internal Benchmark Runner — runs KishMat's own search engine.
//!
//! Uses the `SearchEngine` from `kishmat-search` directly, with
//! shared TT and Lazy SMP threads. This is the "dog-fooding" benchmark.

use std::time::{Duration, Instant};

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
}

impl Default for InternalBenchConfig {
    fn default() -> Self {
        Self {
            depth: 16,
            threads: std::thread::available_parallelism()
                .map(|n| n.get().saturating_sub(2).max(1))
                .unwrap_or(1),
            hash_mb: 128,
            time_per_position: Duration::from_secs(120),
            suite_name: "Bratko-Kopec".to_string(),
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
        let result = engine.search_time(&mut board, config.time_per_position, config.depth);
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

        if let Some(ref cb) = on_progress {
            cb(i, positions.len(), &pos_result);
        }

        results.push(pos_result);
    }

    BenchSummary::from_results("KishMat", results)
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
        };
        let summary = run_internal_bench(&positions, &config, None);
        assert_eq!(summary.total, 1);
        assert!(summary.total_nodes > 0);
    }
}
