//! Bratko–Kopec positions that were failing in a **depth 16 / 120s-per-position** snapshot.
//!
//! ## Always-on tests
//! - [`bk_tactical_targets_are_legal`]: sanity — suite answers are legal moves.
//!
//! ## Aspirational tests (ignored on CI)
//! Run **all nine** in one process (fair CPU, no starvation):
//! `cargo test -p mujrim-search --release --test bk_tactical_targets bk_all_tactical_misses_sequential -- --ignored --nocapture --test-threads=1`
//!
//! Per-case tests run in parallel by default; use `--test-threads=1` if you run
//! `cargo test ... -- --ignored` so each case gets full cores.
//!
//! FENs stay in sync with `mujrim-benchmarker::suite::BK_POSITIONS` indices
//! **4, 6, 8, 9, 11, 14, 16, 17, 22** (zero-based).

use mujrim_search::SearchEngine;
use std::time::Duration;
use types::{Board, Move};

fn setup() {
    types::init();
}

/// `(fen, expected_best_uci)` — known tactical misses to track over time.
const BK_TACTICAL_MISSES: &[(&str, &str)] = &[
    (
        "r1b2rk1/2q1b1pp/p2ppn2/1p6/3QP3/1BN1B3/PPP3PP/R4RK1 w - - 0 1",
        "c3d5",
    ),
    (
        "1nk1r1r1/pp2n1pp/4p3/q2pPp1N/b1pP1P2/B1P2R2/2P1B1PP/R2Q2K1 w - - 0 1",
        "h5f6",
    ),
    (
        "2kr1bnr/pbpq4/2n1pp2/3p3p/3P1P1B/2N2N1Q/PPP3PP/2KR1B1R w - - 0 1",
        "f4f5",
    ),
    (
        "3rr1k1/pp3pp1/1qn2np1/8/3p4/PP1R1P2/2P1NQPP/R1B3K1 b - - 0 1",
        "c6e5",
    ),
    (
        "r3r1k1/ppqb1ppp/8/4p1NQ/8/2P5/PP3PPP/R3R1K1 b - - 0 1",
        "d7f5",
    ),
    (
        "2r3k1/1p2q1pp/2b1pr2/p1pp4/6Q1/1P1PP1R1/P1PN2PP/5RK1 w - - 0 1",
        "g4g7",
    ),
    (
        "r2q1rk1/1ppnbppp/p2p1nb1/3Pp3/2P1P1P1/2N2N1P/PPB1QP2/R1B2RK1 b - - 0 1",
        "g6h5",
    ),
    (
        "r1bq1rk1/pp2ppbp/2np2p1/2n5/P3PP2/N1P2N2/1PB3PP/R1B1QRK1 b - - 0 1",
        "c5b3",
    ),
    (
        "r1bqk2r/pp2bppp/2p5/3pP3/P2Q1P2/2N1B3/1PP3PP/R4RK1 b kq - 0 1",
        "f7f6",
    ),
];

#[test]
fn bk_tactical_targets_are_legal() {
    setup();
    assert_eq!(BK_TACTICAL_MISSES.len(), 9);
    for (fen, uci) in BK_TACTICAL_MISSES {
        let mut board = Board::from_fen(fen).expect(fen);
        let legal = board.generate_legal_moves();
        let needle = Move::from_uci(uci).expect(uci);
        let ok = legal
            .as_slice()
            .iter()
            .any(|m| m.from == needle.from && m.to == needle.to && m.promotion == needle.promotion);
        assert!(ok, "expected move {uci} not legal in position {fen}");
    }
}

#[test]
#[ignore = "Runs ~9×90s wall time; use for full BK tactical audit"]
fn bk_all_tactical_misses_sequential() {
    setup();
    const DEPTH: i32 = 20;
    const SECS: u64 = 90;
    let mut passed = 0usize;
    let mut lines: Vec<String> = Vec::new();

    for (i, (fen, want_uci)) in BK_TACTICAL_MISSES.iter().enumerate() {
        let mut board = Board::from_fen(fen).unwrap();
        let mut eng = SearchEngine::new(256, 1);
        let res = eng.search_time_hard(&mut board, Duration::from_secs(SECS), DEPTH);
        let got = res.best_move.to_uci();
        if got == *want_uci {
            passed += 1;
            lines.push(format!(
                "  [{}] OK  best={got}  depth={}  nodes={}",
                i + 1,
                res.depth,
                res.nodes
            ));
        } else {
            lines.push(format!(
                "  [{}] FAIL  want={want_uci}  got={got}  depth={}  nodes={}",
                i + 1,
                res.depth,
                res.nodes
            ));
        }
    }

    println!(
        "BK tactical misses (sequential release, {SECS}s / d{DEPTH}, 1 thread): {}/9 pass\n{}",
        passed,
        lines.join("\n")
    );
    // Baseline from 2026-04 audit (after incremental MovePicker): 2/9 at these limits.
    const MIN_PASS: usize = 2;
    assert!(
        passed >= MIN_PASS,
        "regression: {} BK hits (min {}). Raise MIN_PASS when strength improves.",
        passed,
        MIN_PASS
    );
    if passed < 9 {
        eprintln!(
            "\nNote: {}/9 — not failing on partial pass. Goal 9/9; increase MIN_PASS or assert_eq!(passed, 9) when ready.",
            passed
        );
    }
}

#[test]
#[ignore = "Aspirational: engine should find c3d5 at high depth"]
fn bk_miss_01_c3d5() {
    setup();
    let fen = BK_TACTICAL_MISSES[0].0;
    let uci = BK_TACTICAL_MISSES[0].1;
    let mut board = Board::from_fen(fen).unwrap();
    let mut eng = SearchEngine::new(256, 1);
    let res = eng.search_time_hard(&mut board, Duration::from_secs(90), 20);
    assert_eq!(res.best_move.to_uci(), uci, "nodes={}", res.nodes);
}

#[test]
#[ignore = "Aspirational: engine should find h5f6"]
fn bk_miss_02_h5f6() {
    setup();
    let fen = BK_TACTICAL_MISSES[1].0;
    let uci = BK_TACTICAL_MISSES[1].1;
    let mut board = Board::from_fen(fen).unwrap();
    let mut eng = SearchEngine::new(256, 1);
    let res = eng.search_time_hard(&mut board, Duration::from_secs(90), 20);
    assert_eq!(res.best_move.to_uci(), uci, "nodes={}", res.nodes);
}

#[test]
#[ignore = "Aspirational: engine should find f4f5"]
fn bk_miss_03_f4f5_w() {
    setup();
    let fen = BK_TACTICAL_MISSES[2].0;
    let uci = BK_TACTICAL_MISSES[2].1;
    let mut board = Board::from_fen(fen).unwrap();
    let mut eng = SearchEngine::new(256, 1);
    let res = eng.search_time_hard(&mut board, Duration::from_secs(90), 20);
    assert_eq!(res.best_move.to_uci(), uci, "nodes={}", res.nodes);
}

#[test]
#[ignore = "Aspirational: engine should find c6e5"]
fn bk_miss_04_c6e5() {
    setup();
    let fen = BK_TACTICAL_MISSES[3].0;
    let uci = BK_TACTICAL_MISSES[3].1;
    let mut board = Board::from_fen(fen).unwrap();
    let mut eng = SearchEngine::new(256, 1);
    let res = eng.search_time_hard(&mut board, Duration::from_secs(90), 20);
    assert_eq!(res.best_move.to_uci(), uci, "nodes={}", res.nodes);
}

#[test]
#[ignore = "Aspirational: engine should find g5f7"]
fn bk_miss_05_g5f7() {
    setup();
    let fen = BK_TACTICAL_MISSES[4].0;
    let uci = BK_TACTICAL_MISSES[4].1;
    let mut board = Board::from_fen(fen).unwrap();
    let mut eng = SearchEngine::new(256, 1);
    let res = eng.search_time_hard(&mut board, Duration::from_secs(90), 20);
    assert_eq!(res.best_move.to_uci(), uci, "nodes={}", res.nodes);
}

#[test]
#[ignore = "Aspirational: engine should find g4g7"]
fn bk_miss_06_g4g7() {
    setup();
    let fen = BK_TACTICAL_MISSES[5].0;
    let uci = BK_TACTICAL_MISSES[5].1;
    let mut board = Board::from_fen(fen).unwrap();
    let mut eng = SearchEngine::new(256, 1);
    let res = eng.search_time_hard(&mut board, Duration::from_secs(90), 20);
    assert_eq!(res.best_move.to_uci(), uci, "nodes={}", res.nodes);
}

#[test]
#[ignore = "Aspirational: engine should find g6h5"]
fn bk_miss_07_g6h5() {
    setup();
    let fen = BK_TACTICAL_MISSES[6].0;
    let uci = BK_TACTICAL_MISSES[6].1;
    let mut board = Board::from_fen(fen).unwrap();
    let mut eng = SearchEngine::new(256, 1);
    let res = eng.search_time_hard(&mut board, Duration::from_secs(90), 20);
    assert_eq!(res.best_move.to_uci(), uci, "nodes={}", res.nodes);
}

#[test]
#[ignore = "Aspirational: engine should find c5b3"]
fn bk_miss_08_c5b3() {
    setup();
    let fen = BK_TACTICAL_MISSES[7].0;
    let uci = BK_TACTICAL_MISSES[7].1;
    let mut board = Board::from_fen(fen).unwrap();
    let mut eng = SearchEngine::new(256, 1);
    let res = eng.search_time_hard(&mut board, Duration::from_secs(90), 20);
    assert_eq!(res.best_move.to_uci(), uci, "nodes={}", res.nodes);
}

#[test]
#[ignore = "Aspirational: engine should find f7f6"]
fn bk_miss_09_f7f6() {
    setup();
    let fen = BK_TACTICAL_MISSES[8].0;
    let uci = BK_TACTICAL_MISSES[8].1;
    let mut board = Board::from_fen(fen).unwrap();
    let mut eng = SearchEngine::new(256, 1);
    let res = eng.search_time_hard(&mut board, Duration::from_secs(90), 20);
    assert_eq!(res.best_move.to_uci(), uci, "nodes={}", res.nodes);
}
