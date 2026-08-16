//! Legal and pseudo-legal move generation.

use super::Board;
use super::attack_tables::*;
use crate::chess_move::{Move, MoveFlag, MoveList};
use crate::piece::Piece;
use crate::square::Square;

impl Board {
    /// Generates all legal moves for the current side to move.
    /// Uses in-place make/unmake for performance (no board cloning).
    pub fn generate_legal_moves(&mut self) -> MoveList {
        let pseudo = self.generate_pseudo_legal_moves(self.side_to_move);
        let mut legal = MoveList::new();

        for i in 0..pseudo.len() {
            let mv = pseudo[i];
            if self.is_legal_move(mv) {
                legal.push(mv);
            }
        }
        legal
    }

    /// Generates legal captures only (for quiescence search).
    pub fn generate_legal_captures(&mut self) -> MoveList {
        let pseudo_caps = self.generate_captures(self.side_to_move);
        let mut legal = MoveList::new();
        for i in 0..pseudo_caps.len() {
            let mv = pseudo_caps[i];
            if self.is_legal_move(mv) {
                legal.push(mv);
            }
        }
        legal
    }

    /// Legal non-capture moves (quiet pushes, castling; excludes EP, captures, promotions).
    ///
    /// Paired with [`Self::generate_legal_captures`], this partitions the legal move set:
    /// captures ∩ quiets = ∅, captures ∪ quiets = legal moves.
    pub fn generate_legal_quiets(&mut self) -> MoveList {
        let pseudo = self.generate_pseudo_legal_quiets(self.side_to_move);
        let mut legal = MoveList::new();

        for i in 0..pseudo.len() {
            let mv = pseudo[i];
            if self.is_legal_move(mv) {
                legal.push(mv);
            }
        }
        legal
    }

    /// Whether `mv` checks the opponent, computed from the current position.
    #[inline]
    pub fn gives_check(&self, mv: Move) -> bool {
        let us = self.side_to_move;
        let them = us.opponent();
        let king = self.king_square(them);
        let king_bb = king.bitboard();
        let from_bb = mv.from.bitboard();
        let to_bb = mv.to.bitboard();
        let mut occ = (self.all_occupancy() & !from_bb) | to_bb;
        let mut vacated = from_bb;
        if mv.flag == MoveFlag::EnPassant {
            let cap = Square::from_file_rank(mv.to.file(), mv.from.rank());
            occ &= !cap.bitboard();
            vacated |= cap.bitboard();
        }
        if mv.is_castling() {
            let kingside = mv.flag == MoveFlag::KingCastle;
            let rook_from = self.castling_rook_from(us, kingside);
            let rook_to = Self::castling_rook_landing(us, kingside);
            occ = (occ & !rook_from.bitboard()) | rook_to.bitboard();
            vacated |= rook_from.bitboard();
            if rook_attacks(rook_to.index(), occ) & king_bb != 0 {
                return true;
            }
        }
        let Some(moved) = self.piece_of_color_on(mv.from, us) else {
            return false;
        };
        let piece = mv.promotion.unwrap_or(moved);
        let direct = match piece {
            Piece::Pawn => pawn_attacks(us.index(), mv.to.index()),
            Piece::Knight => knight_attacks(mv.to.index()),
            Piece::Bishop => bishop_attacks(mv.to.index(), occ),
            Piece::Rook => rook_attacks(mv.to.index(), occ),
            Piece::Queen => queen_attacks(mv.to.index(), occ),
            Piece::King => king_attacks(mv.to.index()),
        };
        if direct & king_bb != 0 {
            return true;
        }
        let sliders = self.color_occupancy(us) & !vacated;
        let bishops =
            sliders & (self.piece_bb(Piece::Bishop, us) | self.piece_bb(Piece::Queen, us));
        let rooks = sliders & (self.piece_bb(Piece::Rook, us) | self.piece_bb(Piece::Queen, us));
        (bishop_attacks(king.index(), occ) & bishops) != 0
            || (rook_attacks(king.index(), occ) & rooks) != 0
    }
}

// ── Perft (performance test for move generation correctness) ────────────────

impl Board {
    /// Counts the number of leaf nodes at a given depth (for testing move gen).
    pub fn perft(&mut self, depth: u32) -> u64 {
        if depth == 0 {
            return 1;
        }

        let moves = self.generate_legal_moves();

        if depth == 1 {
            return moves.len() as u64;
        }

        let mut nodes = 0u64;
        for i in 0..moves.len() {
            let mv = moves[i];
            self.make_move(mv);
            nodes += self.perft(depth - 1);
            self.unmake_move(mv);
        }
        nodes
    }
}

#[cfg(test)]
mod tests {
    use super::super::Board;
    use crate::chess_move::{MoveFlag, MoveList};
    use crate::piece::{Color, Piece};
    use crate::square::Square;

    fn setup() {
        crate::init();
    }

    // ── Starting position ───────────────────────────────────────────────────

    #[test]
    fn test_starting_position_legal_moves() {
        setup();
        let mut board = Board::new();
        let moves = board.generate_legal_moves();
        assert_eq!(
            moves.len(),
            20,
            "Starting position should have 20 legal moves"
        );
    }

    #[test]
    fn test_starting_captures_empty() {
        setup();
        let mut board = Board::new();
        let caps = board.generate_legal_captures();
        assert_eq!(caps.len(), 0, "Starting position should have 0 captures");
    }

    #[test]
    fn test_legal_quiets_partition_full_moves() {
        setup();
        let fens = [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "1k1r4/pp1b1R2/3q2pp/4p3/2B5/4Q3/PPP2B2/2K5 b - - 0 1",
        ];
        for fen in fens {
            let mut board = Board::from_fen(fen).unwrap();
            let full = board.generate_legal_moves();
            let caps = board.generate_legal_captures();
            let quiets = board.generate_legal_quiets();
            assert_eq!(
                caps.len() + quiets.len(),
                full.len(),
                "partition size mismatch for {fen}"
            );
            for m in full.as_slice() {
                let in_caps = caps.iter().any(|c| same_move_struct(*m, *c));
                let in_quiet = quiets.iter().any(|q| same_move_struct(*m, *q));
                assert!(
                    in_caps ^ in_quiet,
                    "move {m} caps={in_caps} quiet={in_quiet} fen={fen}"
                );
            }
        }
    }

    fn same_move_struct(a: crate::chess_move::Move, b: crate::chess_move::Move) -> bool {
        a.from == b.from && a.to == b.to && a.promotion == b.promotion && a.flag == b.flag
    }

    #[test]
    fn test_is_legal_move_accepts_generated_legal() {
        setup();
        let mut board = Board::new();
        let legal = board.generate_legal_moves();
        for m in legal.as_slice() {
            assert!(board.is_legal_move(*m), "{m}");
        }
    }

    #[test]
    fn test_pseudo_legal_quiets_exclude_captures_and_promotions() {
        setup();
        let board =
            Board::from_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1")
                .unwrap();
        let color = board.side_to_move;
        let pq = board.generate_pseudo_legal_quiets(color);
        for m in pq.as_slice() {
            assert!(
                !m.is_capture() && !m.is_promotion(),
                "quiet pseudo contained tactical move {m}"
            );
        }
        let full = board.generate_pseudo_legal_moves(color);
        assert!(
            pq.len() <= full.len(),
            "quiet pseudos should not outnumber full pseudo"
        );
    }

    // ── Perft suite (gold standard for correctness) ─────────────────────────

    #[test]
    fn non_mutating_legality_matches_make_unmake_reference() {
        setup();
        let positions = [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "rnb1kbnr/pppp1ppp/4p3/8/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3",
            "8/8/8/r4pPK/8/8/8/4k3 w - f6 0 1",
        ];

        for fen in positions {
            let mut board = Board::from_fen(fen).unwrap();
            let pseudo = board.generate_pseudo_legal_moves(board.side_to_move);
            let mut reference = MoveList::new();
            for &mv in pseudo.as_slice() {
                let us = board.side_to_move;
                let mut after = board.clone();
                after.make_move(mv);
                if !after.is_in_check(us) {
                    reference.push(mv);
                }
            }
            let fast = board.generate_legal_moves();
            assert_eq!(fast.as_slice(), reference.as_slice(), "FEN: {fen}");
        }
    }

    #[test]
    fn test_perft_1() {
        setup();
        let mut board = Board::new();
        assert_eq!(board.perft(1), 20);
    }

    #[test]
    fn test_perft_2() {
        setup();
        let mut board = Board::new();
        assert_eq!(board.perft(2), 400);
    }

    #[test]
    fn test_perft_3() {
        setup();
        let mut board = Board::new();
        assert_eq!(board.perft(3), 8902);
    }

    #[test]
    fn test_perft_4() {
        setup();
        let mut board = Board::new();
        assert_eq!(board.perft(4), 197_281);
    }

    // KiwiPete — tests en passant, castling, promotions, complex positions
    #[test]
    fn test_kiwipete_perft_1() {
        setup();
        let fen = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
        let mut board = Board::from_fen(fen).unwrap();
        assert_eq!(board.perft(1), 48);
    }

    #[test]
    fn test_kiwipete_perft_2() {
        setup();
        let fen = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
        let mut board = Board::from_fen(fen).unwrap();
        assert_eq!(board.perft(2), 2039);
    }

    #[test]
    fn test_kiwipete_perft_3() {
        setup();
        let fen = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
        let mut board = Board::from_fen(fen).unwrap();
        assert_eq!(board.perft(3), 97862);
    }

    // Position 3 from CPW — tests en passant edge cases
    #[test]
    fn test_cpw_position_3_perft() {
        setup();
        let fen = "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1";
        let mut board = Board::from_fen(fen).unwrap();
        assert_eq!(board.perft(1), 14);
        assert_eq!(board.perft(2), 191);
        assert_eq!(board.perft(3), 2812);
        assert_eq!(board.perft(4), 43238);
    }

    // Position 4 from CPW — tests promotion and mixed captures
    #[test]
    fn test_cpw_position_4_perft() {
        setup();
        let fen = "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1";
        let mut board = Board::from_fen(fen).unwrap();
        assert_eq!(board.perft(1), 6);
        assert_eq!(board.perft(2), 264);
        assert_eq!(board.perft(3), 9467);
    }

    // Position 5 from CPW (mirrored)
    #[test]
    fn test_cpw_position_5_perft() {
        setup();
        let fen = "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8";
        let mut board = Board::from_fen(fen).unwrap();
        assert_eq!(board.perft(1), 44);
        assert_eq!(board.perft(2), 1486);
        assert_eq!(board.perft(3), 62379);
    }

    // ── Check detection ─────────────────────────────────────────────────────

    #[test]
    fn test_in_check_detection() {
        setup();
        let fen = "r1bqkb1r/pppp1Qpp/2n2n2/4p3/2B1P3/8/PPPP1PPP/RNB1K1NR b KQkq - 0 4";
        let mut board = Board::from_fen(fen).unwrap();
        assert!(board.is_in_check(Color::Black));
        assert!(board.is_checkmate());
    }

    #[test]
    fn test_not_checkmate_can_block() {
        setup();
        // White rook checks on e8, but black can block
        let fen = "4k3/8/8/8/8/8/8/R3K3 w - - 0 1";
        let mut board = Board::from_fen(fen).unwrap();
        assert!(!board.is_checkmate()); // White is not in checkmate
    }

    #[test]
    fn test_moves_while_in_check() {
        setup();
        // Black king is in check by white queen
        let fen = "rnb1kbnr/pppp1ppp/4p3/8/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3";
        let mut board = Board::from_fen(fen).unwrap();
        assert!(board.in_check());
        let moves = board.generate_legal_moves();
        // All legal moves must get out of check
        for mv in &moves {
            let mut b2 = board.clone();
            b2.make_move(*mv);
            assert!(
                !b2.is_in_check(Color::White),
                "Move {} leaves king in check!",
                mv
            );
        }
    }

    // ── Castling ────────────────────────────────────────────────────────────

    #[test]
    fn test_castling_generation() {
        setup();
        let fen = "r3k2r/pppppppp/8/8/8/8/PPPPPPPP/R3K2R w KQkq - 0 1";
        let mut board = Board::from_fen(fen).unwrap();
        let moves = board.generate_legal_moves();
        assert_eq!(
            moves.iter().filter(|m| m.is_castling()).count(),
            2,
            "Should have 2 castling moves"
        );
    }

    #[test]
    fn test_no_castling_through_check() {
        setup();
        // Black queen on f4 attacks f1 through open f-file (no f2 pawn)
        let fen = "r3k2r/pppppppp/8/8/5q2/8/PPPPP1PP/R3K2R w KQkq - 0 1";
        let mut board = Board::from_fen(fen).unwrap();
        let moves = board.generate_legal_moves();
        // Should NOT be able to castle kingside (queen attacks f1)
        assert!(
            !moves.iter().any(|m| m.flag == MoveFlag::KingCastle),
            "Should not castle through check on f1"
        );
    }

    #[test]
    fn test_no_castling_while_in_check() {
        setup();
        // Black rook checks white king on e1
        let fen = "4k3/8/8/8/4r3/8/8/R3K2R w KQ - 0 1";
        let mut board = Board::from_fen(fen).unwrap();
        assert!(board.in_check());
        let moves = board.generate_legal_moves();
        assert!(
            !moves.iter().any(|m| m.is_castling()),
            "Should not castle while in check"
        );
    }

    #[test]
    fn test_castling_move_executes_correctly() {
        setup();
        let fen = "r3k2r/pppppppp/8/8/8/8/PPPPPPPP/R3K2R w KQkq - 0 1";
        let mut board = Board::from_fen(fen).unwrap();

        // Kingside castle
        let moves = board.generate_legal_moves();
        let kcastle = moves
            .iter()
            .find(|m| m.flag == MoveFlag::KingCastle)
            .expect("Kingside castle should exist");
        board.make_move(*kcastle);

        assert_eq!(board.king_square(Color::White), Square::G1);
        assert_eq!(
            board.piece_on(Square::F1),
            Some((Piece::Rook, Color::White))
        );
        assert_eq!(board.piece_on(Square::H1), None);
        assert_eq!(board.piece_on(Square::E1), None);
    }

    // ── En passant ──────────────────────────────────────────────────────────

    #[test]
    fn test_en_passant_generation() {
        setup();
        let fen = "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPP1PPP/RNBQKBNR w KQkq f6 0 3";
        let mut board = Board::from_fen(fen).unwrap();
        let moves = board.generate_legal_moves();
        assert!(
            moves.iter().any(|m| m.flag == MoveFlag::EnPassant),
            "Should have at least 1 en passant move"
        );
    }

    #[test]
    fn test_en_passant_capture_removes_pawn() {
        setup();
        let fen = "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPP1PPP/RNBQKBNR w KQkq f6 0 3";
        let mut board = Board::from_fen(fen).unwrap();
        let moves = board.generate_legal_moves();
        let ep = moves
            .iter()
            .find(|m| m.flag == MoveFlag::EnPassant)
            .expect("Should have EP move");

        let captured_sq = Square::from_file_rank(ep.to.file(), ep.from.rank());
        board.make_move(*ep);

        // The captured pawn should be gone
        assert_eq!(
            board.piece_on(captured_sq),
            None,
            "En passant captured pawn should be removed"
        );
        // The capturing pawn should be on the ep square
        assert_eq!(board.piece_on(ep.to), Some((Piece::Pawn, Color::White)));
    }

    // ── Promotions ──────────────────────────────────────────────────────────

    #[test]
    fn test_promotion_generates_four_options() {
        setup();
        // White pawn on a7, can promote
        let fen = "8/P7/8/8/8/8/8/4K2k w - - 0 1";
        let mut board = Board::from_fen(fen).unwrap();
        let moves = board.generate_legal_moves();
        assert_eq!(
            moves.iter().filter(|m| m.is_promotion()).count(),
            4,
            "Should have 4 promotion options (Q/R/B/N)"
        );
    }

    #[test]
    fn test_promotion_capture_generates_four_options() {
        setup();
        // White pawn on a7, can capture and promote on b8
        let fen = "1n6/P7/8/8/8/8/8/4K2k w - - 0 1";
        let mut board = Board::from_fen(fen).unwrap();
        let moves = board.generate_legal_moves();
        assert_eq!(
            moves
                .iter()
                .filter(|m| m.flag == MoveFlag::PromotionCapture)
                .count(),
            4,
            "Should have 4 promotion-capture options"
        );
    }

    // ── All moves are legal ─────────────────────────────────────────────────

    #[test]
    fn test_all_generated_moves_are_legal() {
        setup();
        let positions = [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
        ];

        for fen in positions {
            let mut board = Board::from_fen(fen).unwrap();
            let us = board.side_to_move;
            let moves = board.generate_legal_moves();

            for mv in &moves {
                let predicted = board.gives_check(*mv);
                let mut b2 = board.clone();
                b2.make_move(*mv);
                assert!(
                    !b2.is_in_check(us),
                    "Move {} leaves own king in check in position: {}",
                    mv,
                    fen
                );
                assert_eq!(
                    predicted,
                    b2.in_check(),
                    "gives_check({}) mismatch in {fen}",
                    mv.to_uci()
                );
            }
        }
    }
}
