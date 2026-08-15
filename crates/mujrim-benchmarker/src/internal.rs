//! Internal Benchmark Runner — runs Mujrim's own search engine.
//!
//! Uses the `SearchEngine` from `mujrim-search` directly, with
//! shared TT and Lazy SMP threads. This is the "dog-fooding" benchmark.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eval::nnue::load_network;
use search::engine::SearchEngine;
use types::Board;

use crate::suite::{BenchSummary, PositionResult, TestPosition, format_nps, rate_per_second};

/// Configuration for an internal benchmark run.
#[derive(Clone, Debug)]
pub struct InternalBenchConfig {
    pub depth: i32,
    pub threads: usize,
    pub hash_mb: usize,
    pub time_per_position: Duration,
    pub suite_name: String,
    /// Runtime NNUE preset (`auto`, `akimbo`, `stockfish`, `reckless`).
    pub eval_preset: String,
    /// Optional runtime network file path.
    pub eval_file: Option<PathBuf>,
    /// Suppress per-position `println` (for JSON / agent pipelines).
    pub quiet: bool,
}

impl Default for InternalBenchConfig {
    fn default() -> Self {
        Self {
            depth: 20,
            threads: 1,
            hash_mb: 256,
            time_per_position: Duration::from_secs(90),
            suite_name: "Bratko-Kopec".to_string(),
            eval_preset: "auto".to_string(),
            eval_file: None,
            quiet: false,
        }
    }
}

/// Callback for live progress reporting.
pub type ProgressCallback = Box<dyn Fn(usize, usize, &PositionResult) + Send>;

/// Install the evaluator + search stack that belong together.
///
/// Explicit EvalPreset names always install the matching adapter (network +
/// search). A bare `set_params_for_preset("stockfish")` on top of Reckless is
/// the historical NPS / strength mismatch.
fn configure_engine_eval(
    engine: &mut SearchEngine,
    eval_preset: &str,
    eval_file: Option<&std::path::Path>,
    quiet: bool,
) {
    if matches!(eval_preset, "mujrim-hce" | "hce") {
        let _ = search::install_adapter(engine, "mujrim-hce");
        return;
    }

    if let Some(path) = eval_file {
        match load_network(path) {
            Ok(network) => engine.set_nnue_network(network),
            Err(e) => eprintln!(
                "info string EvalFile load failed for '{}': {e} (using embedded net)",
                path.display()
            ),
        }
    } else if matches!(
        eval_preset,
        "stockfish"
            | "reckless"
            | "akimbo"
            | "viridithas"
            | "obsidian"
            | "plentychess"
            | "ateed"
            | "lc0"
    ) {
        let _ = search::install_adapter(engine, eval_preset);
        return;
    } else if let Some(network) = eval::nnue::embedded_network_for_preset(eval_preset) {
        engine.set_nnue_network(network);
    } else {
        let (auto_net, msg) = eval::nnue::auto_detect_from_search_roots();
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
}

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
    configure_engine_eval(
        &mut engine,
        &config.eval_preset,
        config.eval_file.as_deref(),
        config.quiet,
    );

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
        let correct = pos.matches_expected(&found_move);
        let nps = rate_per_second(result.nodes, elapsed);

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

    BenchSummary::from_results("Mujrim", results)
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
    configure_engine_eval(&mut engine, eval_preset, eval_file, quiet);
    let mut board = Board::new();
    let result = engine.search_time(&mut board, Duration::from_secs(5), 64);
    rate_per_second(result.nodes, result.elapsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::bk_suite;

    #[test]
    fn default_bench_config_favors_stable_bk_scores() {
        let c = InternalBenchConfig::default();
        assert_eq!(c.threads, 1);
        assert_eq!(c.depth, 20);
        assert_eq!(c.time_per_position.as_secs(), 90);
    }

    #[test]
    fn viridithas_and_obsidian_presets_install_matching_search_stacks() {
        let mut viri = SearchEngine::new(4, 1);
        configure_engine_eval(&mut viri, "viridithas", None, true);
        assert_eq!(
            viri.network_profile(),
            Some(eval::nnue::NnueSearchProfile::Viridithas)
        );
        let mut obs = SearchEngine::new(4, 1);
        configure_engine_eval(&mut obs, "obsidian", None, true);
        assert_eq!(
            obs.network_profile(),
            Some(eval::nnue::NnueSearchProfile::Obsidian)
        );
    }

    #[test]
    fn viridithas_preset_installs_sandhi_when_the_file_is_present() {
        if eval::nnue::discover_named_network("sandhi-s2-b200.nnue.zst").is_none()
            && eval::nnue::discover_named_network("viri_default.nnue.zst").is_none()
        {
            return;
        }
        let mut viri = SearchEngine::new(4, 1);
        configure_engine_eval(&mut viri, "viridithas", None, true);
        let info = viri.nnue_info();
        assert_eq!(info.format, eval::nnue::NetworkFormat::Viridithas);
        assert!(
            info.architecture.contains("sandhi"),
            "viridithas preset must install sandhi, got {}",
            info.architecture
        );
    }

    #[test]
    fn stockfish_eval_preset_installs_stockfish_network_and_params() {
        let mut engine = SearchEngine::new(4, 1);
        configure_engine_eval(&mut engine, "stockfish", None, true);
        assert_eq!(engine.params().nmp_base, 5);
        assert_eq!(
            engine.nnue_info().format,
            eval::nnue::NetworkFormat::Stockfish
        );
        assert_eq!(
            engine.network_profile(),
            Some(eval::nnue::NnueSearchProfile::Stockfish)
        );
    }

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
            eval_file: Some(PathBuf::from("/nonexistent/mujrim/net.bin")),
            quiet: true,
        };
        let summary = run_internal_bench(&positions, &config, None);
        assert_eq!(summary.total, 1);
    }
}
