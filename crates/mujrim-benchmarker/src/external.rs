//! External Benchmark Runner — benchmarks UCI/XBoard-compatible engines.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mujrim_protocols::{EngineOptions, EngineSession, ProtocolKind, SearchRequest};

use crate::suite::{
    BenchSummary, PositionFailure, PositionResult, TestPosition, format_nps, rate_per_second,
};

const MINIMUM_SEARCH_READ_TIMEOUT: Duration = Duration::from_secs(20);
const MINIMUM_SEARCH_TIMEOUT_MARGIN: Duration = Duration::from_secs(5);
const FIXED_NODE_READ_TIMEOUT: Duration = Duration::from_secs(300);
const MIB: u64 = 1024 * 1024;

fn search_read_timeout(search_budget: Duration) -> Duration {
    let margin = std::cmp::max(search_budget / 4, MINIMUM_SEARCH_TIMEOUT_MARGIN);
    std::cmp::max(
        search_budget.saturating_add(margin),
        MINIMUM_SEARCH_READ_TIMEOUT,
    )
}

fn benchmark_read_timeout(config: &ExternalBenchConfig) -> Duration {
    config
        .time_per_position
        .filter(|_| config.node_limit.is_none())
        .map_or(FIXED_NODE_READ_TIMEOUT, search_read_timeout)
}

#[inline]
fn measured_nodes(last_reported_nodes: u64, fixed_node_budget: Option<u64>) -> u64 {
    fixed_node_budget.unwrap_or(last_reported_nodes)
}

/// Configuration for an external benchmark run.
#[derive(Clone, Debug)]
pub struct ExternalBenchConfig {
    pub engine_path: PathBuf,
    pub engine_args: Vec<String>,
    pub protocol: ProtocolKind,
    pub depth: i32,
    pub hash_mb: usize,
    pub threads: usize,
    pub memory_limit_mb: usize,
    pub uci_options: Vec<(String, String)>,
    pub node_limit: Option<u64>,
    pub time_per_position: Option<std::time::Duration>,
    pub quiet: bool,
}

impl Default for ExternalBenchConfig {
    fn default() -> Self {
        Self {
            engine_path: PathBuf::from("./mujrim"),
            engine_args: Vec::new(),
            protocol: ProtocolKind::Uci,
            depth: 16,
            hash_mb: 128,
            threads: 1,
            memory_limit_mb: 256,
            uci_options: Vec::new(),
            node_limit: None,
            time_per_position: None,
            quiet: false,
        }
    }
}

fn spawn_configured_session(config: &ExternalBenchConfig) -> Result<EngineSession, String> {
    let minimum_limit = config.hash_mb.saturating_add(64);
    if config.memory_limit_mb < minimum_limit {
        return Err(format!(
            "engine memory limit must be at least {minimum_limit} MiB for a {} MiB hash",
            config.hash_mb
        ));
    }

    let memory_limit_bytes = (config.memory_limit_mb as u64).saturating_mul(MIB);
    let mut session = EngineSession::spawn_with_args_and_memory_limit(
        &config.engine_path,
        &config.engine_args,
        config.protocol,
        Some(memory_limit_bytes),
    )?;
    session.set_read_timeout(benchmark_read_timeout(config));
    session.configure(&EngineOptions {
        hash_mb: Some(config.hash_mb),
        threads: Some(config.threads),
        own_book: Some(false),
        custom: config.uci_options.clone(),
    })?;
    Ok(session)
}

/// Analyze one exact game position in a fresh, resource-bounded engine session.
pub fn run_external_search(
    fen: &str,
    moves: &[String],
    config: &ExternalBenchConfig,
) -> Result<mujrim_protocols::SearchInfo, String> {
    if !config.engine_path.exists() {
        return Err(format!(
            "Engine binary not found: {}",
            config.engine_path.display()
        ));
    }
    if config.node_limit == Some(0) {
        return Err("node limit must be greater than zero".to_owned());
    }

    let mut session = spawn_configured_session(config)?;
    session.new_game()?;
    session.search(&SearchRequest {
        fen: fen.to_owned(),
        moves: moves.to_vec(),
        depth: config.depth,
        movetime: config.time_per_position,
        node_limit: config.node_limit,
        clock: None,
    })
}

/// Run an external benchmark against the given positions.
pub fn run_external_bench(
    positions: &[TestPosition],
    config: &ExternalBenchConfig,
) -> Result<BenchSummary, String> {
    if !config.engine_path.exists() {
        return Err(format!(
            "Engine binary not found: {}",
            config.engine_path.display()
        ));
    }

    let mut session = Some(spawn_configured_session(config)?);

    let engine_name = config
        .engine_path
        .file_name()
        .map_or("unknown".to_string(), |n| n.to_string_lossy().to_string());

    if !config.quiet {
        println!("Engine: {engine_name} ({})", config.protocol);
        println!();
    }

    let mut results = Vec::with_capacity(positions.len());
    let mut failures = Vec::new();

    for (i, pos) in positions.iter().enumerate() {
        let start = Instant::now();
        let fixed_nodes = config.node_limit;
        let active_session = session.as_mut().expect("session is present");
        let search_result = active_session.new_game().and_then(|()| {
            active_session.search(&SearchRequest {
                fen: pos.fen.clone(),
                moves: Vec::new(),
                depth: if fixed_nodes.is_some() {
                    i32::MAX
                } else {
                    config.depth
                },
                movetime: fixed_nodes
                    .is_none()
                    .then_some(config.time_per_position)
                    .flatten(),
                node_limit: fixed_nodes,
                clock: None,
            })
        });
        let elapsed = start.elapsed();
        let info = match search_result {
            Ok(info) => info,
            Err(error) => {
                if !config.quiet {
                    println!("[{:>2}] ERR {} ({}ms)", i + 1, error, elapsed.as_millis());
                }
                failures.push(PositionFailure {
                    index: i,
                    fen: pos.fen.clone(),
                    error,
                    elapsed,
                });

                drop(session.take());
                if i + 1 < positions.len() {
                    match spawn_configured_session(config) {
                        Ok(restarted) => session = Some(restarted),
                        Err(restart_error) => {
                            for (remaining_index, remaining) in
                                positions.iter().enumerate().skip(i + 1)
                            {
                                failures.push(PositionFailure {
                                    index: remaining_index,
                                    fen: remaining.fen.clone(),
                                    error: format!("engine restart failed: {restart_error}"),
                                    elapsed: Duration::ZERO,
                                });
                            }
                            break;
                        }
                    }
                }
                continue;
            }
        };
        let nodes = measured_nodes(info.nodes, fixed_nodes);

        let correct = pos.matches_expected(&info.best_move);
        let nps = rate_per_second(nodes, elapsed);

        let pos_result = PositionResult {
            index: i,
            fen: pos.fen.clone(),
            expected_move: pos.expected_move.clone(),
            found_move: info.best_move.clone(),
            correct,
            score: info.score,
            depth: info.depth,
            nodes,
            nps,
            elapsed,
        };

        let status = if pos.expected_move.is_empty() {
            "  "
        } else if correct {
            "OK"
        } else {
            "--"
        };
        if !config.quiet {
            println!(
                "[{:>2}] {} found={:<8} expected={:<8} score={:>5}cp  {} NPS ({}ms)",
                i + 1,
                status,
                info.best_move,
                if pos.expected_move.is_empty() {
                    "N/A"
                } else {
                    &pos.expected_move
                },
                info.score,
                format_nps(nps),
                elapsed.as_millis(),
            );
        }

        results.push(pos_result);
    }

    Ok(BenchSummary::from_results_and_failures(
        &engine_name,
        results,
        failures,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Write};

    #[test]
    fn test_external_config_defaults_to_uci() {
        let cfg = ExternalBenchConfig::default();
        assert_eq!(cfg.protocol, ProtocolKind::Uci);
        assert_eq!(cfg.depth, 16);
        assert!(cfg.engine_args.is_empty());
        assert!(cfg.uci_options.is_empty());
        assert!(cfg.node_limit.is_none());
        assert!(cfg.time_per_position.is_none());
        assert_eq!(cfg.memory_limit_mb, 256);
        assert!(!cfg.quiet);
    }

    #[test]
    fn search_timeout_scales_beyond_requested_budget() {
        assert_eq!(
            search_read_timeout(Duration::from_secs(30)),
            Duration::from_millis(37_500)
        );
        assert_eq!(
            search_read_timeout(Duration::from_secs(1)),
            MINIMUM_SEARCH_READ_TIMEOUT
        );

        let cfg = ExternalBenchConfig {
            node_limit: Some(1_000_000),
            ..ExternalBenchConfig::default()
        };
        assert_eq!(benchmark_read_timeout(&cfg), FIXED_NODE_READ_TIMEOUT);
        assert_eq!(measured_nodes(421_337, Some(500_000)), 500_000);
        assert_eq!(measured_nodes(421_337, None), 421_337);
    }

    #[test]
    fn external_benchmark_restarts_after_a_position_crash() {
        let test_name = "external::tests::mock_uci_engine_child";
        let config = ExternalBenchConfig {
            engine_path: std::env::current_exe().unwrap(),
            engine_args: vec!["--exact".into(), test_name.into(), "--nocapture".into()],
            hash_mb: 1,
            memory_limit_mb: 128,
            node_limit: Some(10),
            ..ExternalBenchConfig::default()
        };
        let positions = [
            TestPosition {
                fen: "ok-one".into(),
                expected_move: "e2e4".into(),
            },
            TestPosition {
                fen: "crash".into(),
                expected_move: "e2e4".into(),
            },
            TestPosition {
                fen: "ok-two".into(),
                expected_move: "e2e4".into(),
            },
        ];

        let summary = run_external_bench(&positions, &config).unwrap();

        assert_eq!(summary.correct, 2);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.results.len(), 2);
        assert_eq!(summary.failures.len(), 1);
        assert_eq!(summary.failures[0].index, 1);
        assert!(summary.failures[0].error.contains("engine closed stdout"));
    }

    #[test]
    fn external_search_preserves_the_full_move_history() {
        let test_name = "external::tests::mock_uci_engine_child";
        let config = ExternalBenchConfig {
            engine_path: std::env::current_exe().unwrap(),
            engine_args: vec!["--exact".into(), test_name.into(), "--nocapture".into()],
            hash_mb: 1,
            memory_limit_mb: 128,
            node_limit: Some(10),
            ..ExternalBenchConfig::default()
        };

        let info = run_external_search(
            "moves-fixture",
            &["e2e4".to_owned(), "e7e5".to_owned()],
            &config,
        )
        .unwrap();

        assert_eq!(info.best_move, "d2d4");
        assert_eq!(info.nodes, 10);
    }

    #[test]
    fn mock_uci_engine_child() {
        let arguments = std::env::args().collect::<Vec<_>>();
        let is_child = arguments
            .windows(2)
            .any(|pair| pair[0] == "--exact" && pair[1].ends_with("mock_uci_engine_child"));
        if !is_child {
            return;
        }

        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout().lock();
        let mut should_crash = false;
        let mut replay_history_seen = false;
        let mut own_book_disabled = false;
        let mut new_game_seen = false;
        for line in stdin.lock().lines() {
            let line = line.unwrap();
            if line == "uci" {
                writeln!(stdout, "id name Mujrim benchmark crash fixture").unwrap();
                writeln!(stdout, "uciok").unwrap();
                stdout.flush().unwrap();
            } else if line == "isready" {
                writeln!(stdout, "readyok").unwrap();
                stdout.flush().unwrap();
            } else if line == "setoption name OwnBook value false" {
                own_book_disabled = true;
            } else if line == "ucinewgame" {
                new_game_seen = true;
            } else if let Some(fen) = line.strip_prefix("position fen ") {
                should_crash = fen == "crash";
                replay_history_seen = fen == "moves-fixture moves e2e4 e7e5";
            } else if line.starts_with("go ") {
                if should_crash || !new_game_seen {
                    std::process::exit(23);
                }
                new_game_seen = false;
                writeln!(stdout, "info depth 1 score cp 0 nodes 10").unwrap();
                let best_move = if replay_history_seen && own_book_disabled {
                    "d2d4"
                } else {
                    "e2e4"
                };
                writeln!(stdout, "bestmove {best_move}").unwrap();
                stdout.flush().unwrap();
            } else if line == "quit" {
                break;
            }
        }
    }
}
