//! Legal and pseudo-legal move generation.

use super::attack_tables::*;
use super::{BLACK_KING_CASTLE, BLACK_QUEEN_CASTLE, Board, WHITE_KING_CASTLE, WHITE_QUEEN_CASTLE};
use crate::bitboard::*;
use crate::chess_move::{Move, MoveFlag, MoveList};
use crate::piece::{Color, Piece};
use crate::square::Square;

impl Board {
    /// Returns true if the given square is attacked by the given color.
    pub fn is_square_attacked(&self, sq: Square, by_color: Color) -> bool {
        let sq_idx = sq.index();
        let occ = self.all_occupancy();

        // Pawn attacks: check if any opponent pawn attacks this square
        if pawn_attacks(by_color.opponent().index(), sq_idx) & self.piece_bb(Piece::Pawn, by_color)
            != 0
        {
            return true;
        }
        // Knight
        if knight_attacks(sq_idx) & self.piece_bb(Piece::Knight, by_color) != 0 {
            return true;
        }
        // King
        if king_attacks(sq_idx) & self.piece_bb(Piece::King, by_color) != 0 {
            return true;
        }
        // Bishop / Queen (diagonals)
        let diag = bishop_attacks(sq_idx, occ);
        if diag & (self.piece_bb(Piece::Bishop, by_color) | self.piece_bb(Piece::Queen, by_color))
            != 0
        {
            return true;
        }
        // Rook / Queen (lines)
        let lines = rook_attacks(sq_idx, occ);
        if lines & (self.piece_bb(Piece::Rook, by_color) | self.piece_bb(Piece::Queen, by_color))
            != 0
        {
            return true;
        }

        false
    }

    #[inline(always)]
    fn is_square_attacked_after(
        &self,
        sq: Square,
        by_color: Color,
        occupancy: Bitboard,
        removed_attackers: Bitboard,
    ) -> bool {
        let sq_idx = sq.index();
        let attackers = !removed_attackers;
        if pawn_attacks(by_color.opponent().index(), sq_idx)
            & self.piece_bb(Piece::Pawn, by_color)
            & attackers
            != 0
        {
            return true;
        }
        if knight_attacks(sq_idx) & self.piece_bb(Piece::Knight, by_color) & attackers != 0 {
            return true;
        }
        if king_attacks(sq_idx) & self.piece_bb(Piece::King, by_color) & attackers != 0 {
            return true;
        }
        if bishop_attacks(sq_idx, occupancy)
            & (self.piece_bb(Piece::Bishop, by_color) | self.piece_bb(Piece::Queen, by_color))
            & attackers
            != 0
        {
            return true;
        }
        rook_attacks(sq_idx, occupancy)
            & (self.piece_bb(Piece::Rook, by_color) | self.piece_bb(Piece::Queen, by_color))
            & attackers
            != 0
    }

    #[inline(always)]
    fn castling_is_legal_after_move(&self, mv: Move, us: Color) -> bool {
        let them = us.opponent();
        let kingside = match mv.flag {
            MoveFlag::KingCastle => true,
            MoveFlag::QueenCastle => false,
            _ => return false,
        };
        let king_to = Board::castling_king_landing(us, kingside);
        let rook_from = self.castling_rook_from(us, kingside);
        let rook_to = Board::castling_rook_landing(us, kingside);
        let transit = rook_to;
        if self.is_square_attacked_after(mv.from, them, self.all_occupancy(), 0) {
            return false;
        }
        let without_king = self.all_occupancy() & !mv.from.bitboard();
        let transit_occupancy = without_king | transit.bitboard();
        if self.is_square_attacked_after(transit, them, transit_occupancy, 0) {
            return false;
        }
        let final_occupancy =
            (without_king & !rook_from.bitboard()) | rook_to.bitboard() | king_to.bitboard();
        !self.is_square_attacked_after(king_to, them, final_occupancy, 0)
    }

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

    /// Generates all pseudo-legal moves (may leave king in check).
    pub fn generate_pseudo_legal_moves(&self, color: Color) -> MoveList {
        let mut moves = MoveList::new();

        self.gen_pawn_moves(color, &mut moves);
        self.gen_knight_moves(color, &mut moves);
        self.gen_bishop_moves(color, &mut moves);
        self.gen_rook_moves(color, &mut moves);
        self.gen_queen_moves(color, &mut moves);
        self.gen_king_moves(color, &mut moves);
        self.gen_castling_moves(color, &mut moves);

        moves
    }

    /// Generates all pseudo-legal capture moves directly (for quiescence search).
    /// Much faster than generating all moves and filtering.
    pub fn generate_captures(&self, color: Color) -> MoveList {
        let mut moves = MoveList::new();
        self.gen_pawn_captures(color, &mut moves);
        self.gen_piece_captures(Piece::Knight, color, &mut moves);
        self.gen_sliding_captures(Piece::Bishop, color, &mut moves);
        self.gen_sliding_captures(Piece::Rook, color, &mut moves);
        self.gen_sliding_captures(Piece::Queen, color, &mut moves);
        self.gen_king_captures(color, &mut moves);
        moves
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

    /// Pseudo-legal quiet moves only (empty destination squares); no captures, EP, or promotions.
    pub fn generate_pseudo_legal_quiets(&self, color: Color) -> MoveList {
        let mut moves = MoveList::new();
        self.gen_pawn_quiets(color, &mut moves);
        self.gen_knight_quiets(color, &mut moves);
        self.gen_bishop_quiets(color, &mut moves);
        self.gen_rook_quiets(color, &mut moves);
        self.gen_queen_quiets(color, &mut moves);
        self.gen_king_quiets(color, &mut moves);
        self.gen_castling_moves(color, &mut moves);
        moves
    }

    /// Returns true if a pseudo-legal move leaves the moving king safe.
    #[inline(always)]
    pub fn is_legal_move(&self, mv: Move) -> bool {
        let us = self.side_to_move;
        let them = us.opponent();
        let moving_piece = match self.piece_of_color_on(mv.from, us) {
            Some(piece) => piece,
            None => return false,
        };

        if mv.is_castling() {
            return moving_piece == Piece::King && self.castling_is_legal_after_move(mv, us);
        }

        let removed_attacker = if mv.flag == MoveFlag::EnPassant {
            Square::from_file_rank(mv.to.file(), mv.from.rank()).bitboard()
        } else if mv.is_capture() {
            mv.to.bitboard()
        } else {
            0
        };
        let occupancy =
            (self.all_occupancy() & !mv.from.bitboard() & !removed_attacker) | mv.to.bitboard();
        let king = if moving_piece == Piece::King {
            mv.to
        } else {
            self.king_square(us)
        };
        !self.is_square_attacked_after(king, them, occupancy, removed_attacker)
    }

    // ── Pawn moves ──────────────────────────────────────────────────────────

    fn gen_pawn_moves(&self, color: Color, moves: &mut MoveList) {
        let pawns = self.piece_bb(Piece::Pawn, color);
        let occ = self.all_occupancy();
        let enemies = self.color_occupancy(color.opponent());
        let promo_rank = color.promotion_rank();
        let dir = color.pawn_direction();
        let start_rank = color.pawn_start_rank();

        for from_idx in iter_bits(pawns) {
            let from = Square::from_index(from_idx);
            let from_rank = from.rank();
            let _from_file = from.file();

            // Single push
            let to_idx = (from_idx as i32 + dir) as usize;
            if to_idx < 64 {
                let to = Square::from_index(to_idx);
                let to_bb = to.bitboard();

                if occ & to_bb == 0 {
                    if to.rank() == promo_rank {
                        // Promotion
                        for promo_piece in [Piece::Queen, Piece::Rook, Piece::Bishop, Piece::Knight]
                        {
                            moves.push(Move::promotion(from, to, promo_piece));
                        }
                    } else {
                        moves.push(Move::quiet(from, to));

                        // Double push from starting rank
                        if from_rank == start_rank {
                            let double_idx = (from_idx as i32 + 2 * dir) as usize;
                            if double_idx < 64 {
                                let double_to = Square::from_index(double_idx);
                                if occ & double_to.bitboard() == 0 {
                                    moves.push(Move::double_pawn(from, double_to));
                                }
                            }
                        }
                    }
                }
            }

            // Captures (diagonal)
            let pawn_atk = pawn_attacks(color.index(), from_idx);
            for to_idx in iter_bits(pawn_atk & enemies) {
                let to = Square::from_index(to_idx);
                if to.rank() == promo_rank {
                    for promo_piece in [Piece::Queen, Piece::Rook, Piece::Bishop, Piece::Knight] {
                        moves.push(Move::promotion_capture(from, to, promo_piece));
                    }
                } else {
                    moves.push(Move::capture(from, to));
                }
            }

            // En passant
            if let Some(ep_sq) = self.en_passant
                && pawn_atk & ep_sq.bitboard() != 0
            {
                moves.push(Move::en_passant(from, ep_sq));
            }
        }
    }

    fn gen_pawn_quiets(&self, color: Color, moves: &mut MoveList) {
        let pawns = self.piece_bb(Piece::Pawn, color);
        let occ = self.all_occupancy();
        let promo_rank = color.promotion_rank();
        let dir = color.pawn_direction();
        let start_rank = color.pawn_start_rank();

        for from_idx in iter_bits(pawns) {
            let from = Square::from_index(from_idx);
            let from_rank = from.rank();

            let to_idx = (from_idx as i32 + dir) as usize;
            if to_idx < 64 {
                let to = Square::from_index(to_idx);
                if occ & to.bitboard() == 0 {
                    if to.rank() == promo_rank {
                        continue;
                    }
                    moves.push(Move::quiet(from, to));
                    if from_rank == start_rank {
                        let double_idx = (from_idx as i32 + 2 * dir) as usize;
                        if double_idx < 64 {
                            let double_to = Square::from_index(double_idx);
                            if occ & double_to.bitboard() == 0 {
                                moves.push(Move::double_pawn(from, double_to));
                            }
                        }
                    }
                }
            }
        }
    }

    // ── Knight moves ────────────────────────────────────────────────────────

    fn gen_knight_moves(&self, color: Color, moves: &mut MoveList) {
        let knights = self.piece_bb(Piece::Knight, color);
        let friendly = self.color_occupancy(color);
        let enemies = self.color_occupancy(color.opponent());

        for from_idx in iter_bits(knights) {
            let from = Square::from_index(from_idx);
            let attacks = knight_attacks(from_idx) & !friendly;

            for to_idx in iter_bits(attacks) {
                let to = Square::from_index(to_idx);
                if enemies & to.bitboard() != 0 {
                    moves.push(Move::capture(from, to));
                } else {
                    moves.push(Move::quiet(from, to));
                }
            }
        }
    }

    fn gen_knight_quiets(&self, color: Color, moves: &mut MoveList) {
        let knights = self.piece_bb(Piece::Knight, color);
        let not_occupied_by_us = !self.color_occupancy(color);
        let enemies = self.color_occupancy(color.opponent());

        for from_idx in iter_bits(knights) {
            let from = Square::from_index(from_idx);
            let attacks = knight_attacks(from_idx) & not_occupied_by_us & !enemies;
            for to_idx in iter_bits(attacks) {
                moves.push(Move::quiet(from, Square::from_index(to_idx)));
            }
        }
    }

    // ── Bishop moves ────────────────────────────────────────────────────────

    fn gen_bishop_moves(&self, color: Color, moves: &mut MoveList) {
        let bishops = self.piece_bb(Piece::Bishop, color);
        let friendly = self.color_occupancy(color);
        let enemies = self.color_occupancy(color.opponent());
        let occ = self.all_occupancy();

        for from_idx in iter_bits(bishops) {
            let from = Square::from_index(from_idx);
            let attacks = bishop_attacks(from_idx, occ) & !friendly;

            for to_idx in iter_bits(attacks) {
                let to = Square::from_index(to_idx);
                if enemies & to.bitboard() != 0 {
                    moves.push(Move::capture(from, to));
                } else {
                    moves.push(Move::quiet(from, to));
                }
            }
        }
    }

    fn gen_bishop_quiets(&self, color: Color, moves: &mut MoveList) {
        let bishops = self.piece_bb(Piece::Bishop, color);
        let not_occupied_by_us = !self.color_occupancy(color);
        let enemies = self.color_occupancy(color.opponent());
        let occ = self.all_occupancy();

        for from_idx in iter_bits(bishops) {
            let from = Square::from_index(from_idx);
            let attacks = bishop_attacks(from_idx, occ) & not_occupied_by_us & !enemies;
            for to_idx in iter_bits(attacks) {
                moves.push(Move::quiet(from, Square::from_index(to_idx)));
            }
        }
    }

    // ── Rook moves ──────────────────────────────────────────────────────────

    fn gen_rook_moves(&self, color: Color, moves: &mut MoveList) {
        let rooks = self.piece_bb(Piece::Rook, color);
        let friendly = self.color_occupancy(color);
        let enemies = self.color_occupancy(color.opponent());
        let occ = self.all_occupancy();

        for from_idx in iter_bits(rooks) {
            let from = Square::from_index(from_idx);
            let attacks = rook_attacks(from_idx, occ) & !friendly;

            for to_idx in iter_bits(attacks) {
                let to = Square::from_index(to_idx);
                if enemies & to.bitboard() != 0 {
                    moves.push(Move::capture(from, to));
                } else {
                    moves.push(Move::quiet(from, to));
                }
            }
        }
    }

    fn gen_rook_quiets(&self, color: Color, moves: &mut MoveList) {
        let rooks = self.piece_bb(Piece::Rook, color);
        let not_occupied_by_us = !self.color_occupancy(color);
        let enemies = self.color_occupancy(color.opponent());
        let occ = self.all_occupancy();

        for from_idx in iter_bits(rooks) {
            let from = Square::from_index(from_idx);
            let attacks = rook_attacks(from_idx, occ) & not_occupied_by_us & !enemies;
            for to_idx in iter_bits(attacks) {
                moves.push(Move::quiet(from, Square::from_index(to_idx)));
            }
        }
    }

    // ── Queen moves ─────────────────────────────────────────────────────────

    fn gen_queen_moves(&self, color: Color, moves: &mut MoveList) {
        let queens = self.piece_bb(Piece::Queen, color);
        let friendly = self.color_occupancy(color);
        let enemies = self.color_occupancy(color.opponent());
        let occ = self.all_occupancy();

        for from_idx in iter_bits(queens) {
            let from = Square::from_index(from_idx);
            let attacks = queen_attacks(from_idx, occ) & !friendly;

            for to_idx in iter_bits(attacks) {
                let to = Square::from_index(to_idx);
                if enemies & to.bitboard() != 0 {
                    moves.push(Move::capture(from, to));
                } else {
                    moves.push(Move::quiet(from, to));
                }
            }
        }
    }

    fn gen_queen_quiets(&self, color: Color, moves: &mut MoveList) {
        let queens = self.piece_bb(Piece::Queen, color);
        let not_occupied_by_us = !self.color_occupancy(color);
        let enemies = self.color_occupancy(color.opponent());
        let occ = self.all_occupancy();

        for from_idx in iter_bits(queens) {
            let from = Square::from_index(from_idx);
            let attacks = queen_attacks(from_idx, occ) & not_occupied_by_us & !enemies;
            for to_idx in iter_bits(attacks) {
                moves.push(Move::quiet(from, Square::from_index(to_idx)));
            }
        }
    }

    // ── King moves ──────────────────────────────────────────────────────────

    fn gen_king_moves(&self, color: Color, moves: &mut MoveList) {
        let king_bb = self.piece_bb(Piece::King, color);
        if king_bb == 0 {
            return;
        }
        let from_idx = get_lsb(king_bb);
        let from = Square::from_index(from_idx);
        let friendly = self.color_occupancy(color);
        let enemies = self.color_occupancy(color.opponent());

        let attacks = king_attacks(from_idx) & !friendly;

        for to_idx in iter_bits(attacks) {
            let to = Square::from_index(to_idx);
            if enemies & to.bitboard() != 0 {
                moves.push(Move::capture(from, to));
            } else {
                moves.push(Move::quiet(from, to));
            }
        }
    }

    fn gen_king_quiets(&self, color: Color, moves: &mut MoveList) {
        let king_bb = self.piece_bb(Piece::King, color);
        if king_bb == 0 {
            return;
        }
        let from_idx = get_lsb(king_bb);
        let from = Square::from_index(from_idx);
        let not_occupied_by_us = !self.color_occupancy(color);
        let enemies = self.color_occupancy(color.opponent());
        let attacks = king_attacks(from_idx) & not_occupied_by_us & !enemies;
        for to_idx in iter_bits(attacks) {
            moves.push(Move::quiet(from, Square::from_index(to_idx)));
        }
    }

    // ── Castling ────────────────────────────────────────────────────────────

    fn gen_castling_moves(&self, color: Color, moves: &mut MoveList) {
        if self.castling_rights == 0 {
            return;
        }
        if self.is_chess960() {
            if self.can_castle(color, true) {
                let from = self.king_square(color);
                moves.push(Move::king_castle(from, self.castle_uci_to(color, true)));
            }
            if self.can_castle(color, false) {
                let from = self.king_square(color);
                moves.push(Move::queen_castle(from, self.castle_uci_to(color, false)));
            }
            return;
        }

        let occ = self.all_occupancy();
        let enemy = color.opponent();

        match color {
            Color::White => {
                // Kingside: E1 -> G1, need F1 and G1 empty, E1/F1/G1 not attacked
                if self.castling_rights & WHITE_KING_CASTLE != 0 {
                    let between = Square::F1.bitboard() | Square::G1.bitboard();
                    if occ & between == 0
                        && !self.is_square_attacked(Square::E1, enemy)
                        && !self.is_square_attacked(Square::F1, enemy)
                        && !self.is_square_attacked(Square::G1, enemy)
                    {
                        moves.push(Move::king_castle(Square::E1, Square::G1));
                    }
                }
                // Queenside: E1 -> C1, need B1/C1/D1 empty, E1/D1/C1 not attacked
                if self.castling_rights & WHITE_QUEEN_CASTLE != 0 {
                    let between =
                        Square::B1.bitboard() | Square::C1.bitboard() | Square::D1.bitboard();
                    if occ & between == 0
                        && !self.is_square_attacked(Square::E1, enemy)
                        && !self.is_square_attacked(Square::D1, enemy)
                        && !self.is_square_attacked(Square::C1, enemy)
                    {
                        moves.push(Move::queen_castle(Square::E1, Square::C1));
                    }
                }
            }
            Color::Black => {
                if self.castling_rights & BLACK_KING_CASTLE != 0 {
                    let between = Square::F8.bitboard() | Square::G8.bitboard();
                    if occ & between == 0
                        && !self.is_square_attacked(Square::E8, enemy)
                        && !self.is_square_attacked(Square::F8, enemy)
                        && !self.is_square_attacked(Square::G8, enemy)
                    {
                        moves.push(Move::king_castle(Square::E8, Square::G8));
                    }
                }
                if self.castling_rights & BLACK_QUEEN_CASTLE != 0 {
                    let between =
                        Square::B8.bitboard() | Square::C8.bitboard() | Square::D8.bitboard();
                    if occ & between == 0
                        && !self.is_square_attacked(Square::E8, enemy)
                        && !self.is_square_attacked(Square::D8, enemy)
                        && !self.is_square_attacked(Square::C8, enemy)
                    {
                        moves.push(Move::queen_castle(Square::E8, Square::C8));
                    }
                }
            }
        }
    }

    fn can_castle(&self, color: Color, kingside: bool) -> bool {
        let right = match (color, kingside) {
            (Color::White, true) => WHITE_KING_CASTLE,
            (Color::White, false) => WHITE_QUEEN_CASTLE,
            (Color::Black, true) => BLACK_KING_CASTLE,
            (Color::Black, false) => BLACK_QUEEN_CASTLE,
        };
        if self.castling_rights & right == 0 {
            return false;
        }
        let king = self.king_square(color);
        let rook = self.castling_rook_from(color, kingside);
        if self.piece_on(rook) != Some((Piece::Rook, color)) {
            return false;
        }
        let king_to = Board::castling_king_landing(color, kingside);
        let rook_to = Board::castling_rook_landing(color, kingside);
        let occ = self.all_occupancy() & !king.bitboard() & !rook.bitboard();
        if rank_between(king, rook) & occ != 0 {
            return false;
        }
        if rank_between(king, king_to) & occ != 0 {
            return false;
        }
        if rank_between(rook, rook_to) & occ != 0 {
            return false;
        }
        let enemy = color.opponent();
        for sq in inclusive_rank_walk(king, king_to) {
            if self.is_square_attacked(sq, enemy) {
                return false;
            }
        }
        true
    }

    // ── Capture-only generation (for quiescence search) ────────────────────

    fn gen_pawn_captures(&self, color: Color, moves: &mut MoveList) {
        let pawns = self.piece_bb(Piece::Pawn, color);
        let enemies = self.color_occupancy(color.opponent());
        let promo_rank = color.promotion_rank();

        for from_idx in iter_bits(pawns) {
            let from = Square::from_index(from_idx);
            let pawn_atk = pawn_attacks(color.index(), from_idx);

            // Normal captures
            for to_idx in iter_bits(pawn_atk & enemies) {
                let to = Square::from_index(to_idx);
                if to.rank() == promo_rank {
                    for promo_piece in [Piece::Queen, Piece::Rook, Piece::Bishop, Piece::Knight] {
                        moves.push(Move::promotion_capture(from, to, promo_piece));
                    }
                } else {
                    moves.push(Move::capture(from, to));
                }
            }

            // En passant
            if let Some(ep_sq) = self.en_passant
                && pawn_atk & ep_sq.bitboard() != 0
            {
                moves.push(Move::en_passant(from, ep_sq));
            }

            // Non-capture promotions (also tactical)
            let occ = self.all_occupancy();
            let dir = color.pawn_direction();
            let to_idx = (from_idx as i32 + dir) as usize;
            if to_idx < 64 {
                let to = Square::from_index(to_idx);
                if occ & to.bitboard() == 0 && to.rank() == promo_rank {
                    for promo_piece in [Piece::Queen, Piece::Rook, Piece::Bishop, Piece::Knight] {
                        moves.push(Move::promotion(from, to, promo_piece));
                    }
                }
            }
        }
    }

    fn gen_piece_captures(&self, piece: Piece, color: Color, moves: &mut MoveList) {
        let pieces = self.piece_bb(piece, color);
        let enemies = self.color_occupancy(color.opponent());

        for from_idx in iter_bits(pieces) {
            let from = Square::from_index(from_idx);
            let attacks = knight_attacks(from_idx) & enemies;
            for to_idx in iter_bits(attacks) {
                moves.push(Move::capture(from, Square::from_index(to_idx)));
            }
        }
    }

    fn gen_sliding_captures(&self, piece: Piece, color: Color, moves: &mut MoveList) {
        let pieces = self.piece_bb(piece, color);
        let enemies = self.color_occupancy(color.opponent());
        let occ = self.all_occupancy();

        for from_idx in iter_bits(pieces) {
            let from = Square::from_index(from_idx);
            let attacks = match piece {
                Piece::Bishop => bishop_attacks(from_idx, occ),
                Piece::Rook => rook_attacks(from_idx, occ),
                Piece::Queen => queen_attacks(from_idx, occ),
                _ => 0,
            } & enemies;
            for to_idx in iter_bits(attacks) {
                moves.push(Move::capture(from, Square::from_index(to_idx)));
            }
        }
    }

    fn gen_king_captures(&self, color: Color, moves: &mut MoveList) {
        let king_bb = self.piece_bb(Piece::King, color);
        if king_bb == 0 {
            return;
        }
        let from_idx = get_lsb(king_bb);
        let from = Square::from_index(from_idx);
        let enemies = self.color_occupancy(color.opponent());
        let attacks = king_attacks(from_idx) & enemies;
        for to_idx in iter_bits(attacks) {
            moves.push(Move::capture(from, Square::from_index(to_idx)));
        }
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

#[inline(always)]
fn rank_between(from: Square, to: Square) -> u64 {
    if from.rank() != to.rank() {
        return 0;
    }
    let (lo, hi) = if from.file() < to.file() {
        (from.file(), to.file())
    } else {
        (to.file(), from.file())
    };
    let mut bb = 0u64;
    let rank = from.rank();
    let mut file = lo + 1;
    while file < hi {
        bb |= Square::from_file_rank(file, rank).bitboard();
        file += 1;
    }
    bb
}

#[inline(always)]
fn inclusive_rank_walk(from: Square, to: Square) -> impl Iterator<Item = Square> {
    let rank = from.rank();
    let start = from.file();
    let end = to.file();
    let step: i8 = if end >= start { 1 } else { -1 };
    let count = start.abs_diff(end) as usize + 1;
    (0..count).map(move |i| {
        let file = (start as i8 + step * i as i8) as u8;
        Square::from_file_rank(file, rank)
    })
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
                let mut b2 = board.clone();
                b2.make_move(*mv);
                assert!(
                    !b2.is_in_check(us),
                    "Move {} leaves own king in check in position: {}",
                    mv,
                    fen
                );
            }
        }
    }
}
