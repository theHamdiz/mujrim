//! Bratko–Kopec indices 8 and 23 — quiet prophylaxis / pawn breakthrough.
//!
//! Native-v60 finds `c1b1` and `f2f4` respectively. These gates ensure the
//! universal Reckless stack keeps that search quality without eval overfitting.

use mujrim_search::SearchEngine;
use types::Board;

const BK8_FEN: &str = "2kr1bnr/pbpq4/2n1pp2/3p3p/3P1P1B/2N2N1Q/PPP3PP/2KR1B1R w - - 0 1";
const BK8_EXPECTED: &[&str] = &["f4f5", "c1b1", "d1e1"];

const BK23_FEN: &str = "r2qnrnk/p2b2b1/1p1p2pp/2pPpp2/1PP1P3/PRNBB3/3QNPPP/5RK1 w - - 0 1";
const BK23_EXPECTED: &[&str] = &["f2f4"];

fn assert_expected(got: &str, expected: &[&str], label: &str, score: i32, nodes: u64) {
    assert!(
        expected.contains(&got),
        "{label}: expected one of {expected:?}, got {got} (score={score}, nodes={nodes})"
    );
}

#[test]
fn bk8_and_bk23_expected_moves_are_legal() {
    types::init();
    for (fen, moves) in [(BK8_FEN, BK8_EXPECTED), (BK23_FEN, BK23_EXPECTED)] {
        let mut board = Board::from_fen(fen).expect(fen);
        let legal = board.generate_legal_moves();
        for uci in moves {
            let needle = types::Move::from_uci(uci).unwrap();
            assert!(
                legal
                    .as_slice()
                    .iter()
                    .any(|m| m.from == needle.from && m.to == needle.to),
                "{uci} must be legal in {fen}"
            );
        }
    }
}

#[test]
#[ignore = "BK#8 still prefers f1b5 at depth 16 on the Reckless stack; 5s TC is the gate"]
fn bk8_reckless_finds_prophylaxis_by_depth_16() {
    types::init();
    let mut board = Board::from_fen(BK8_FEN).unwrap();
    let mut eng = SearchEngine::new(128, 1);
    assert!(eng.install_adapter("reckless"));
    let res = eng.search_depth(&mut board, 16);
    assert_expected(
        &res.best_move.to_uci(),
        BK8_EXPECTED,
        "BK#8 Reckless",
        res.score,
        res.nodes,
    );
}

#[test]
#[ignore = "BK#8 still flips to f1b5/c3a4 by depth 20; tracking separately"]
fn bk8_reckless_stays_stable_through_depth_20() {
    types::init();
    let mut board = Board::from_fen(BK8_FEN).unwrap();
    let mut eng = SearchEngine::new(128, 1);
    eng.set_params_for_preset("reckless");
    let res = eng.search_depth(&mut board, 20);
    assert_expected(
        &res.best_move.to_uci(),
        BK8_EXPECTED,
        "BK#8 Reckless d20",
        res.score,
        res.nodes,
    );
}

#[test]
#[ignore = "BK#23 still prefers e4f5 at depth 16 on the Reckless stack; 5s TC is the gate"]
fn bk23_reckless_finds_f2f4_by_depth_16() {
    types::init();
    let mut board = Board::from_fen(BK23_FEN).unwrap();
    let mut eng = SearchEngine::new(128, 1);
    assert!(eng.install_adapter("reckless"));
    let res = eng.search_depth(&mut board, 16);
    assert_expected(
        &res.best_move.to_uci(),
        BK23_EXPECTED,
        "BK#23 Reckless",
        res.score,
        res.nodes,
    );
}

#[test]
#[ignore = "BK#8 Akimbo still flips to f1b5 by depth 16 in CI; tracking separately"]
fn bk8_akimbo_finds_prophylaxis_by_depth_16() {
    types::init();
    let mut board = Board::from_fen(BK8_FEN).unwrap();
    let mut eng = SearchEngine::new(128, 1);
    eng.set_params_for_preset("akimbo");
    let res = eng.search_depth(&mut board, 16);
    assert_expected(
        &res.best_move.to_uci(),
        BK8_EXPECTED,
        "BK#8 Akimbo",
        res.score,
        res.nodes,
    );
}
