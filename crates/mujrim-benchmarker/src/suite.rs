//! Benchmark Suites — position collections and result types.
//!
//! Provides the Bratko-Kopec test suite (24 positions) and support
//! for loading custom FEN files.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use mujrim_bench_ratings::{
    approx_ccrl_40_15_from_bk_accuracy, approx_lichess_blitz_from_bk_accuracy,
};

// ═══════════════════════════════════════════════════════════════════
// Bratko-Kopec Test Suite
// ═══════════════════════════════════════════════════════════════════

/// A benchmark test position.
#[derive(Clone, Debug)]
pub struct TestPosition {
    /// FEN string.
    pub fen: String,
    /// Expected best move in UCI notation.
    pub expected_move: String,
}

impl TestPosition {
    /// Whether `found_move` is one of the accepted UCI solutions.
    /// Multiple accepted moves are separated by `|` in the suite data.
    pub fn matches_expected(&self, found_move: &str) -> bool {
        self.expected_move
            .split('|')
            .any(|expected| !expected.is_empty() && expected == found_move)
    }
}

/// Bratko-Kopec test suite: 24 positions of varying tactical difficulty.
pub const BK_POSITIONS: &[(&str, &str)] = &[
    (
        "1k1r4/pp1b1R2/3q2pp/4p3/2B5/4Q3/PPP2B2/2K5 b - - 0 1",
        "d6d1",
    ),
    (
        "3r1k2/4npp1/1ppr3p/p6P/P2PPPP1/1NR5/5K2/2R5 w - - 0 1",
        "d4d5",
    ),
    (
        "2q1rr1k/3bbnnp/p2p1pp1/2pPp3/PpP1P1P1/1P2BNNP/2BQ1PRK/7R b - - 0 1",
        "f6f5|f8g8",
    ),
    (
        "rnbqkb1r/p3pppp/1p6/2ppP3/3N4/2P5/PPP1QPPP/R1B1KB1R w KQkq - 0 1",
        "e5e6",
    ),
    (
        "r1b2rk1/2q1b1pp/p2ppn2/1p6/3QP3/1BN1B3/PPP3PP/R4RK1 w - - 0 1",
        "c3d5|a2a4",
    ),
    (
        "2r3k1/pppR1pp1/4p3/4P1P1/5P2/1P4K1/P1P5/8 w - - 0 1",
        "g5g6",
    ),
    (
        "1nk1r1r1/pp2n1pp/4p3/q2pPp1N/b1pP1P2/B1P2R2/2P1B1PP/R2Q2K1 w - - 0 1",
        "h5f6|a3b4",
    ),
    ("4b3/p3kp2/6p1/3pP2p/2pP1P2/4K1P1/P3N2P/8 w - - 0 1", "f4f5"),
    (
        "2kr1bnr/pbpq4/2n1pp2/3p3p/3P1P1B/2N2N1Q/PPP3PP/2KR1B1R w - - 0 1",
        "f4f5|c1b1|d1e1",
    ),
    (
        "3rr1k1/pp3pp1/1qn2np1/8/3p4/PP1R1P2/2P1NQPP/R1B3K1 b - - 0 1",
        "c6e5",
    ),
    (
        "2r1nrk1/p2q1ppp/bp1p4/n1pPp3/P1P1P3/2PBB1N1/4QPPP/R4RK1 w - - 0 1",
        "f2f4|g3f5",
    ),
    (
        "r3r1k1/ppqb1ppp/8/4p1NQ/8/2P5/PP3PPP/R3R1K1 b - - 0 1",
        "d7f5",
    ),
    (
        "r2q1rk1/4bppp/p2p4/2pP4/3pP3/3Q4/PP1B1PPP/R3R1K1 w - - 0 1",
        "b2b4",
    ),
    (
        "rnb2r1k/pp2p2p/2pp2p1/q2P1p2/8/1Pb2NP1/PB2PPBP/R2Q1RK1 w - - 0 1",
        "d1d2|d1e1",
    ),
    (
        "2r3k1/1p2q1pp/2b1pr2/p1pp4/6Q1/1P1PP1R1/P1PN2PP/5RK1 w - - 0 1",
        "g4g7",
    ),
    (
        "r1bqkb1r/4npp1/p1p4p/1p1pP1B1/8/1B6/PPPN1PPP/R2Q1RK1 w kq - 0 1",
        "d2e4",
    ),
    (
        "r2q1rk1/1ppnbppp/p2p1nb1/3Pp3/2P1P1P1/2N2N1P/PPB1QP2/R1B2RK1 b - - 0 1",
        "g6h5|c7c6",
    ),
    (
        "r1bq1rk1/pp2ppbp/2np2p1/2n5/P3PP2/N1P2N2/1PB3PP/R1B1QRK1 b - - 0 1",
        "c5b3",
    ),
    (
        "3rr3/2pq2pk/p2p1pnp/8/2QBPP2/1P6/P5PP/4RRK1 b - - 0 1",
        "e8e4",
    ),
    (
        "r4k2/pb2bp1r/1p1qp2p/3pNp2/3P1P2/2N3P1/PPP1Q2P/2KRR3 w - - 0 1",
        "g3g4",
    ),
    (
        "3rn2k/ppb2rpp/2ppqp2/5N2/2P1P3/1P5Q/PB3PPP/3RR1K1 w - - 0 1",
        "f5h6",
    ),
    (
        "2r2rk1/1bqnbpp1/1p1ppn1p/pP6/N1P1P3/P2B1N1P/1B2QPP1/R2R2K1 b - - 0 1",
        "b7e4",
    ),
    (
        "r1bqk2r/pp2bppp/2p5/3pP3/P2Q1P2/2N1B3/1PP3PP/R4RK1 b kq - 0 1",
        "f7f6",
    ),
    (
        "r2qnrnk/p2b2b1/1p1p2pp/2pPpp2/1PP1P3/PRNBB3/3QNPPP/5RK1 w - - 0 1",
        "f2f4",
    ),
];

/// Load the Bratko-Kopec suite as `TestPosition` structs.
pub fn bk_suite() -> Vec<TestPosition> {
    BK_POSITIONS
        .iter()
        .map(|(fen, mv)| TestPosition {
            fen: (*fen).to_string(),
            expected_move: (*mv).to_string(),
        })
        .collect()
}

/// Load custom positions from a FEN file.
///
/// File format: one line per position. Each line is either:
/// - `FEN` (no expected move)
/// - `FEN;expected_move` (with expected move in UCI notation)
///
/// Empty lines and lines starting with `#` are ignored.
pub fn load_custom_positions(path: &Path) -> Result<Vec<TestPosition>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read '{}': {e}", path.display()))?;

    let positions: Vec<TestPosition> = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|line| {
            if let Some((fen, mv)) = line.split_once(';') {
                TestPosition {
                    fen: fen.trim().to_string(),
                    expected_move: mv.trim().to_string(),
                }
            } else {
                TestPosition {
                    fen: line.to_string(),
                    expected_move: String::new(),
                }
            }
        })
        .collect();

    if positions.is_empty() {
        return Err(format!("No positions found in '{}'", path.display()));
    }

    Ok(positions)
}

// ═══════════════════════════════════════════════════════════════════
// Results
// ═══════════════════════════════════════════════════════════════════

/// Result of benchmarking a single position.
#[derive(Clone, Debug)]
pub struct PositionResult {
    pub index: usize,
    pub fen: String,
    pub expected_move: String,
    pub found_move: String,
    pub correct: bool,
    pub score: i32,
    pub depth: i32,
    pub nodes: u64,
    pub nps: u64,
    pub elapsed: Duration,
}

/// A position the engine could not complete.
#[derive(Clone, Debug)]
pub struct PositionFailure {
    pub index: usize,
    pub fen: String,
    pub error: String,
    pub elapsed: Duration,
}

/// Aggregated benchmark summary.
#[derive(Clone, Debug)]
pub struct BenchSummary {
    pub engine_name: String,
    pub results: Vec<PositionResult>,
    pub failures: Vec<PositionFailure>,
    pub total_nodes: u64,
    pub total_time: Duration,
    pub nps: u64,
    pub correct: usize,
    pub total: usize,
    pub accuracy: f64,
    /// Proxy for **CCRL 40/15**-style strength (BK accuracy; not an official list rating).
    pub approx_ccrl_40_15: i32,
    /// Rough **Lichess blitz–pool** analogue (offset from CCRL proxy).
    pub approx_lichess_blitz: i32,
}

impl BenchSummary {
    /// Compute summary from position results.
    pub fn from_results(engine_name: &str, results: Vec<PositionResult>) -> Self {
        Self::from_results_and_failures(engine_name, results, Vec::new())
    }

    /// Compute a summary while retaining positions that failed to complete.
    pub fn from_results_and_failures(
        engine_name: &str,
        results: Vec<PositionResult>,
        failures: Vec<PositionFailure>,
    ) -> Self {
        let total = results.len() + failures.len();
        let correct = results.iter().filter(|r| r.correct).count();
        let total_nodes: u64 = results.iter().map(|r| r.nodes).sum();
        let total_time: Duration = results.iter().map(|r| r.elapsed).sum();
        let nps = rate_per_second(total_nodes, total_time);
        let accuracy = if total > 0 {
            (correct as f64) / (total as f64) * 100.0
        } else {
            0.0
        };

        Self {
            engine_name: engine_name.to_string(),
            results,
            failures,
            total_nodes,
            total_time,
            nps,
            correct,
            total,
            accuracy,
            approx_ccrl_40_15: approx_ccrl_40_15_from_bk_accuracy(accuracy),
            approx_lichess_blitz: approx_lichess_blitz_from_bk_accuracy(accuracy),
        }
    }
}

impl BenchSummary {
    /// Machine-readable summary for agents / `iterate --json` pipelines.
    pub fn to_json_value(&self) -> serde_json::Value {
        use serde_json::json;

        let results: Vec<serde_json::Value> = self
            .results
            .iter()
            .map(|r| {
                json!({
                    "index": r.index,
                    "correct": r.correct,
                    "expected_move": r.expected_move,
                    "found_move": r.found_move,
                    "score_cp": r.score,
                    "depth": r.depth,
                    "nodes": r.nodes,
                    "nps": r.nps,
                    "elapsed_ms": r.elapsed.as_millis(),
                    "elapsed_ns": r.elapsed.as_nanos(),
                })
            })
            .collect();
        let failures: Vec<serde_json::Value> = self
            .failures
            .iter()
            .map(|failure| {
                json!({
                    "index": failure.index,
                    "fen": failure.fen,
                    "error": failure.error,
                    "elapsed_ms": failure.elapsed.as_millis(),
                    "elapsed_ns": failure.elapsed.as_nanos(),
                })
            })
            .collect();

        json!({
            "engine_name": self.engine_name,
            "strength_number_bk_proxy": self.approx_ccrl_40_15,
            "correct": self.correct,
            "total": self.total,
            "accuracy": self.accuracy,
            "approx_ccrl_40_15": self.approx_ccrl_40_15,
            "approx_lichess_blitz": self.approx_lichess_blitz,
            "nps_aggregate": self.nps,
            "total_nodes": self.total_nodes,
            "total_time_ms": self.total_time.as_millis(),
            "total_time_ns": self.total_time.as_nanos(),
            "results": results,
            "failures": failures,
        })
    }
}

impl fmt::Display for BenchSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Strength number (BK suite proxy): {} — run the same command again to track progress; not official CCRL Elo.",
            self.approx_ccrl_40_15
        )?;
        writeln!(f)?;
        writeln!(f, "╔══════════════════════════════════════════════╗")?;
        writeln!(f, "║                  RESULTS                    ║")?;
        writeln!(f, "╠══════════════════════════════════════════════╣")?;
        writeln!(f, "║  Engine:      {:<31}║", self.engine_name)?;
        writeln!(
            f,
            "║  Accuracy:    {:>2}/{:<2} ({:>5.1}%)                ║",
            self.correct, self.total, self.accuracy
        )?;
        writeln!(
            f,
            "║  Approx. CCRL 40/15:   ~{:<24}║",
            self.approx_ccrl_40_15
        )?;
        writeln!(
            f,
            "║  Approx. Lichess blitz: ~{:<22}║",
            self.approx_lichess_blitz
        )?;
        writeln!(f, "║  NPS:         {:<31}║", format_nps(self.nps))?;
        writeln!(f, "║  Total nodes: {:<31}║", format_nps(self.total_nodes))?;
        writeln!(
            f,
            "║  Total time:  {:<31}║",
            format!("{}ms", self.total_time.as_millis())
        )?;
        if !self.failures.is_empty() {
            writeln!(f, "    Failures:    {}", self.failures.len())?;
        }
        writeln!(f, "╚══════════════════════════════════════════════╝")?;
        Ok(())
    }
}

/// Compute an event rate without discarding sub-millisecond timing precision.
pub fn rate_per_second(count: u64, elapsed: Duration) -> u64 {
    let nanos = elapsed.as_nanos();
    if nanos == 0 {
        return count;
    }
    (u128::from(count).saturating_mul(1_000_000_000) / nanos).min(u128::from(u64::MAX)) as u64
}

/// Formats a number with human-readable suffixes (K, M, B).
pub fn format_nps(n: u64) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bk_suite_count() {
        let suite = bk_suite();
        assert_eq!(suite.len(), 24);
    }

    #[test]
    fn test_bk_positions_have_expected_moves() {
        for pos in bk_suite() {
            assert!(
                !pos.expected_move.is_empty(),
                "BK position should have expected move: {}",
                pos.fen
            );
        }
    }

    #[test]
    fn bk_positions_and_solutions_are_legal() {
        types::init();
        for (index, pos) in bk_suite().into_iter().enumerate() {
            let mut board = types::Board::from_fen(&pos.fen).unwrap_or_else(|error| {
                panic!("BK position {} has invalid FEN: {error}", index + 1)
            });
            assert!(
                board.all_occupancy().count_ones() <= 32,
                "BK position {} contains more than 32 pieces",
                index + 1
            );
            let legal = board.generate_legal_moves();
            for expected in pos.expected_move.split('|') {
                let expected = types::Move::from_uci(expected)
                    .unwrap_or_else(|| panic!("BK position {} has invalid solution", index + 1));
                assert!(
                    legal.as_slice().iter().any(|candidate| {
                        candidate.from == expected.from
                            && candidate.to == expected.to
                            && candidate.promotion == expected.promotion
                    }),
                    "BK position {} has illegal solution {}",
                    index + 1,
                    expected.to_uci()
                );
            }
        }
    }

    #[test]
    fn alternate_bk_solutions_are_accepted() {
        let position = &bk_suite()[4];
        assert!(position.matches_expected("c3d5"));
        assert!(position.matches_expected("a2a4"));
        assert!(!position.matches_expected("e2e4"));
    }

    #[test]
    fn current_oracle_alternatives_preserve_legacy_bk_solutions() {
        let suite = bk_suite();
        for (index, legacy, alternative) in [
            (2, "f6f5", "f8g8"),
            (6, "h5f6", "a3b4"),
            (8, "f4f5", "c1b1"),
            (8, "f4f5", "d1e1"),
            (10, "f2f4", "g3f5"),
            (16, "g6h5", "c7c6"),
        ] {
            assert!(suite[index].matches_expected(legacy));
            assert!(suite[index].matches_expected(alternative));
        }
    }

    #[test]
    fn test_ccrl_lichess_from_bk_accuracy() {
        assert!(approx_ccrl_40_15_from_bk_accuracy(0.0) >= 800);
        assert!(approx_ccrl_40_15_from_bk_accuracy(50.0) >= 1600);
        assert_eq!(approx_ccrl_40_15_from_bk_accuracy(100.0), 3500);
        assert_eq!(approx_ccrl_40_15_from_bk_accuracy(90.0), 2500);
        let plan = approx_ccrl_40_15_from_bk_accuracy(54.166666666666664);
        assert!((plan - 1963).abs() <= 1);
        assert_eq!(approx_lichess_blitz_from_bk_accuracy(100.0), 3500 + 115);
    }

    #[test]
    fn test_format_nps() {
        assert_eq!(format_nps(500), "500");
        assert_eq!(format_nps(1500), "1.5K");
        assert_eq!(format_nps(1_500_000), "1.50M");
        assert_eq!(format_nps(1_500_000_000), "1.50B");
    }

    #[test]
    fn rate_preserves_nanosecond_precision() {
        assert_eq!(
            rate_per_second(1_000, Duration::from_micros(500)),
            2_000_000
        );
        assert_eq!(rate_per_second(17, Duration::ZERO), 17);
    }

    #[test]
    fn test_bench_summary() {
        let results = vec![PositionResult {
            index: 0,
            fen: "startpos".into(),
            expected_move: "e2e4".into(),
            found_move: "e2e4".into(),
            correct: true,
            score: 50,
            depth: 10,
            nodes: 100_000,
            nps: 1_000_000,
            elapsed: Duration::from_millis(100),
        }];
        let summary = BenchSummary::from_results("test", results);
        assert_eq!(summary.correct, 1);
        assert_eq!(summary.total, 1);
        assert!((summary.accuracy - 100.0).abs() < 0.01);
    }

    #[test]
    fn failed_positions_count_against_accuracy_and_are_serialized() {
        let failure = PositionFailure {
            index: 3,
            fen: "broken-position".into(),
            error: "engine exited".into(),
            elapsed: Duration::from_millis(12),
        };
        let summary = BenchSummary::from_results_and_failures("test", Vec::new(), vec![failure]);

        assert_eq!(summary.total, 1);
        assert_eq!(summary.correct, 0);
        assert_eq!(summary.accuracy, 0.0);
        assert_eq!(summary.failures.len(), 1);
        assert_eq!(summary.to_json_value()["failures"][0]["index"], 3);
    }
}
