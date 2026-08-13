//! Classical HCE throughput microbench (UseNNUE=false).
//!
//! Release hosts can target ~[`RELEASE_HCE_NPS_TARGET`] aggregate nodes/sec with
//! Lazy SMP + rayon-backed leaf eval. CI uses small node budgets so the suite
//! stays under a second.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rayon::prelude::*;
use types::Board;

use crate::engine::{SearchEngine, SearchLimits};

/// Aggregate NPS goal for release benches on high-core x86_64 hosts.
pub const RELEASE_HCE_NPS_TARGET: u64 = 100_000_000;

/// Default CI node budget (far below the release target).
pub const CI_HCE_NODE_BUDGET: u64 = 50_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HceNpsReport {
    pub nodes: u64,
    pub elapsed: Duration,
    pub nps: u64,
    pub threads: usize,
    pub use_nnue: bool,
}

impl HceNpsReport {
    fn from_nodes(nodes: u64, elapsed: Duration, threads: usize, use_nnue: bool) -> Self {
        let secs = elapsed.as_secs_f64().max(1e-9);
        let nps = (nodes as f64 / secs).round() as u64;
        Self {
            nodes,
            elapsed,
            nps,
            threads,
            use_nnue,
        }
    }
}

/// Time-limited Lazy SMP search with classical evaluation.
///
/// Prefer this for release NPS measurements. Helpers stay enabled because the
/// limit is wall-clock rather than a hard node cap.
pub fn measure_hce_search_nps(threads: usize, time: Duration, hash_mb: usize) -> HceNpsReport {
    let mut engine = SearchEngine::new(hash_mb.max(1), threads.max(1));
    engine.set_use_nnue(false);
    let mut board = Board::new();
    let start = Instant::now();
    let result = engine.search_time(&mut board, time, 64);
    HceNpsReport::from_nodes(result.nodes, start.elapsed(), threads.max(1), false)
}

/// Rayon-backed leaf-eval node counter for classical HCE.
///
/// Walks a shallow legal tree and evaluates every leaf with `eval::evaluate`.
/// Work is partitioned across root moves so CI can keep budgets tiny while
/// still exercising the multicore path that release benches scale up.
pub fn measure_hce_eval_nodes(threads: usize, node_budget: u64) -> HceNpsReport {
    let threads = threads.max(1);
    let node_budget = node_budget.max(1);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("HCE rayon pool");

    let mut board = Board::new();
    let root_moves: Vec<_> = board.generate_legal_moves().iter().copied().collect();
    let counter = AtomicU64::new(0);
    let start = Instant::now();

    pool.install(|| {
        root_moves.par_iter().for_each(|mv| {
            if counter.load(Ordering::Relaxed) >= node_budget {
                return;
            }
            let mut child = board.clone();
            child.make_move(*mv);
            walk_hce_nodes(&mut child, 2, node_budget, &counter);
        });
    });

    let nodes = counter.load(Ordering::Relaxed).min(node_budget);
    HceNpsReport::from_nodes(nodes, start.elapsed(), threads, false)
}

/// Node-limited Lazy SMP search with classical eval (helpers forced on).
pub fn measure_hce_search_nodes(
    threads: usize,
    nodes: u64,
    hash_mb: usize,
    max_depth: i32,
) -> HceNpsReport {
    let mut engine = SearchEngine::new(hash_mb.max(1), threads.max(1));
    engine.set_use_nnue(false);
    let mut board = Board::new();
    let start = Instant::now();
    let result = engine.search(
        &mut board,
        SearchLimits {
            max_depth,
            time_limit: None,
            node_limit: Some(nodes.max(1)),
            stopped: false,
            use_soft_time: false,
            force_helpers: true,
        },
    );
    HceNpsReport::from_nodes(result.nodes, start.elapsed(), threads.max(1), false)
}

fn walk_hce_nodes(board: &mut Board, depth: i32, budget: u64, counter: &AtomicU64) {
    if counter.fetch_add(1, Ordering::Relaxed) >= budget {
        return;
    }
    // Touch classical eval on every node so NPS reflects UseNNUE=false cost.
    let _ = eval::evaluate(board);
    if depth <= 0 {
        return;
    }
    let moves = board.generate_legal_moves();
    for mv in moves.iter() {
        if counter.load(Ordering::Relaxed) >= budget {
            break;
        }
        board.make_move(*mv);
        walk_hce_nodes(board, depth - 1, budget, counter);
        board.unmake_move(*mv);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() {
        types::init();
    }

    #[test]
    fn hce_eval_microbench_stays_within_ci_budget() {
        setup();
        let report = measure_hce_eval_nodes(2, CI_HCE_NODE_BUDGET);
        assert!(!report.use_nnue);
        assert!(report.nodes > 0);
        assert!(report.nodes <= CI_HCE_NODE_BUDGET);
        assert_eq!(report.threads, 2);
        assert!(report.nps > 0);
    }

    #[test]
    fn hce_search_respects_use_nnue_false_and_small_node_cap() {
        setup();
        let report = measure_hce_search_nodes(2, 2_000, 4, 8);
        assert!(!report.use_nnue);
        assert!(report.nodes > 0);
        // Lazy SMP node limits are soft; helpers may overshoot modestly.
        assert!(report.nodes < 50_000);
        assert_eq!(report.threads, 2);
    }

    #[test]
    fn release_nps_target_is_documented() {
        assert_eq!(RELEASE_HCE_NPS_TARGET, 100_000_000);
    }
}
