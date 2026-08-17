//! Shared mailbox + bitboard accessors and move generation.

use super::attack_tables::*;
use super::zobrist::zobrist;
use super::{
    BLACK_KING_CASTLE, BLACK_QUEEN_CASTLE, Board, BoardSnapshot, WHITE_KING_CASTLE,
    WHITE_QUEEN_CASTLE, apply_castling_rights_update,
};
use crate::bitboard::{Bitboard, get_lsb, iter_bits};
use crate::chess_move::{Move, MoveFlag, MoveList};
use crate::piece::{Color, Piece};
use crate::square::Square;

pub trait MailboxPos {
    fn pieces(&self) -> &[[Bitboard; 6]; 2];
    fn occupancy(&self) -> &[Bitboard; 2];
    fn piece_at(&self, square: Square) -> u8;
    fn side_to_move(&self) -> Color;
    fn en_passant(&self) -> Option<Square>;
    fn castling_rights(&self) -> u8;
    fn chess960(&self) -> bool;
    fn castling_king_file(&self, color: Color) -> u8;
    fn castling_rook_file(&self, color: Color, kingside: bool) -> u8;
    fn hash(&self) -> u64;
    fn halfmove_clock(&self) -> u32;

    #[inline(always)]
    fn piece_of_color_on(&self, square: Square, color: Color) -> Option<Piece> {
        let id = self.piece_at(square);
        (id & 1 == color.index() as u8)
            .then(|| Piece::from_index(usize::from(id) / 2))
            .flatten()
    }

    #[inline]
    fn piece_on(&self, square: Square) -> Option<(Piece, Color)> {
        let id = self.piece_at(square);
        let piece = Piece::from_index(usize::from(id) / 2)?;
        let color = if id & 1 == 0 {
            Color::White
        } else {
            Color::Black
        };
        Some((piece, color))
    }

    #[inline(always)]
    fn castling_rook_from(&self, color: Color, kingside: bool) -> Square {
        let file = self.castling_rook_file(color, kingside);
        let rank = if color == Color::White { 0 } else { 7 };
        Square::from_file_rank(file, rank)
    }

    #[inline(always)]
    fn castle_uci_to(&self, color: Color, kingside: bool) -> Square {
        if self.chess960() {
            self.castling_rook_from(color, kingside)
        } else {
            Board::castling_king_landing(color, kingside)
        }
    }

    #[inline(always)]
    fn piece_bb(&self, piece: Piece, color: Color) -> Bitboard {
        self.pieces()[color.index()][piece.index()]
    }

    #[inline(always)]
    fn all_occupancy(&self) -> Bitboard {
        let occ = self.occupancy();
        occ[0] | occ[1]
    }

    #[inline(always)]
    fn color_occupancy(&self, color: Color) -> Bitboard {
        self.occupancy()[color.index()]
    }

    #[inline(always)]
    fn king_square(&self, color: Color) -> Square {
        let king_bb = self.piece_bb(Piece::King, color);
        debug_assert!(king_bb != 0, "king must exist on the board");
        Square::from_index(get_lsb(king_bb))
    }

    fn is_square_attacked(&self, sq: Square, by_color: Color) -> bool {
        let sq_idx = sq.index();
        let occ = self.all_occupancy();

        if pawn_attacks(by_color.opponent().index(), sq_idx) & self.piece_bb(Piece::Pawn, by_color)
            != 0
        {
            return true;
        }
        if knight_attacks(sq_idx) & self.piece_bb(Piece::Knight, by_color) != 0 {
            return true;
        }
        if king_attacks(sq_idx) & self.piece_bb(Piece::King, by_color) != 0 {
            return true;
        }
        let diag = bishop_attacks(sq_idx, occ);
        if diag & (self.piece_bb(Piece::Bishop, by_color) | self.piece_bb(Piece::Queen, by_color))
            != 0
        {
            return true;
        }
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

    fn generate_pseudo_legal_moves(&self, color: Color) -> MoveList {
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

    fn generate_captures(&self, color: Color) -> MoveList {
        let mut moves = MoveList::new();
        self.gen_pawn_captures(color, &mut moves);
        self.gen_piece_captures(Piece::Knight, color, &mut moves);
        self.gen_sliding_captures(Piece::Bishop, color, &mut moves);
        self.gen_sliding_captures(Piece::Rook, color, &mut moves);
        self.gen_sliding_captures(Piece::Queen, color, &mut moves);
        self.gen_king_captures(color, &mut moves);
        moves
    }

    fn generate_pseudo_legal_quiets(&self, color: Color) -> MoveList {
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

    /// Geometry + occupancy, not king safety. TT/killer hydration uses this
    /// so a colliding from/to cannot pass `is_legal_move` and then corrupt make.
    fn is_pseudo_legal(&self, mv: Move) -> bool {
        if mv.from == mv.to {
            return false;
        }
        let us = self.side_to_move();
        let piece = match self.piece_of_color_on(mv.from, us) {
            Some(piece) => piece,
            None => return false,
        };
        if mv.is_castling() {
            return piece == Piece::King && self.castling_is_legal_after_move(mv, us);
        }
        if self.piece_of_color_on(mv.to, us).is_some() {
            return false;
        }
        let dest_enemy = self.piece_of_color_on(mv.to, us.opponent()).is_some();
        let from_idx = mv.from.index();
        let to_bb = mv.to.bitboard();
        let occ = self.all_occupancy();
        match piece {
            Piece::Pawn => {
                if mv.flag == MoveFlag::EnPassant {
                    return self.en_passant() == Some(mv.to)
                        && pawn_attacks(us.index(), from_idx) & to_bb != 0;
                }
                if dest_enemy {
                    pawn_attacks(us.index(), from_idx) & to_bb != 0
                        && (mv.to.rank() == us.promotion_rank()) == mv.is_promotion()
                } else if occ & to_bb == 0 {
                    let one = (from_idx as i32 + us.pawn_direction()) as usize;
                    if one < 64 && Square::from_index(one) == mv.to {
                        (mv.to.rank() == us.promotion_rank()) == mv.is_promotion()
                    } else if mv.flag == MoveFlag::DoublePawn
                        && mv.from.rank() == us.pawn_start_rank()
                    {
                        let mid = (from_idx as i32 + us.pawn_direction()) as usize;
                        let two = (from_idx as i32 + 2 * us.pawn_direction()) as usize;
                        two < 64
                            && Square::from_index(two) == mv.to
                            && occ & Square::from_index(mid).bitboard() == 0
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            Piece::Knight => knight_attacks(from_idx) & to_bb != 0 && !mv.is_promotion(),
            Piece::Bishop => bishop_attacks(from_idx, occ) & to_bb != 0 && !mv.is_promotion(),
            Piece::Rook => rook_attacks(from_idx, occ) & to_bb != 0 && !mv.is_promotion(),
            Piece::Queen => queen_attacks(from_idx, occ) & to_bb != 0 && !mv.is_promotion(),
            Piece::King => king_attacks(from_idx) & to_bb != 0 && !mv.is_promotion(),
        }
    }

    /// Fill in capture / EP / double-push / castle flags from the current mailbox.
    /// TT and killer moves are often stored with only from/to.
    fn hydrate_move(&self, mv: Move) -> Option<Move> {
        if mv.from == mv.to {
            return None;
        }
        let us = self.side_to_move();
        let piece = self.piece_of_color_on(mv.from, us)?;
        if piece == Piece::King {
            if mv.is_castling() {
                return self.is_legal_move(mv).then_some(mv);
            }
            if !self.chess960() {
                let castle = match (us, mv.from, mv.to) {
                    (Color::White, Square::E1, Square::G1)
                    | (Color::Black, Square::E8, Square::G8) => {
                        Some(Move::king_castle(mv.from, mv.to))
                    }
                    (Color::White, Square::E1, Square::C1)
                    | (Color::Black, Square::E8, Square::C8) => {
                        Some(Move::queen_castle(mv.from, mv.to))
                    }
                    _ => None,
                };
                if let Some(castle) = castle {
                    return self.is_legal_move(castle).then_some(castle);
                }
            } else if self.piece_of_color_on(mv.to, us) == Some(Piece::Rook) {
                let castle = if mv.to.file() > mv.from.file() {
                    Move::king_castle(mv.from, mv.to)
                } else {
                    Move::queen_castle(mv.from, mv.to)
                };
                return self.is_legal_move(castle).then_some(castle);
            }
        }
        let dest_enemy = self.piece_of_color_on(mv.to, us.opponent()).is_some();
        let resolved = if let Some(promo) = mv.promotion {
            if dest_enemy {
                Move::promotion_capture(mv.from, mv.to, promo)
            } else {
                Move::promotion(mv.from, mv.to, promo)
            }
        } else if piece == Piece::Pawn && self.en_passant() == Some(mv.to) {
            Move::en_passant(mv.from, mv.to)
        } else if dest_enemy {
            Move::capture(mv.from, mv.to)
        } else if piece == Piece::Pawn && mv.from.rank().abs_diff(mv.to.rank()) == 2 {
            Move::double_pawn(mv.from, mv.to)
        } else {
            Move::quiet(mv.from, mv.to)
        };
        (self.is_pseudo_legal(resolved) && self.is_legal_move(resolved)).then_some(resolved)
    }

    #[inline(always)]
    fn is_legal_move(&self, mv: Move) -> bool {
        let us = self.side_to_move();
        let them = us.opponent();
        let moving_piece = match self.piece_of_color_on(mv.from, us) {
            Some(piece) => piece,
            None => return false,
        };

        if mv.is_castling() {
            return moving_piece == Piece::King && self.castling_is_legal_after_move(mv, us);
        }
        if self.piece_of_color_on(mv.to, us).is_some() {
            return false;
        }
        let dest_enemy = self.piece_of_color_on(mv.to, them).is_some();
        if mv.flag == MoveFlag::EnPassant {
            if dest_enemy || self.en_passant() != Some(mv.to) {
                return false;
            }
        } else if mv.is_capture() {
            if !dest_enemy {
                return false;
            }
        } else if dest_enemy {
            return false;
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

    #[inline(always)]
    fn tt_hash(&self) -> u64 {
        tt_hash_with_clock(self.hash(), self.halfmove_clock())
    }

    #[inline(always)]
    fn tt_hash_after(&self, mv: Move) -> u64 {
        let z = zobrist();
        let us = self.side_to_move();
        let them = us.opponent();
        let from = mv.from;
        let to = mv.to;
        let piece = self
            .piece_of_color_on(from, us)
            .expect("tt_hash_after: no piece on source square");
        let mut hash = self.hash();

        if let Some(ep) = self.en_passant() {
            hash ^= z.en_passant_keys[ep.file() as usize];
        }

        let captured = match mv.flag {
            MoveFlag::Capture | MoveFlag::PromotionCapture => {
                self.piece_of_color_on(to, them).inspect(|captured| {
                    hash ^= z.piece_keys[them.index()][captured.index()][to.index()];
                })
            }
            MoveFlag::EnPassant => {
                let captured_square = Square::from_file_rank(to.file(), from.rank());
                hash ^= z.piece_keys[them.index()][Piece::Pawn.index()][captured_square.index()];
                Some(Piece::Pawn)
            }
            _ => None,
        };

        hash ^= z.piece_keys[us.index()][piece.index()][from.index()];
        if let Some(promotion) = mv.promotion {
            hash ^= z.piece_keys[us.index()][promotion.index()][to.index()];
        } else {
            hash ^= z.piece_keys[us.index()][piece.index()][to.index()];
        }

        if mv.is_castling() {
            let kingside = mv.flag == MoveFlag::KingCastle;
            let king_to = Board::castling_king_landing(us, kingside);
            if king_to != to {
                hash ^= z.piece_keys[us.index()][piece.index()][to.index()];
                hash ^= z.piece_keys[us.index()][piece.index()][king_to.index()];
            }
            let rook_from = self.castling_rook_from(us, kingside);
            let rook_to = Board::castling_rook_landing(us, kingside);
            hash ^= z.piece_keys[us.index()][Piece::Rook.index()][rook_from.index()];
            hash ^= z.piece_keys[us.index()][Piece::Rook.index()][rook_to.index()];
        }

        if mv.flag == MoveFlag::DoublePawn {
            let ep_rank = (from.rank() as i32 + (to.rank() as i32 - from.rank() as i32) / 2) as u8;
            let ep_square = Square::from_file_rank(from.file(), ep_rank);
            hash ^= z.en_passant_keys[ep_square.file() as usize];
        }

        let old_castling = self.castling_rights();
        let new_castling = self.castling_rights_after(from, to);
        if old_castling != new_castling {
            hash ^= z.castling_keys[old_castling as usize];
            hash ^= z.castling_keys[new_castling as usize];
        }

        let halfmove_clock = if piece == Piece::Pawn || captured.is_some() {
            0
        } else {
            self.halfmove_clock() + 1
        };
        let bucket = (halfmove_clock.saturating_sub(8) as usize / 8).min(15);
        hash ^ z.side_to_move_key ^ z.fiftymove_keys[bucket]
    }

    #[inline(always)]
    fn castling_rights_after(&self, from: Square, to: Square) -> u8 {
        apply_castling_rights_update(
            self.chess960(),
            self.castling_rights(),
            [
                self.castling_king_file(Color::White),
                self.castling_king_file(Color::Black),
            ],
            |color, kingside| self.castling_rook_from(color, kingside),
            from,
            to,
        )
    }

    #[inline(always)]
    fn castling_is_legal_after_move(&self, mv: Move, us: Color) -> bool {
        let kingside = match mv.flag {
            MoveFlag::KingCastle => true,
            MoveFlag::QueenCastle => false,
            _ => return false,
        };
        self.can_castle(us, kingside)
            && self.king_square(us) == mv.from
            && mv.to == self.castle_uci_to(us, kingside)
    }

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

            let to_idx = (from_idx as i32 + dir) as usize;
            if to_idx < 64 {
                let to = Square::from_index(to_idx);
                let to_bb = to.bitboard();

                if occ & to_bb == 0 {
                    if to.rank() == promo_rank {
                        for promo_piece in [Piece::Queen, Piece::Rook, Piece::Bishop, Piece::Knight]
                        {
                            moves.push(Move::promotion(from, to, promo_piece));
                        }
                    } else {
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

            if let Some(ep_sq) = self.en_passant()
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

    fn gen_castling_moves(&self, color: Color, moves: &mut MoveList) {
        if self.castling_rights() == 0 {
            return;
        }
        if self.can_castle(color, true) {
            let from = self.king_square(color);
            moves.push(Move::king_castle(from, self.castle_uci_to(color, true)));
        }
        if self.can_castle(color, false) {
            let from = self.king_square(color);
            moves.push(Move::queen_castle(from, self.castle_uci_to(color, false)));
        }
    }

    fn can_castle(&self, color: Color, kingside: bool) -> bool {
        let right = match (color, kingside) {
            (Color::White, true) => WHITE_KING_CASTLE,
            (Color::White, false) => WHITE_QUEEN_CASTLE,
            (Color::Black, true) => BLACK_KING_CASTLE,
            (Color::Black, false) => BLACK_QUEEN_CASTLE,
        };
        if self.castling_rights() & right == 0 {
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

    fn gen_pawn_captures(&self, color: Color, moves: &mut MoveList) {
        let pawns = self.piece_bb(Piece::Pawn, color);
        let enemies = self.color_occupancy(color.opponent());
        let promo_rank = color.promotion_rank();

        for from_idx in iter_bits(pawns) {
            let from = Square::from_index(from_idx);
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

            if let Some(ep_sq) = self.en_passant()
                && pawn_atk & ep_sq.bitboard() != 0
            {
                moves.push(Move::en_passant(from, ep_sq));
            }

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

#[inline(always)]
pub(super) fn tt_hash_with_clock(hash: u64, halfmove_clock: u32) -> u64 {
    let bucket = (halfmove_clock.saturating_sub(8) as usize / 8).min(15);
    hash ^ zobrist().fiftymove_keys[bucket]
}

macro_rules! impl_mailbox_accessors {
    ($ty:ty) => {
        impl MailboxPos for $ty {
            #[inline(always)]
            fn pieces(&self) -> &[[Bitboard; 6]; 2] {
                &self.pieces
            }

            #[inline(always)]
            fn occupancy(&self) -> &[Bitboard; 2] {
                &self.occupancy
            }

            #[inline(always)]
            fn piece_at(&self, square: Square) -> u8 {
                self.piece_at[square.index()]
            }

            #[inline(always)]
            fn side_to_move(&self) -> Color {
                self.side_to_move
            }

            #[inline(always)]
            fn en_passant(&self) -> Option<Square> {
                self.en_passant
            }

            #[inline(always)]
            fn castling_rights(&self) -> u8 {
                self.castling_rights
            }

            #[inline(always)]
            fn chess960(&self) -> bool {
                self.chess960
            }

            #[inline(always)]
            fn castling_king_file(&self, color: Color) -> u8 {
                self.castling_king_file[color.index()]
            }

            #[inline(always)]
            fn castling_rook_file(&self, color: Color, kingside: bool) -> u8 {
                self.castling_rook_file[Board::rook_file_index(color, kingside)]
            }

            #[inline(always)]
            fn hash(&self) -> u64 {
                self.hash
            }

            #[inline(always)]
            fn halfmove_clock(&self) -> u32 {
                self.halfmove_clock
            }
        }
    };
}

impl_mailbox_accessors!(Board);
impl_mailbox_accessors!(BoardSnapshot);

impl Board {
    pub fn is_square_attacked(&self, sq: Square, by_color: Color) -> bool {
        MailboxPos::is_square_attacked(self, sq, by_color)
    }

    #[inline(always)]
    pub fn is_square_attacked_after(
        &self,
        sq: Square,
        by_color: Color,
        occupancy: Bitboard,
        removed_attackers: Bitboard,
    ) -> bool {
        MailboxPos::is_square_attacked_after(self, sq, by_color, occupancy, removed_attackers)
    }

    pub fn generate_pseudo_legal_moves(&self, color: Color) -> MoveList {
        MailboxPos::generate_pseudo_legal_moves(self, color)
    }

    pub fn generate_captures(&self, color: Color) -> MoveList {
        MailboxPos::generate_captures(self, color)
    }

    pub fn generate_pseudo_legal_quiets(&self, color: Color) -> MoveList {
        MailboxPos::generate_pseudo_legal_quiets(self, color)
    }

    #[inline(always)]
    pub fn is_legal_move(&self, mv: Move) -> bool {
        MailboxPos::is_legal_move(self, mv)
    }

    #[inline]
    pub fn hydrate_move(&self, mv: Move) -> Option<Move> {
        MailboxPos::hydrate_move(self, mv)
    }
}

impl BoardSnapshot {
    pub fn is_square_attacked(&self, sq: Square, by_color: Color) -> bool {
        MailboxPos::is_square_attacked(self, sq, by_color)
    }

    #[inline(always)]
    pub fn is_square_attacked_after(
        &self,
        sq: Square,
        by_color: Color,
        occupancy: Bitboard,
        removed_attackers: Bitboard,
    ) -> bool {
        MailboxPos::is_square_attacked_after(self, sq, by_color, occupancy, removed_attackers)
    }

    pub fn generate_pseudo_legal_moves(&self, color: Color) -> MoveList {
        MailboxPos::generate_pseudo_legal_moves(self, color)
    }

    pub fn generate_captures(&self, color: Color) -> MoveList {
        MailboxPos::generate_captures(self, color)
    }

    pub fn generate_pseudo_legal_quiets(&self, color: Color) -> MoveList {
        MailboxPos::generate_pseudo_legal_quiets(self, color)
    }

    #[inline(always)]
    pub fn is_legal_move(&self, mv: Move) -> bool {
        MailboxPos::is_legal_move(self, mv)
    }

    #[inline]
    pub fn hydrate_move(&self, mv: Move) -> Option<Move> {
        MailboxPos::hydrate_move(self, mv)
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
    use super::super::{AkimboPos, Board, BoardSnapshot, compute_opponent_attacks};
    use super::MailboxPos;
    use crate::chess_move::Move;
    use crate::piece::{Color, Piece};
    use crate::square::Square;

    fn setup() {
        crate::init();
    }

    const STARTPOS: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    const KIWIPETE: &str = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
    const EP_POS: &str = "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPP1PPP/RNBQKBNR w KQkq f6 0 3";

    fn assert_moves_match(board: &Board, snap: &BoardSnapshot, fen: &str) {
        let color = board.side_to_move;
        assert_eq!(
            snap.generate_captures(color).as_slice(),
            board.generate_captures(color).as_slice(),
            "captures {fen}"
        );
        assert_eq!(
            snap.generate_pseudo_legal_quiets(color).as_slice(),
            board.generate_pseudo_legal_quiets(color).as_slice(),
            "quiets {fen}"
        );
        assert_eq!(
            snap.generate_pseudo_legal_moves(color).as_slice(),
            board.generate_pseudo_legal_moves(color).as_slice(),
            "pseudos {fen}"
        );
        for &mv in board.generate_pseudo_legal_moves(color).as_slice() {
            assert_eq!(
                snap.is_legal_move(mv),
                board.is_legal_move(mv),
                "is_legal_move {} {fen}",
                mv.to_uci()
            );
        }
    }

    #[test]
    fn snapshot_generate_matches_board_on_startpos_kiwipete_and_ep() {
        setup();
        for fen in [STARTPOS, KIWIPETE, EP_POS] {
            let board = Board::from_fen(fen).unwrap();
            let snap = board.snapshot();
            assert_moves_match(&board, &snap, fen);
        }
    }

    #[test]
    fn snapshot_make_e2e4_matches_board_generate_and_not_in_check() {
        setup();
        let mut board = Board::new();
        let mv = board
            .generate_pseudo_legal_moves(board.side_to_move)
            .iter()
            .copied()
            .find(|candidate| candidate.to_uci() == "e2e4")
            .expect("e2e4");
        let mut snap = board.snapshot();
        assert!(!snap.make(mv));
        assert!(!snap.in_check());
        board.make_move(mv);
        assert_moves_match(&board, &snap, "startpos after e2e4");
    }

    #[test]
    fn snapshot_make_does_not_refresh_threats() {
        setup();
        let board = Board::new();
        let mut snap = board.snapshot();
        let threats_before = snap.threats;
        let mv = Move::double_pawn(Square::E2, Square::E4);
        assert!(!snap.make(mv));
        assert_eq!(snap.threats, threats_before);
    }

    #[test]
    fn snapshot_accessors_match_board() {
        setup();
        for fen in [STARTPOS, KIWIPETE, EP_POS] {
            let mut board = Board::from_fen(fen).unwrap();
            let snap = board.snapshot();
            for sq in Square::ALL {
                assert_eq!(snap.piece_on(sq), board.piece_on(sq), "{fen} {sq}");
            }
            assert_eq!(
                snap.color_occupancy(Color::White),
                board.color_occupancy(Color::White),
                "{fen}"
            );
            assert_eq!(
                snap.color_occupancy(Color::Black),
                board.color_occupancy(Color::Black),
                "{fen}"
            );
            assert_eq!(snap.is_chess960(), board.is_chess960(), "{fen}");
            assert_eq!(snap.tt_hash(), board.tt_hash(), "{fen}");
            assert_eq!(
                snap.snapshot12(),
                [
                    board.piece_bb(Piece::Pawn, Color::White),
                    board.piece_bb(Piece::Knight, Color::White),
                    board.piece_bb(Piece::Bishop, Color::White),
                    board.piece_bb(Piece::Rook, Color::White),
                    board.piece_bb(Piece::Queen, Color::White),
                    board.piece_bb(Piece::King, Color::White),
                    board.piece_bb(Piece::Pawn, Color::Black),
                    board.piece_bb(Piece::Knight, Color::Black),
                    board.piece_bb(Piece::Bishop, Color::Black),
                    board.piece_bb(Piece::Rook, Color::Black),
                    board.piece_bb(Piece::Queen, Color::Black),
                    board.piece_bb(Piece::King, Color::Black),
                ],
                "{fen}"
            );
            assert_eq!(
                snap.opponent_attacks(),
                compute_opponent_attacks(&snap.pieces, snap.occupancy, snap.side_to_move),
                "{fen}"
            );
            assert_eq!(snap.opponent_attacks(), board.opponent_attacks(), "{fen}");
            for &mv in board.generate_legal_moves().iter() {
                assert_eq!(
                    snap.tt_hash_after(mv),
                    board.tt_hash_after(mv),
                    "tt_hash_after {} {fen}",
                    mv.to_uci()
                );
            }
        }
    }

    #[test]
    fn snapshot_is_search_draw_halfmove_material_and_history() {
        setup();
        let start = Board::new().snapshot();
        assert!(!start.is_search_draw(&[], &[], 0));

        let fifty = Board::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 100 1")
            .unwrap()
            .snapshot();
        assert!(fifty.is_search_draw(&[], &[], 0));

        let bare_kings = Board::from_fen("8/8/4k3/8/8/4K3/8/8 w - - 0 1")
            .unwrap()
            .snapshot();
        assert!(bare_kings.is_search_draw(&[], &[], 0));

        let mut board = Board::new();
        for uci in ["g1f3", "g8f6", "f3g1", "f6g8"] {
            let mv = board
                .generate_legal_moves()
                .iter()
                .copied()
                .find(|candidate| candidate.to_uci() == uci)
                .unwrap_or_else(|| panic!("{uci}"));
            board.make_move(mv);
        }
        let snap = board.snapshot();
        let akimbo = AkimboPos::from_board(&board);
        let hist = board.hash_history.as_slice();
        assert_eq!(
            snap.is_search_draw(hist, &[], 4),
            akimbo.is_search_draw(hist, &[], 4)
        );
        assert_eq!(
            snap.is_search_draw(hist, &[], 5),
            akimbo.is_search_draw(hist, &[], 5)
        );
        let (root, path) = hist.split_at(2);
        assert_eq!(
            snap.is_search_draw(root, path, 5),
            akimbo.is_search_draw(root, path, 5)
        );
    }

    #[test]
    fn mailbox_trait_matches_board_inherent_without_importing_on_callers() {
        setup();
        let board = Board::new();
        assert_eq!(
            MailboxPos::generate_captures(&board, Color::White).as_slice(),
            board.generate_captures(Color::White).as_slice()
        );
    }

    #[test]
    fn hydrate_move_recovers_capture_ep_and_double_push_flags() {
        setup();
        let start = Board::new();
        let e2e4 = start
            .hydrate_move(Move::quiet(Square::E2, Square::E4))
            .expect("e2e4");
        assert_eq!(e2e4.flag, crate::chess_move::MoveFlag::DoublePawn);
        assert_eq!(start.snapshot().hydrate_move(e2e4), Some(e2e4));

        let kiwi = Board::from_fen(KIWIPETE).unwrap();
        let capture = kiwi
            .hydrate_move(Move::quiet(Square::E5, Square::D7))
            .expect("Nxd7");
        assert_eq!(capture.flag, crate::chess_move::MoveFlag::Capture);
        assert_eq!(kiwi.snapshot().hydrate_move(capture), Some(capture));

        let ep_board = Board::from_fen(EP_POS).unwrap();
        let ep = ep_board
            .hydrate_move(Move::quiet(Square::E5, Square::F6))
            .expect("exf6 ep");
        assert_eq!(ep.flag, crate::chess_move::MoveFlag::EnPassant);
        assert_eq!(ep_board.snapshot().hydrate_move(ep), Some(ep));
        assert!(
            ep_board
                .hydrate_move(Move::quiet(Square::A1, Square::A8))
                .is_none()
        );

        let castle = kiwi
            .hydrate_move(Move::quiet(Square::E1, Square::G1))
            .expect("O-O");
        assert_eq!(castle.flag, crate::chess_move::MoveFlag::KingCastle);
    }

    #[test]
    fn castle_without_rook_is_rejected_even_when_rights_remain() {
        setup();
        let board = Board::from_fen("4k3/8/8/8/8/8/8/4K3 b k - 0 1").unwrap();
        let castle = Move::king_castle(Square::E8, Square::G8);
        assert!(!board.is_legal_move(castle));
        assert!(board.hydrate_move(castle).is_none());
        assert!(
            board
                .hydrate_move(Move::quiet(Square::E8, Square::G8))
                .is_none()
        );
        assert!(
            !board
                .generate_pseudo_legal_quiets(Color::Black)
                .as_slice()
                .iter()
                .any(|mv| mv.is_castling())
        );
        let kiwi = Board::from_fen(KIWIPETE).unwrap();
        assert!(
            kiwi.hydrate_move(Move::quiet(Square::E1, Square::G1))
                .is_some()
        );
    }

    #[test]
    fn is_legal_move_rejects_king_onto_own_castled_rook() {
        setup();
        let board =
            Board::from_fen("rnbqk2r/ppp1ppbp/5np1/3p4/3P4/2N2N2/PPP1PPPP/R1BQ1RK1 w kq - 2 5")
                .unwrap();
        let onto_rook = Move::quiet(Square::G1, Square::F1);
        assert!(!board.is_legal_move(onto_rook));
        assert!(board.hydrate_move(onto_rook).is_none());
        assert!(!board.snapshot().is_legal_move(onto_rook));
        assert!(
            !board
                .generate_pseudo_legal_quiets(Color::White)
                .as_slice()
                .iter()
                .any(|mv| mv.from == Square::G1 && mv.to == Square::F1)
        );
    }
}
