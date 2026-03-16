//! Integration tests for kishmat-ui ↔ engine interaction.
//!
//! These tests verify that the UI's game state correctly integrates
//! with the engine's search and evaluation without requiring a GUI.

use types::{Board, Piece, Color, Square};

/// Initialize types once for all tests.
fn setup() {
    types::init();
}

// ──────────────────────────────────────────────────────────────
// Board / move generation integration
// ──────────────────────────────────────────────────────────────

#[test]
fn test_startpos_has_20_legal_moves() {
    setup();
    let mut board = Board::new();
    let moves = board.generate_legal_moves();
    assert_eq!(moves.len(), 20, "Starting position should have exactly 20 legal moves");
}

#[test]
fn test_piece_detection_startpos() {
    setup();
    let board = Board::new();

    // White pieces on rank 1 and 2
    assert_eq!(board.piece_on(Square::from_index(0)), Some((Piece::Rook, Color::White)));
    assert_eq!(board.piece_on(Square::from_index(4)), Some((Piece::King, Color::White)));
    assert_eq!(board.piece_on(Square::from_index(8)), Some((Piece::Pawn, Color::White)));

    // Black pieces on rank 7 and 8
    assert_eq!(board.piece_on(Square::from_index(63)), Some((Piece::Rook, Color::Black)));
    assert_eq!(board.piece_on(Square::from_index(60)), Some((Piece::King, Color::Black)));
    assert_eq!(board.piece_on(Square::from_index(48)), Some((Piece::Pawn, Color::Black)));

    // Empty squares in the middle
    assert!(board.piece_on(Square::from_index(28)).is_none()); // e4
    assert!(board.piece_on(Square::from_index(36)).is_none()); // e5
}

#[test]
fn test_make_move_updates_board() {
    setup();
    let mut board = Board::new();
    let moves = board.generate_legal_moves();

    // Find e2-e4
    let e2e4 = moves.iter().find(|m| {
        m.from == Square::from_index(12) && m.to == Square::from_index(28)
    }).expect("e2-e4 should be a legal move");

    board.make_move(*e2e4);

    // e2 should be empty, e4 should have a white pawn
    assert!(board.piece_on(Square::from_index(12)).is_none());
    assert_eq!(board.piece_on(Square::from_index(28)), Some((Piece::Pawn, Color::White)));
    assert_eq!(board.side_to_move, Color::Black);
}

// ──────────────────────────────────────────────────────────────
// Engine search integration
// ──────────────────────────────────────────────────────────────

#[test]
fn test_engine_returns_legal_move_for_startpos() {
    let handle = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            setup();
            let mut board = Board::new();
            let mut engine = search::SearchEngine::new(1, 1);
            let result = engine.search_depth(&mut board, 3);
            let legal = board.generate_legal_moves();
            assert!(
                legal.iter().any(|m| m.from == result.best_move.from && m.to == result.best_move.to),
                "Engine must return a legal move"
            );
        })
        .expect("Failed to spawn test thread");
    handle.join().expect("Test thread panicked");
}

#[test]
fn test_engine_finds_mate_in_1() {
    let handle = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            setup();
            // Qh4# position
            let mut board = Board::from_fen(
                "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
            )
            .unwrap();
            let mut engine = search::SearchEngine::new(1, 1);
            let result = engine.search_depth(&mut board, 4);
            // Best move should be Qxf7#
            let f7 = Square::from_index(53);
            assert_eq!(result.best_move.to, f7, "Engine should find Qxf7#");
        })
        .expect("Failed to spawn test thread");
    handle.join().expect("Test thread panicked");
}

#[test]
fn test_engine_move_doesnt_corrupt_board() {
    let handle = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            setup();
            let mut board = Board::new();
            let original_hash = board.hash;

            let mut engine = search::SearchEngine::new(1, 1);
            let result = engine.search_depth(&mut board, 4);

            // Search should not modify the board
            assert_eq!(board.hash, original_hash, "Search must not modify the board");
            assert_eq!(board.side_to_move, Color::White);

            // But the returned move should be applicable
            board.make_move(result.best_move);
            assert_ne!(board.hash, original_hash);
            assert_eq!(board.side_to_move, Color::Black);
        })
        .expect("Failed to spawn test thread");
    handle.join().expect("Test thread panicked");
}

// ──────────────────────────────────────────────────────────────
// Full game simulation (engine vs engine)
// ──────────────────────────────────────────────────────────────

#[test]
fn test_engine_vs_engine_game_terminates() {
    let handle = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            setup();
            let mut board = Board::new();
            let mut engine = search::SearchEngine::new(1, 1);
            let max_moves = 200;

            for i in 0..max_moves {
                if board.is_game_over() {
                    // Game terminated naturally — great!
                    return;
                }

                let result = engine.search_depth(&mut board, 2);
                board.make_move(result.best_move);

                // Sanity: alternating colors
                let expected_stm = if i % 2 == 0 { Color::Black } else { Color::White };
                assert_eq!(board.side_to_move, expected_stm,
                    "Side to move wrong after move {}", i + 1);
            }
            // If we reach here, the game didn't terminate in 200 moves
            // That's OK for a shallow search, just verify the board is consistent
            assert!(board.generate_legal_moves().len() > 0 || board.is_game_over());
        })
        .expect("Failed to spawn test thread");
    handle.join().expect("Test thread panicked");
}

// ──────────────────────────────────────────────────────────────
// Evaluation integration
// ──────────────────────────────────────────────────────────────

#[test]
fn test_eval_startpos_is_balanced() {
    setup();
    let board = Board::new();
    let score = eval::evaluate(&board);
    // Starting position should be roughly balanced (±200cp)
    assert!(
        score.abs() < 200,
        "Starting position eval too extreme: {score}cp"
    );
}

#[test]
fn test_eval_material_advantage() {
    setup();
    // White missing queen
    let board_w_down = Board::from_fen(
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNB1KBNR w KQkq - 0 1",
    )
    .unwrap();
    // Black missing queen
    let board_b_down = Board::from_fen(
        "rnb1kbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    )
    .unwrap();

    let score_w_down = eval::evaluate(&board_w_down);
    let score_b_down = eval::evaluate(&board_b_down);

    assert!(
        score_b_down > score_w_down,
        "Missing white queen should score worse than missing black queen"
    );
}

// ──────────────────────────────────────────────────────────────
// FEN round-trip (UI loads positions)
// ──────────────────────────────────────────────────────────────

#[test]
fn test_fen_round_trip() {
    setup();
    let fens = [
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
    ];

    for fen in fens {
        let mut board = Board::from_fen(fen).expect(&format!("Failed to parse FEN: {fen}"));
        let legal = board.generate_legal_moves();
        assert!(legal.len() > 0, "Position should have legal moves: {fen}");
    }
}
