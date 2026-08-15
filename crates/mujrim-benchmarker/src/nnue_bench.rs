//! Focused, allocation-free timing for the active embedded NNUE network.

use std::hint::black_box;
use std::time::{Duration, Instant};

use eval::nnue::{ActiveNetwork, NNUEState, NnueNetworkSource, default_embedded_network};
use serde_json::{Value, json};
use types::{Board, Move};

use crate::suite::BK_POSITIONS;

#[derive(Clone, Copy, Debug)]
pub struct NnueBenchConfig {
    pub iterations: u64,
    pub warmup: u64,
}

#[derive(Clone, Debug)]
pub struct NnueBenchResult {
    pub network: String,
    pub iterations: u64,
    pub hot_elapsed: Duration,
    pub incremental_elapsed: Duration,
    pub suite_elapsed: Duration,
    pub checksum: i64,
}

impl NnueBenchResult {
    pub fn hot_evals_per_second(&self) -> f64 {
        rate(self.iterations, self.hot_elapsed)
    }

    pub fn suite_evals_per_second(&self) -> f64 {
        rate(self.iterations, self.suite_elapsed)
    }

    pub fn incremental_evals_per_second(&self) -> f64 {
        rate(self.iterations, self.incremental_elapsed)
    }

    pub fn hot_ns_per_eval(&self) -> f64 {
        nanos_per_eval(self.iterations, self.hot_elapsed)
    }

    pub fn suite_ns_per_eval(&self) -> f64 {
        nanos_per_eval(self.iterations, self.suite_elapsed)
    }

    pub fn incremental_ns_per_eval(&self) -> f64 {
        nanos_per_eval(self.iterations, self.incremental_elapsed)
    }

    pub fn to_json_value(&self) -> Value {
        json!({
            "type": "mujrim-nnue-benchmark",
            "network": self.network,
            "iterations": self.iterations,
            "hot": {
                "elapsed_ns": elapsed_nanos(self.hot_elapsed),
                "evals_per_second": self.hot_evals_per_second(),
                "ns_per_eval": self.hot_ns_per_eval(),
            },
            "suite": {
                "elapsed_ns": elapsed_nanos(self.suite_elapsed),
                "evals_per_second": self.suite_evals_per_second(),
                "ns_per_eval": self.suite_ns_per_eval(),
            },
            "incremental": {
                "elapsed_ns": elapsed_nanos(self.incremental_elapsed),
                "evals_per_second": self.incremental_evals_per_second(),
                "ns_per_eval": self.incremental_ns_per_eval(),
            },
            "checksum": self.checksum,
        })
    }
}

pub fn run(config: NnueBenchConfig) -> Result<NnueBenchResult, String> {
    run_with_network(config, default_embedded_network())
}

pub fn run_with_network(
    config: NnueBenchConfig,
    network: ActiveNetwork,
) -> Result<NnueBenchResult, String> {
    if config.iterations == 0 {
        return Err("iterations must be greater than zero".to_owned());
    }

    let network_name = network.info().name;
    let source: std::sync::Arc<ActiveNetwork> = std::sync::Arc::new(network);
    let mut hot_state = NNUEState::with_network(std::sync::Arc::clone(&source));
    let hot_board = Board::new();
    let mut checksum = warm_up(&mut hot_state, &hot_board, config.warmup);

    let hot_start = Instant::now();
    for _ in 0..config.iterations {
        checksum = checksum.wrapping_add(i64::from(black_box(
            hot_state.evaluate(black_box(&hot_board)),
        )));
    }
    let hot_elapsed = hot_start.elapsed();

    let line = build_move_line(16)?;
    let mut incremental_state = NNUEState::with_network(std::sync::Arc::clone(&source));
    let mut incremental_board = Board::new();
    checksum = checksum.wrapping_add(i64::from(incremental_state.evaluate(&incremental_board)));
    checksum = checksum.wrapping_add(replay_line(
        &mut incremental_state,
        &mut incremental_board,
        &line,
        config.warmup,
    ));
    let incremental_start = Instant::now();
    checksum = checksum.wrapping_add(replay_line(
        &mut incremental_state,
        &mut incremental_board,
        &line,
        config.iterations,
    ));
    let incremental_elapsed = incremental_start.elapsed();

    let boards = BK_POSITIONS
        .iter()
        .map(|(fen, _)| Board::from_fen(fen).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut suite_state = NNUEState::with_network(source);
    for index in 0..config.warmup {
        checksum = checksum.wrapping_add(i64::from(black_box(
            suite_state.evaluate(black_box(&boards[index as usize % boards.len()])),
        )));
    }

    let suite_start = Instant::now();
    for index in 0..config.iterations {
        checksum = checksum.wrapping_add(i64::from(black_box(
            suite_state.evaluate(black_box(&boards[index as usize % boards.len()])),
        )));
    }
    let suite_elapsed = suite_start.elapsed();

    Ok(NnueBenchResult {
        network: network_name,
        iterations: config.iterations,
        hot_elapsed,
        incremental_elapsed,
        suite_elapsed,
        checksum,
    })
}

fn build_move_line(max_plies: usize) -> Result<Vec<Move>, String> {
    let mut board = Board::new();
    let mut line = Vec::with_capacity(max_plies);
    for ply in 0..max_plies {
        let moves = board.generate_legal_moves();
        if moves.is_empty() {
            break;
        }
        let mv = moves[(ply * 7 + 3) % moves.len()];
        board.make_move(mv);
        line.push(mv);
    }
    if line.is_empty() {
        Err("could not generate an incremental benchmark line".to_owned())
    } else {
        Ok(line)
    }
}

fn replay_line(state: &mut NNUEState, board: &mut Board, line: &[Move], iterations: u64) -> i64 {
    let mut checksum = 0i64;
    let mut completed = 0u64;
    while completed < iterations {
        let remaining = (iterations - completed).min(line.len() as u64) as usize;
        for &mv in &line[..remaining] {
            state.push_move(board, mv);
            board.make_move(mv);
            checksum =
                checksum.wrapping_add(i64::from(black_box(state.evaluate(black_box(board)))));
            completed += 1;
        }
        for &mv in line[..remaining].iter().rev() {
            board.unmake_move(mv);
            state.pop_move();
        }
    }
    checksum
}

fn warm_up(state: &mut NNUEState, board: &Board, iterations: u64) -> i64 {
    let mut checksum = 0i64;
    for _ in 0..iterations {
        checksum = checksum.wrapping_add(i64::from(black_box(state.evaluate(black_box(board)))));
    }
    checksum
}

fn rate(iterations: u64, elapsed: Duration) -> f64 {
    iterations as f64 / elapsed.as_secs_f64()
}

fn nanos_per_eval(iterations: u64, elapsed: Duration) -> f64 {
    elapsed.as_nanos() as f64 / iterations as f64
}

fn elapsed_nanos(elapsed: Duration) -> u64 {
    elapsed.as_nanos().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_iterations() {
        let error = run(NnueBenchConfig {
            iterations: 0,
            warmup: 0,
        })
        .unwrap_err();
        assert_eq!(error, "iterations must be greater than zero");
    }

    #[test]
    fn focused_benchmark_reports_both_workloads() {
        let result = run(NnueBenchConfig {
            iterations: 8,
            warmup: 1,
        })
        .unwrap();
        assert_eq!(result.iterations, 8);
        assert!(result.hot_elapsed > Duration::ZERO);
        assert!(result.incremental_elapsed > Duration::ZERO);
        assert!(result.suite_elapsed > Duration::ZERO);
        assert!(result.hot_evals_per_second().is_finite());
        assert!(result.incremental_evals_per_second().is_finite());
        assert!(result.suite_evals_per_second().is_finite());
        assert_eq!(result.to_json_value()["type"], "mujrim-nnue-benchmark");
    }

    #[test]
    fn akimbo_embedded_incremental_workload_completes() {
        let result = run_with_network(
            NnueBenchConfig {
                iterations: 8,
                warmup: 1,
            },
            ActiveNetwork::Embedded,
        )
        .unwrap();
        assert_eq!(result.network, "Embedded Akimbo 1024");
        assert!(result.incremental_elapsed > Duration::ZERO);
        assert!(result.incremental_ns_per_eval().is_finite());
    }
}
