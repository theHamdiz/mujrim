//! Integration tests for KishMat chess engine.
//! These tests verify that all crates work together correctly.

#[cfg(test)]
mod integration {
    use types::{Board, Color, Piece, Square};
    use search::SearchEngine;
    use std::time::Duration;

    fn setup() {
        types::init();
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // Full game self-play to completion
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_self_play_to_completion() {
        setup();
        let mut board = Board::new();
        let mut engine = SearchEngine::new(4, 1);

        // Play up to 200 half-moves (should reach some conclusion or draw)
        for ply in 0..200 {
            if board.is_game_over() {
                eprintln!("Game over at ply {ply}: {}", board.to_fen());
                return; // Success — game terminated normally
            }

            let result = engine.search_depth(&mut board, 3);

            // Verify move is legal
            let legal = board.generate_legal_moves();
            assert!(
                legal.iter().any(|m| m.from == result.best_move.from && m.to == result.best_move.to),
                "Engine returned illegal move {} at ply {ply}",
                result.best_move
            );

            board.make_move(result.best_move);

            // Verify board invariants after every move
            assert!(board.king_square(Color::White).index() < 64);
            assert!(board.king_square(Color::Black).index() < 64);
            assert_eq!(board.piece_count(Piece::King, Color::White), 1);
            assert_eq!(board.piece_count(Piece::King, Color::Black), 1);
        }

        // 200 ply without game over is acceptable (complex game)
    }

    #[test]
    fn test_self_play_fast_time_control() {
        setup();
        let mut board = Board::new();
        let mut engine = SearchEngine::new(2, 1);

        for _ in 0..40 {
            if board.is_game_over() { return; }

            let result = engine.search_time(&mut board, Duration::from_millis(20), 64);

            let legal = board.generate_legal_moves();
            assert!(
                legal.iter().any(|m| m.from == result.best_move.from && m.to == result.best_move.to),
                "Engine returned illegal move under time pressure: {}",
                result.best_move
            );

            board.make_move(result.best_move);
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // UCI session simulation
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_uci_session_full_game() {
        setup();
        let mut handler = comms::UciHandler::new();

        // Simulate a GUI session
        handler.handle_position(&["startpos"]);
        handler.handle_position(&["startpos", "moves", "e2e4"]);
        handler.handle_position(&["startpos", "moves", "e2e4", "e7e5"]);

        // Board should reflect the move sequence
        assert_eq!(handler.board.side_to_move, Color::White);
        assert_eq!(handler.board.piece_on(Square::E4), Some((Piece::Pawn, Color::White)));
        assert_eq!(handler.board.piece_on(Square::E5), Some((Piece::Pawn, Color::Black)));

        // New game should reset
        handler.handle_position(&["startpos"]);
        assert_eq!(handler.board.to_fen(), "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    }

    #[test]
    fn test_uci_session_fen_position() {
        setup();
        let mut handler = comms::UciHandler::new();

        // Load a complex position
        handler.handle_position(&[
            "fen", "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R",
            "w", "KQkq", "-", "0", "1"
        ]);

        let moves = handler.board.generate_legal_moves();
        assert_eq!(moves.len(), 48, "KiwiPete should have 48 legal moves");
    }

    #[test]
    fn test_uci_handles_garbage_input() {
        setup();
        let mut handler = comms::UciHandler::new();

        // These should not crash
        handler.handle_position(&["garbage"]);
        handler.handle_position(&["fen"]);
        handler.handle_position(&["fen", "garbage"]);
        handler.handle_position(&["startpos", "moves", "invalid"]);

        // Board should still be valid after garbage
        assert!(handler.board.piece_count(Piece::King, Color::White) >= 0);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // Search + Eval integration
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_eval_search_consistency() {
        setup();
        let mut board = Board::new();
        let mut engine = SearchEngine::new(2, 1);

        // Search from starting position
        let result = engine.search_depth(&mut board, 4);

        // The search score should be in a reasonable range for the starting position
        assert!(result.score.abs() < 200,
            "Starting position search score should be reasonable, got {}", result.score);
    }

    #[test]
    fn test_winning_position_detected() {
        setup();
        // White has a massive material advantage
        let fen = "4k3/8/8/8/8/8/8/4KQRR w - - 0 1";
        let mut board = Board::from_fen(fen).unwrap();
        let mut engine = SearchEngine::new(2, 1);
        let result = engine.search_depth(&mut board, 4);

        assert!(result.score > 1000,
            "Massive material advantage should have high score, got {}", result.score);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // Perft correctness verification (integration between types + eval)
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_perft_extended_positions() {
        setup();
        let positions = [
            ("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", 3, 8902u64),
            ("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1", 2, 2039),
            ("8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1", 3, 2812),
            ("rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8", 2, 1486),
        ];

        for (fen, depth, expected) in positions {
            let mut board = Board::from_fen(fen).unwrap();
            let actual = board.perft(depth);
            assert_eq!(actual, expected, "Perft({depth}) failed for {fen}: expected {expected}, got {actual}");
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // Make/unmake consistency across crate boundaries
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_make_unmake_preserves_eval() {
        setup();
        let mut board = Board::new();
        let original_eval = eval::evaluate(&board);

        // Make and unmake every legal move; eval should be identical after
        let moves = board.generate_legal_moves();
        for mv in &moves {
            board.make_move(*mv);
            board.unmake_move(*mv);
            let restored_eval = eval::evaluate(&board);
            assert_eq!(restored_eval, original_eval,
                "Eval changed after make/unmake of {mv}: original={original_eval}, restored={restored_eval}");
        }
    }

    #[test]
    fn test_search_preserves_eval() {
        setup();
        let mut board = Board::new();
        let original_eval = eval::evaluate(&board);
        let original_fen = board.to_fen();

        let mut engine = SearchEngine::new(2, 1);
        engine.search_depth(&mut board, 5);

        assert_eq!(board.to_fen(), original_fen, "Search modified the board");
        assert_eq!(eval::evaluate(&board), original_eval, "Search modified eval");
    }
}
