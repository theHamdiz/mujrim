//! External Benchmark Runner — benchmarks UCI/XBoard-compatible engines.

use std::path::PathBuf;
use std::time::Instant;

use kishmat_protocols::{EngineOptions, EngineSession, ProtocolKind, SearchRequest};

use crate::suite::{BenchSummary, PositionResult, TestPosition, format_nps};

/// Configuration for an external benchmark run.
#[derive(Clone, Debug)]
pub struct ExternalBenchConfig {
    pub engine_path: PathBuf,
    pub engine_args: Vec<String>,
    pub protocol: ProtocolKind,
    pub depth: i32,
    pub hash_mb: usize,
    pub threads: usize,
    pub time_per_position: std::time::Duration,
}

impl Default for ExternalBenchConfig {
    fn default() -> Self {
        Self {
            engine_path: PathBuf::from("./kishmat"),
            engine_args: Vec::new(),
            protocol: ProtocolKind::Uci,
            depth: 16,
            hash_mb: 128,
            threads: 1,
            time_per_position: std::time::Duration::from_secs(30),
        }
    }
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

    let mut session =
        EngineSession::spawn_with_args(&config.engine_path, &config.engine_args, config.protocol)?;
    session.configure(&EngineOptions {
        hash_mb: Some(config.hash_mb),
        threads: Some(config.threads),
    })?;

    let engine_name = config
        .engine_path
        .file_name()
        .map_or("unknown".to_string(), |n| n.to_string_lossy().to_string());

    println!("Engine: {engine_name} ({})", config.protocol);
    println!();

    let mut results = Vec::with_capacity(positions.len());

    for (i, pos) in positions.iter().enumerate() {
        let start = Instant::now();
        let info = session.search(&SearchRequest {
            fen: pos.fen.clone(),
            depth: config.depth,
            movetime: Some(config.time_per_position),
            node_limit: None,
        })?;
        let elapsed = start.elapsed();

        let correct = !pos.expected_move.is_empty() && info.best_move == pos.expected_move;
        let nps = if elapsed.as_millis() > 0 {
            info.nodes * 1000 / elapsed.as_millis() as u64
        } else {
            info.nodes
        };

        let pos_result = PositionResult {
            index: i,
            fen: pos.fen.clone(),
            expected_move: pos.expected_move.clone(),
            found_move: info.best_move.clone(),
            correct,
            score: info.score,
            depth: info.depth,
            nodes: info.nodes,
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

        results.push(pos_result);
    }

    Ok(BenchSummary::from_results(&engine_name, results))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_external_config_defaults_to_uci() {
        let cfg = ExternalBenchConfig::default();
        assert_eq!(cfg.protocol, ProtocolKind::Uci);
        assert_eq!(cfg.depth, 16);
        assert!(cfg.engine_args.is_empty());
    }
}
