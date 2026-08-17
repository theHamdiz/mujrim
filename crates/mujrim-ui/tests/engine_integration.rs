//! GUI integration: board, book, and dedicated-engine discovery.
//!
//! The UI must not link `mujrim-search` / `mujrim-eval`. Play and analysis
//! spawn the dedicated engine binary at runtime.

use std::path::PathBuf;

use types::{Board, Color, Piece, Square};

fn setup() {
    types::init();
}

fn load_opening_book() -> types::book::OpeningBook {
    let crate_book = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("mujrim-search")
        .join("book")
        .join("book.bin");
    types::book::OpeningBook::load(&crate_book)
        .or_else(|_| types::book::OpeningBook::load_embedded())
        .expect("Opening book should load from mujrim-search/book or a books/ directory")
}

#[test]
fn test_startpos_has_20_legal_moves() {
    setup();
    let mut board = Board::new();
    let moves = board.generate_legal_moves();
    assert_eq!(
        moves.len(),
        20,
        "Starting position should have exactly 20 legal moves"
    );
}

#[test]
fn test_piece_detection_startpos() {
    setup();
    let board = Board::new();
    assert_eq!(
        board.piece_on(Square::from_index(0)),
        Some((Piece::Rook, Color::White))
    );
}

#[test]
fn test_fen_round_trip() {
    setup();
    let fens = [
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
    ];
    for fen in fens {
        let mut board =
            Board::from_fen(fen).unwrap_or_else(|_| panic!("Failed to parse FEN: {fen}"));
        let legal = board.generate_legal_moves();
        assert!(!legal.is_empty(), "Position should have legal moves: {fen}");
    }
}

#[test]
fn test_opening_book_returns_move_for_startpos() {
    setup();
    let book = load_opening_book();
    let board = Board::new();
    let mv = book.probe(&board);
    assert!(
        mv.is_some(),
        "Opening book should have at least one move for the starting position"
    );
}

#[test]
fn test_opening_book_move_is_legal() {
    setup();
    let book = load_opening_book();
    let mut board = Board::new();
    let book_move = book
        .probe(&board)
        .expect("Book should return a move for startpos");
    let legal = board.generate_legal_moves();
    assert!(
        legal
            .iter()
            .any(|mv| mv.from == book_move.from && mv.to == book_move.to),
        "Book move should be in the legal moves list"
    );
}

#[test]
fn test_opening_book_move_after_e4() {
    setup();
    let book = load_opening_book();
    let mut board = Board::new();
    let e2 = Square::from_index(12);
    let e4 = Square::from_index(28);
    let moves = board.generate_legal_moves();
    let e2e4 = moves
        .iter()
        .find(|mv| mv.from == e2 && mv.to == e4)
        .expect("e2e4 should be legal");
    board.make_move(*e2e4);
    let book_reply = book.probe(&board);
    assert!(
        book_reply.is_some(),
        "Opening book should have a reply to 1. e4"
    );
    let reply = book_reply.unwrap();
    let legal_after = board.generate_legal_moves();
    assert!(
        legal_after
            .iter()
            .any(|mv| mv.from == reply.from && mv.to == reply.to),
        "Book reply to 1. e4 should be a legal move"
    );
}

#[test]
fn gui_manifest_does_not_link_search_eval_or_gpu() {
    let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
    assert!(
        !manifest.contains("package = \"mujrim-search\""),
        "GUI must load engines from dedicated binaries, not mujrim-search"
    );
    assert!(
        !manifest.contains("package = \"mujrim-eval\""),
        "GUI must not link mujrim-eval / NNUE nets"
    );
    assert!(
        !manifest.contains("package = \"mujrim-gpu\""),
        "GUI must not link mujrim-gpu"
    );
    assert!(
        !manifest.contains("embedded-networks"),
        "GUI must never enable embedded-networks"
    );
}
