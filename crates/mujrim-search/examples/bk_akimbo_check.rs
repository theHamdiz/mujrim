fn main() {
    types::init();
    for (name, fen, want) in [
        ("idx8", "2kr1bnr/pbpq4/2n1pp2/3p3p/3P1P1B/2N2N1Q/PPP3PP/2KR1B1R w - - 0 1", &["f4f5","c1b1","d1e1"][..]),
        ("idx23", "r2qnrnk/p2b2b1/1p1p2pp/2pPpp2/1PP1P3/PRNBB3/3QNPPP/5RK1 w - - 0 1", &["f2f4"][..]),
    ] {
        for d in [16i32, 18] {
            let mut board = types::Board::from_fen(fen).unwrap();
            let mut eng = mujrim_search::SearchEngine::new(256, 1);
            eng.set_params_for_preset("akimbo");
            let res = eng.search_depth(&mut board, d);
            let uci = res.best_move.to_uci();
            println!("akimbo {name} d{d}: {uci} ok={}", want.contains(&uci.as_str()));
        }
    }
}
