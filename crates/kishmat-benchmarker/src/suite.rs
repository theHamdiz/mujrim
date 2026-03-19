//! Benchmark Suites — position collections and result types.
//!
//! Provides the Bratko-Kopec test suite (24 positions) and support
//! for loading custom FEN files.

use std::fmt;
use std::path::Path;
use std::time::Duration;

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
        "f6f5",
    ),
    (
        "rnbqkb1r/p3pppp/1p6/2ppP3/3N4/2P5/PPP1QPPP/R1B1KB1R w KQkq - 0 1",
        "e5e6",
    ),
    (
        "r1b2rk1/2q1b1pp/p2ppn2/1p6/3QP3/1BN1B3/PPP3PP/R4RK1 w - - 0 1",
        "a2a4",
    ),
    (
        "2r3k1/pppR1pp1/4p3/4P1P1/5P2/1P4K1/P1P5/8 w - - 0 1",
        "g5g6",
    ),
    (
        "1nk1r1r1/pp2n1pp/4p3/q2pPp1N/b1pP1P2/B1P2R2/2P1B1PP/R2Q2K1 w - - 0 1",
        "h5f6",
    ),
    ("4b3/p3kp2/6p1/3pP2p/2pP1P2/2P1K1P1/P7/8 w - - 0 1", "f4f5"),
    (
        "2kr1bnr/pbpq4/2n1pp2/3p3p/3P1P1B/2N2N1P/PPP3P1/2KR1B1R w - - 0 1",
        "f4f5",
    ),
    (
        "3rr1k1/pp3pp1/1qn2np1/8/3p4/PP1R1P2/2P1NQPP/R1B3K1 b - - 0 1",
        "c6e5",
    ),
    (
        "2r1nrk1/p2q1ppp/bp1p4/n1pPp3/P1P1P3/2PBB1N1/4QPPP/R4RK1 w - - 0 1",
        "f2f4",
    ),
    (
        "r3r1k1/ppqb1ppp/8/4p1NQ/8/2P5/PP3PPP/R3R1K1 w - - 0 1",
        "g5f7",
    ),
    (
        "r2q1rk1/4bppp/p2p4/2pP4/3pP3/3Q4/PP1B1PPP/R3R1K1 w - - 0 1",
        "b2b4",
    ),
    (
        "rnb2r1k/pp2p2p/2pp2p1/q2P1p2/8/1Pb2NP1/PB2PPBP/R2Q1RK1 w - - 0 1",
        "d1d2",
    ),
    (
        "2r3k1/1p2q1pp/2b1pr2/p1pp4/6Q1/1P1PP3/P1RB1PPP/1K3B1R w - - 0 1",
        "g4g7",
    ),
    (
        "r1bqkb1r/4npp1/p1p4p/1p1pP1B1/8/1B6/PPPN1PPP/R2Q1RK1 w kq - 0 1",
        "d2e4",
    ),
    (
        "r2q1rk1/1ppnbppp/p2p1nb1/3Pp3/2P1P1p1/2N2N1P/PPB1QPP1/R1BR2K1 b - - 0 1",
        "g6h5",
    ),
    (
        "r1bq1rk1/pp2ppbp/2np2p1/2n5/P3PP2/3pBN2/1PP1B1PP/RN1Q1RK1 b - - 0 1",
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

/// Aggregated benchmark summary.
#[derive(Clone, Debug)]
pub struct BenchSummary {
    pub engine_name: String,
    pub results: Vec<PositionResult>,
    pub total_nodes: u64,
    pub total_time: Duration,
    pub nps: u64,
    pub correct: usize,
    pub total: usize,
    pub accuracy: f64,
    pub estimated_elo: i32,
}

impl BenchSummary {
    /// Compute summary from position results.
    pub fn from_results(engine_name: &str, results: Vec<PositionResult>) -> Self {
        let total = results.len();
        let correct = results.iter().filter(|r| r.correct).count();
        let total_nodes: u64 = results.iter().map(|r| r.nodes).sum();
        let total_time: Duration = results.iter().map(|r| r.elapsed).sum();
        let total_ms = total_time.as_millis().max(1) as u64;
        let nps = total_nodes * 1000 / total_ms;
        let accuracy = if total > 0 {
            (correct as f64) / (total as f64) * 100.0
        } else {
            0.0
        };

        Self {
            engine_name: engine_name.to_string(),
            results,
            total_nodes,
            total_time,
            nps,
            correct,
            total,
            accuracy,
            estimated_elo: estimate_elo(accuracy),
        }
    }
}

impl fmt::Display for BenchSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "╔══════════════════════════════════════════════╗")?;
        writeln!(f, "║                  RESULTS                    ║")?;
        writeln!(f, "╠══════════════════════════════════════════════╣")?;
        writeln!(f, "║  Engine:      {:<31}║", self.engine_name)?;
        writeln!(
            f,
            "║  Accuracy:    {:>2}/{:<2} ({:>5.1}%)                ║",
            self.correct, self.total, self.accuracy
        )?;
        writeln!(f, "║  Est. ELO:    ~{:<30}║", self.estimated_elo)?;
        writeln!(f, "║  NPS:         {:<31}║", format_nps(self.nps))?;
        writeln!(f, "║  Total nodes: {:<31}║", format_nps(self.total_nodes))?;
        writeln!(
            f,
            "║  Total time:  {:<31}║",
            format!("{}ms", self.total_time.as_millis())
        )?;
        writeln!(f, "╚══════════════════════════════════════════════╝")?;
        Ok(())
    }
}

/// Maps BK accuracy (0–100) to an approximate ELO rating.
pub fn estimate_elo(accuracy: f64) -> i32 {
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
    fn test_estimate_elo_range() {
        assert!(estimate_elo(0.0) >= 800);
        assert!(estimate_elo(50.0) >= 1600);
        assert!(estimate_elo(100.0) >= 2500);
    }

    #[test]
    fn test_format_nps() {
        assert_eq!(format_nps(500), "500");
        assert_eq!(format_nps(1500), "1.5K");
        assert_eq!(format_nps(1_500_000), "1.50M");
        assert_eq!(format_nps(1_500_000_000), "1.50B");
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
}
