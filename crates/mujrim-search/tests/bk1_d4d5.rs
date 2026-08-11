//! Bratko–Kopec index 1: White breakthrough `d4d5`.
//!
//! Reckless must find this by depth 16 (native-v60 finds it by ~depth 10).

use mujrim_search::SearchEngine;
use types::Board;

const BK1_FEN: &str = "3r1k2/4npp1/1ppr3p/p6P/P2PPPP1/1NR5/5K2/2R5 w - - 0 1";

#[test]
fn bk1_expected_move_is_legal() {
    types::init();
    let mut board = Board::from_fen(BK1_FEN).expect(BK1_FEN);
    let legal = board.generate_legal_moves();
    let needle = types::Move::from_uci("d4d5").unwrap();
    assert!(
        legal
            .as_slice()
            .iter()
            .any(|m| m.from == needle.from && m.to == needle.to),
        "d4d5 must be legal in BK#1"
    );
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "depth-16 Reckless search is too slow under debug"
)]
fn bk1_reckless_finds_d4d5_by_depth_16() {
    types::init();
    let mut board = Board::from_fen(BK1_FEN).unwrap();
    let mut eng = SearchEngine::new(128, 1);
    eng.set_params_for_preset("reckless");
    let res = eng.search_depth(&mut board, 16);
    assert_eq!(
        res.best_move.to_uci(),
        "d4d5",
        "BK#1 Reckless must find d4d5 by depth 16 (got {}, score={}, nodes={})",
        res.best_move.to_uci(),
        res.score,
        res.nodes
    );
}
