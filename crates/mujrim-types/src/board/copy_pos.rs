//! Official Akimbo-style `Copy` position: make on a child, never write the parent.

use super::zobrist::zobrist;
use super::{
    BoardSnapshot, EMPTY_PIECE_ID, MailboxPos, apply_castling_rights_update,
    compute_opponent_attacks, evaluation_material_value, history_hash_at,
};
use crate::bitboard::{clear_bit, get_lsb, set_bit};
use crate::chess_move::{Move, MoveFlag};
use crate::piece::{Color, Piece};
use crate::square::Square;

impl BoardSnapshot {
    /// Official `let mut new = *pos; new.make(mv)`.
    ///
    /// Returns `true` when the mover's king is left in check (illegal).
    #[inline]
    pub fn make(&mut self, mv: Move) -> bool {
        let z = zobrist();
        let us = self.side_to_move;
        let them = us.opponent();
        let from = mv.from;
        let to = mv.to;
        let piece = self
            .piece_of_color_on(from, us)
            .expect("BoardSnapshot::make: no piece on source");

        if let Some(ep) = self.en_passant {
            self.hash ^= z.en_passant_keys[ep.file() as usize];
        }
        self.en_passant = None;

        let mut captured = None;
        match mv.flag {
            MoveFlag::Capture | MoveFlag::PromotionCapture => {
                if let Some(cap_piece) = self.piece_of_color_on(to, them) {
                    self.remove_piece(piece, us, from);
                    self.remove_piece(cap_piece, them, to);
                    self.put_piece(piece, us, to);
                    captured = Some(cap_piece);
                }
            }
            _ => {}
        }

        if captured.is_none() && !mv.is_castling() {
            self.relocate_piece(piece, us, from, to);
        }

        if mv.flag == MoveFlag::EnPassant {
            let cap_sq = Square::from_file_rank(to.file(), from.rank());
            self.remove_piece(Piece::Pawn, them, cap_sq);
            captured = Some(Piece::Pawn);
        }

        if let Some(promotion) = mv.promotion {
            self.remove_piece(piece, us, to);
            self.put_piece(promotion, us, to);
        }

        if mv.is_castling() {
            let kingside = mv.flag == MoveFlag::KingCastle;
            let king_to = castling_king_landing(us, kingside);
            let rook_from = self.castling_rook_from(us, kingside);
            let rook_to = castling_rook_landing(us, kingside);
            if from != king_to {
                self.relocate_piece(piece, us, from, king_to);
            }
            if rook_from != rook_to {
                self.relocate_piece(Piece::Rook, us, rook_from, rook_to);
            }
        }

        if mv.flag == MoveFlag::DoublePawn {
            let ep_rank = (from.rank() as i32 + (to.rank() as i32 - from.rank() as i32) / 2) as u8;
            let ep_sq = Square::from_file_rank(from.file(), ep_rank);
            self.en_passant = Some(ep_sq);
            self.hash ^= z.en_passant_keys[ep_sq.file() as usize];
        }

        let old_castling = self.castling_rights;
        self.castling_rights = self.castling_rights_after(from, to);
        if old_castling != self.castling_rights {
            self.hash ^= z.castling_keys[old_castling as usize];
            self.hash ^= z.castling_keys[self.castling_rights as usize];
        }

        if piece == Piece::Pawn || captured.is_some() {
            self.halfmove_clock = 0;
        } else {
            self.halfmove_clock += 1;
        }
        self.plies_from_null += 1;
        self.hash_history_len += 1;
        if us == Color::Black {
            self.fullmove_number += 1;
        }
        self.side_to_move = them;
        self.hash ^= z.side_to_move_key;
        self.is_in_check(us)
    }

    #[inline]
    pub fn make_null(&mut self) {
        let z = zobrist();
        if let Some(ep) = self.en_passant {
            self.hash ^= z.en_passant_keys[ep.file() as usize];
        }
        self.en_passant = None;
        self.plies_from_null = 0;
        self.hash_history_len += 1;
        self.side_to_move = self.side_to_move.opponent();
        self.hash ^= z.side_to_move_key;
    }

    #[inline]
    pub fn in_check(&self) -> bool {
        self.is_in_check(self.side_to_move)
    }

    #[inline]
    pub fn is_in_check(&self, color: Color) -> bool {
        self.is_square_attacked(self.king_square(color), color.opponent())
    }

    #[inline]
    pub fn king_square(&self, color: Color) -> Square {
        let king_bb = self.pieces[color.index()][Piece::King.index()];
        Square::from_index(get_lsb(king_bb))
    }

    #[inline]
    pub fn hash(&self) -> u64 {
        self.hash
    }

    #[inline]
    pub fn side_to_move(&self) -> Color {
        self.side_to_move
    }

    #[inline]
    pub fn halfmove_clock(&self) -> u32 {
        self.halfmove_clock
    }

    #[inline]
    pub fn plies_from_null(&self) -> usize {
        self.plies_from_null
    }

    #[inline]
    pub fn piece_bb(&self, piece: Piece, color: Color) -> u64 {
        self.pieces[color.index()][piece.index()]
    }

    #[inline]
    pub fn all_occupancy(&self) -> u64 {
        self.occupancy[0] | self.occupancy[1]
    }

    #[inline]
    pub fn piece_of_color_on(&self, square: Square, color: Color) -> Option<Piece> {
        let id = self.piece_at[square.index()];
        (id & 1 == color.index() as u8)
            .then(|| Piece::from_index(usize::from(id) / 2))
            .flatten()
    }

    #[inline]
    pub fn piece_on(&self, square: Square) -> Option<(Piece, Color)> {
        let id = self.piece_at[square.index()];
        let piece = Piece::from_index(usize::from(id) / 2)?;
        let color = if id & 1 == 0 {
            Color::White
        } else {
            Color::Black
        };
        Some((piece, color))
    }

    #[inline]
    pub fn color_occupancy(&self, color: Color) -> u64 {
        self.occupancy[color.index()]
    }

    #[inline]
    pub const fn is_chess960(&self) -> bool {
        self.chess960
    }

    #[inline]
    pub fn castle_uci_to(&self, color: Color, kingside: bool) -> Square {
        if self.chess960 {
            self.castling_rook_from(color, kingside)
        } else {
            castling_king_landing(color, kingside)
        }
    }

    #[inline(always)]
    pub fn tt_hash(&self) -> u64 {
        MailboxPos::tt_hash(self)
    }

    #[inline(always)]
    pub fn tt_hash_after(&self, mv: Move) -> u64 {
        MailboxPos::tt_hash_after(self, mv)
    }

    #[inline]
    pub fn snapshot12(&self) -> [u64; 12] {
        [
            self.piece_bb(Piece::Pawn, Color::White),
            self.piece_bb(Piece::Knight, Color::White),
            self.piece_bb(Piece::Bishop, Color::White),
            self.piece_bb(Piece::Rook, Color::White),
            self.piece_bb(Piece::Queen, Color::White),
            self.piece_bb(Piece::King, Color::White),
            self.piece_bb(Piece::Pawn, Color::Black),
            self.piece_bb(Piece::Knight, Color::Black),
            self.piece_bb(Piece::Bishop, Color::Black),
            self.piece_bb(Piece::Rook, Color::Black),
            self.piece_bb(Piece::Queen, Color::Black),
            self.piece_bb(Piece::King, Color::Black),
        ]
    }

    #[inline]
    pub fn opponent_attacks(&self) -> u64 {
        compute_opponent_attacks(&self.pieces, self.occupancy, self.side_to_move)
    }

    #[inline]
    pub fn has_non_pawn_material(&self, color: Color) -> bool {
        let side_occ = self.color_occupancy(color);
        let pawn_king = self.piece_bb(Piece::Pawn, color) | self.piece_bb(Piece::King, color);
        (side_occ & !pawn_king) != 0
    }

    pub fn is_search_draw(&self, root_history: &[u64], path: &[u64], ply: usize) -> bool {
        if self.halfmove_clock >= 100 {
            return true;
        }
        if self.is_insufficient_material() {
            return true;
        }
        let total = root_history.len() + path.len();
        if total < 4 {
            return false;
        }
        let check_len = (self.halfmove_clock as usize)
            .min(self.plies_from_null)
            .min(total);
        let mut matches = 0;
        let mut i = 4;
        while i <= check_len {
            let hist = history_hash_at(root_history, path, total - i);
            if hist == self.hash {
                if i < ply {
                    return true;
                }
                matches += 1;
                if matches >= 2 {
                    return true;
                }
            }
            i += 2;
        }
        false
    }

    #[inline]
    pub fn is_insufficient_material(&self) -> bool {
        let total = self.all_occupancy().count_ones();
        if total == 2 {
            return true;
        }
        if total == 3 {
            for color in [Color::White, Color::Black] {
                if self.piece_bb(Piece::Knight, color).count_ones() == 1
                    || self.piece_bb(Piece::Bishop, color).count_ones() == 1
                {
                    return true;
                }
            }
        }
        false
    }

    #[inline]
    fn put_piece(&mut self, piece: Piece, color: Color, square: Square) {
        set_bit(
            &mut self.pieces[color.index()][piece.index()],
            square.index(),
        );
        set_bit(&mut self.occupancy[color.index()], square.index());
        self.piece_at[square.index()] = (piece.index() * 2 + color.index()) as u8;
        self.total_material += evaluation_material_value(piece);
        self.hash ^= zobrist().piece_keys[color.index()][piece.index()][square.index()];
    }

    #[inline]
    fn remove_piece(&mut self, piece: Piece, color: Color, square: Square) {
        clear_bit(
            &mut self.pieces[color.index()][piece.index()],
            square.index(),
        );
        clear_bit(&mut self.occupancy[color.index()], square.index());
        self.piece_at[square.index()] = EMPTY_PIECE_ID;
        self.total_material -= evaluation_material_value(piece);
        self.hash ^= zobrist().piece_keys[color.index()][piece.index()][square.index()];
    }

    #[inline]
    fn relocate_piece(&mut self, piece: Piece, color: Color, from: Square, to: Square) {
        clear_bit(&mut self.pieces[color.index()][piece.index()], from.index());
        set_bit(&mut self.pieces[color.index()][piece.index()], to.index());
        clear_bit(&mut self.occupancy[color.index()], from.index());
        set_bit(&mut self.occupancy[color.index()], to.index());
        self.piece_at[from.index()] = EMPTY_PIECE_ID;
        self.piece_at[to.index()] = (piece.index() * 2 + color.index()) as u8;
        self.hash ^= zobrist().piece_keys[color.index()][piece.index()][from.index()];
        self.hash ^= zobrist().piece_keys[color.index()][piece.index()][to.index()];
    }

    fn castling_rights_after(&self, from: Square, to: Square) -> u8 {
        apply_castling_rights_update(
            self.chess960,
            self.castling_rights,
            self.castling_king_file,
            |color, kingside| self.castling_rook_from(color, kingside),
            from,
            to,
        )
    }

    #[inline]
    pub fn castling_rook_from(&self, color: Color, kingside: bool) -> Square {
        let idx = match (color, kingside) {
            (Color::White, true) => 0,
            (Color::White, false) => 1,
            (Color::Black, true) => 2,
            (Color::Black, false) => 3,
        };
        let file = self.castling_rook_file[idx];
        let rank = if color == Color::White { 0 } else { 7 };
        Square::from_file_rank(file, rank)
    }
}

#[inline]
fn castling_king_landing(color: Color, kingside: bool) -> Square {
    match (color, kingside) {
        (Color::White, true) => Square::G1,
        (Color::White, false) => Square::C1,
        (Color::Black, true) => Square::G8,
        (Color::Black, false) => Square::C8,
    }
}

#[inline]
fn castling_rook_landing(color: Color, kingside: bool) -> Square {
    match (color, kingside) {
        (Color::White, true) => Square::F1,
        (Color::White, false) => Square::D1,
        (Color::Black, true) => Square::F8,
        (Color::Black, false) => Square::D8,
    }
}

const _: () = assert!(std::mem::size_of::<BoardSnapshot>() <= 256);
const _: () = assert!(
    std::mem::offset_of!(super::Board, history) == std::mem::offset_of!(BoardSnapshot, history_len)
);
