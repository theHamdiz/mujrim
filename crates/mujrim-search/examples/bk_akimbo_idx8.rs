fn main() {
    types::init();
    let fen = "2kr1bnr/pbpq4/2n1pp2/3p3p/3P1P1B/2N2N1Q/PPP3PP/2KR1B1R w - - 0 1";
    let mut board = types::Board::from_fen(fen).unwrap();
    let mut eng = mujrim_search::SearchEngine::new(256, 1);
    eng.set_params_for_preset("akimbo");
    // Search each candidate in isolation via making the move and evaluating opposite?
    // Better: use search_depth and print PV; also force-search after each root move via go searchmoves style if available.
    let res = eng.search_depth(&mut board, 16);
    println!(
        "best={} score={} pv_len nodes={}",
        res.best_move.to_uci(),
        res.score,
        res.nodes
    );
}
