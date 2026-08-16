//! Official jw1912/akimbo `Position`: 88-byte `Copy`, XOR-toggle make.

use super::attack_tables::{
    bishop_attacks, king_attacks, knight_attacks, pawn_attacks, queen_attacks, rook_attacks,
};
use super::zobrist::zobrist;
use super::{
    BLACK_KING_CASTLE, BLACK_QUEEN_CASTLE, Board, CASTLING_RIGHTS_UPDATE, WHITE_KING_CASTLE,
    WHITE_QUEEN_CASTLE,
};
use crate::bitboard::{count_bits, iter_bits};
use crate::chess_move::{Move, MoveFlag, MoveList};
use crate::piece::{Color, Piece};
use crate::square::Square;

const SIDE_WHITE: usize = 0;
const SIDE_BLACK: usize = 1;
const PC_PAWN: usize = 2;
const NONE_EP: u8 = 64;

/// Official `bb: [u64; 8]` plus the scalars Akimbo keeps on `Position`.
///
/// Official stays at 88 bytes because their `Move` carries `moved_pc`. Ours
/// generate mailbox-free `Move`s, so a compact `[u8; 64]` piece-at keeps
/// `piece_on` O(1) without writing the full `Board`.
#[derive(Clone, Copy, Debug)]
pub struct AkimboPos {
    bb: [u64; 8],
    hash: u64,
    stm: Color,
    halfmove: u8,
    ep: u8,
    rights: u8,
    plies_from_null: u8,
    piece_at: [u8; 64],
}

impl AkimboPos {
    #[inline]
    pub fn from_board(board: &Board) -> Self {
        let mut bb = [0u64; 8];
        bb[SIDE_WHITE] = board.occupancy[SIDE_WHITE];
        bb[SIDE_BLACK] = board.occupancy[SIDE_BLACK];
        for piece in Piece::ALL {
            bb[PC_PAWN + piece.index()] =
                board.pieces[SIDE_WHITE][piece.index()] | board.pieces[SIDE_BLACK][piece.index()];
        }
        let mut piece_at = [super::EMPTY_PIECE_ID; 64];
        piece_at.copy_from_slice(board.piece_ids());
        Self {
            bb,
            hash: board.hash,
            stm: board.side_to_move,
            halfmove: board.halfmove_clock.min(u32::from(u8::MAX)) as u8,
            ep: board
                .en_passant
                .map(|sq| sq.index() as u8)
                .unwrap_or(NONE_EP),
            rights: board.castling_rights,
            plies_from_null: board.plies_from_null().min(usize::from(u8::MAX)) as u8,
            piece_at,
        }
    }

    #[inline]
    pub fn side_to_move(&self) -> Color {
        self.stm
    }

    #[inline]
    pub fn hash(&self) -> u64 {
        self.hash
    }

    #[inline]
    pub fn tt_hash(&self) -> u64 {
        let bucket = (u32::from(self.halfmove).saturating_sub(8) as usize / 8).min(15);
        self.hash ^ zobrist().fiftymove_keys[bucket]
    }

    #[inline]
    pub fn halfmove_clock(&self) -> u32 {
        u32::from(self.halfmove)
    }

    #[inline]
    pub fn plies_from_null(&self) -> usize {
        usize::from(self.plies_from_null)
    }

    #[inline]
    pub fn piece_bb(&self, piece: Piece, color: Color) -> u64 {
        self.bb[PC_PAWN + piece.index()] & self.bb[color.index()]
    }

    #[inline]
    pub fn color_occupancy(&self, color: Color) -> u64 {
        self.bb[color.index()]
    }

    #[inline]
    pub fn all_occupancy(&self) -> u64 {
        self.bb[SIDE_WHITE] | self.bb[SIDE_BLACK]
    }

    /// Official `Position.bb`: `[white, black, pawn, knight, bishop, rook, queen, king]`.
    #[inline]
    pub fn bb8(&self) -> [u64; 8] {
        self.bb
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
    pub fn king_square(&self, color: Color) -> Square {
        Square::from_index((self.piece_bb(Piece::King, color)).trailing_zeros() as usize)
    }

    #[inline]
    pub fn piece_on(&self, sq: Square) -> Option<(Piece, Color)> {
        let id = self.piece_at[sq.index()];
        let piece = Piece::from_index(usize::from(id) / 2)?;
        let color = if id & 1 == 0 {
            Color::White
        } else {
            Color::Black
        };
        Some((piece, color))
    }

    #[inline(always)]
    pub fn piece_of_color_on(&self, sq: Square, color: Color) -> Option<Piece> {
        let id = self.piece_at[sq.index()];
        (id & 1 == color.index() as u8)
            .then(|| Piece::from_index(usize::from(id) / 2))
            .flatten()
    }

    #[inline]
    pub fn has_non_pawn_material(&self, color: Color) -> bool {
        let pk = self.piece_bb(Piece::Pawn, color) | self.piece_bb(Piece::King, color);
        self.bb[color.index()] & !pk != 0
    }

    #[inline]
    pub fn threats(&self) -> u64 {
        let mut pieces = [[0u64; 6]; 2];
        for piece in Piece::ALL {
            pieces[0][piece.index()] = self.piece_bb(piece, Color::White);
            pieces[1][piece.index()] = self.piece_bb(piece, Color::Black);
        }
        super::compute_opponent_attacks(&pieces, [self.bb[0], self.bb[1]], self.stm)
    }

    #[inline]
    fn toggle(&mut self, color: Color, piece: Piece, sq: Square) {
        let bit = sq.bitboard();
        self.bb[color.index()] ^= bit;
        self.bb[PC_PAWN + piece.index()] ^= bit;
        self.hash ^= zobrist().piece_keys[color.index()][piece.index()][sq.index()];
        let idx = sq.index();
        if self.piece_at[idx] == super::EMPTY_PIECE_ID {
            self.piece_at[idx] = (piece.index() * 2 + color.index()) as u8;
        } else {
            debug_assert_eq!(
                self.piece_at[idx],
                (piece.index() * 2 + color.index()) as u8
            );
            self.piece_at[idx] = super::EMPTY_PIECE_ID;
        }
    }

    /// Official `make`: returns `true` when the mover's king is left in check.
    #[inline]
    pub fn make(&mut self, mv: Move) -> bool {
        let z = zobrist();
        let us = self.stm;
        let them = us.opponent();
        let from = mv.from;
        let to = mv.to;
        let piece = self
            .piece_of_color_on(from, us)
            .expect("AkimboPos::make: no piece on source");

        if self.ep < NONE_EP {
            self.hash ^= z.en_passant_keys[Square::from_index(self.ep as usize).file() as usize];
        }
        self.ep = NONE_EP;

        let mut captured = None;
        match mv.flag {
            MoveFlag::Capture | MoveFlag::PromotionCapture => {
                if let Some(cap) = self.piece_of_color_on(to, them) {
                    self.toggle(them, cap, to);
                    captured = Some(cap);
                }
            }
            MoveFlag::EnPassant => {
                let cap_sq = Square::from_file_rank(to.file(), from.rank());
                self.toggle(them, Piece::Pawn, cap_sq);
                captured = Some(Piece::Pawn);
            }
            _ => {}
        }

        if mv.is_castling() {
            let kingside = mv.flag == MoveFlag::KingCastle;
            let king_to = Board::castling_king_landing(us, kingside);
            let rook_from = standard_rook_from(us, kingside);
            let rook_to = Board::castling_rook_landing(us, kingside);
            if from != king_to {
                self.toggle(us, piece, from);
                self.toggle(us, piece, king_to);
            }
            if rook_from != rook_to {
                self.toggle(us, Piece::Rook, rook_from);
                self.toggle(us, Piece::Rook, rook_to);
            }
        } else {
            self.toggle(us, piece, from);
            if let Some(promo) = mv.promotion {
                self.toggle(us, promo, to);
            } else {
                self.toggle(us, piece, to);
            }
        }

        if mv.flag == MoveFlag::DoublePawn {
            let ep_rank = (from.rank() as i32 + (to.rank() as i32 - from.rank() as i32) / 2) as u8;
            let ep_sq = Square::from_file_rank(from.file(), ep_rank);
            self.ep = ep_sq.index() as u8;
            self.hash ^= z.en_passant_keys[ep_sq.file() as usize];
        }

        let old_rights = self.rights;
        self.rights &= CASTLING_RIGHTS_UPDATE[from.index()] & CASTLING_RIGHTS_UPDATE[to.index()];
        if old_rights != self.rights {
            self.hash ^= z.castling_keys[old_rights as usize];
            self.hash ^= z.castling_keys[self.rights as usize];
        }

        if piece == Piece::Pawn || captured.is_some() {
            self.halfmove = 0;
        } else {
            self.halfmove = self.halfmove.saturating_add(1);
        }
        self.plies_from_null = self.plies_from_null.saturating_add(1);
        self.stm = them;
        self.hash ^= z.side_to_move_key;
        self.is_in_check(us)
    }

    #[inline]
    pub fn make_null(&mut self) {
        let z = zobrist();
        if self.ep < NONE_EP {
            self.hash ^= z.en_passant_keys[Square::from_index(self.ep as usize).file() as usize];
        }
        self.ep = NONE_EP;
        self.plies_from_null = 0;
        self.stm = self.stm.opponent();
        self.hash ^= z.side_to_move_key;
    }

    #[inline]
    pub fn in_check(&self) -> bool {
        self.is_in_check(self.stm)
    }

    #[inline]
    pub fn is_in_check(&self, color: Color) -> bool {
        self.is_square_attacked(self.king_square(color), color.opponent())
    }

    #[inline]
    pub fn is_square_attacked(&self, sq: Square, by: Color) -> bool {
        let sq_idx = sq.index();
        let occ = self.all_occupancy();
        if pawn_attacks(by.opponent().index(), sq_idx) & self.piece_bb(Piece::Pawn, by) != 0 {
            return true;
        }
        if knight_attacks(sq_idx) & self.piece_bb(Piece::Knight, by) != 0 {
            return true;
        }
        if king_attacks(sq_idx) & self.piece_bb(Piece::King, by) != 0 {
            return true;
        }
        let diag = bishop_attacks(sq_idx, occ);
        if diag & (self.piece_bb(Piece::Bishop, by) | self.piece_bb(Piece::Queen, by)) != 0 {
            return true;
        }
        let lines = rook_attacks(sq_idx, occ);
        lines & (self.piece_bb(Piece::Rook, by) | self.piece_bb(Piece::Queen, by)) != 0
    }

    #[inline]
    pub fn is_legal_move(&self, mv: Move) -> bool {
        let us = self.stm;
        let them = us.opponent();
        let moving_piece = match self.piece_of_color_on(mv.from, us) {
            Some(piece) => piece,
            None => return false,
        };
        if mv.is_castling() {
            return moving_piece == Piece::King;
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

    #[inline]
    fn is_square_attacked_after(
        &self,
        sq: Square,
        by: Color,
        occupancy: u64,
        removed_attackers: u64,
    ) -> bool {
        let sq_idx = sq.index();
        let attackers = !removed_attackers;
        if pawn_attacks(by.opponent().index(), sq_idx) & self.piece_bb(Piece::Pawn, by) & attackers
            != 0
        {
            return true;
        }
        if knight_attacks(sq_idx) & self.piece_bb(Piece::Knight, by) & attackers != 0 {
            return true;
        }
        if king_attacks(sq_idx) & self.piece_bb(Piece::King, by) & attackers != 0 {
            return true;
        }
        if bishop_attacks(sq_idx, occupancy)
            & (self.piece_bb(Piece::Bishop, by) | self.piece_bb(Piece::Queen, by))
            & attackers
            != 0
        {
            return true;
        }
        rook_attacks(sq_idx, occupancy)
            & (self.piece_bb(Piece::Rook, by) | self.piece_bb(Piece::Queen, by))
            & attackers
            != 0
    }

    pub fn generate_captures(&self, color: Color) -> MoveList {
        let mut moves = MoveList::new();
        self.gen_pawn_captures(color, &mut moves);
        self.gen_piece_caps(Piece::Knight, color, &mut moves);
        self.gen_slider_caps(Piece::Bishop, color, &mut moves);
        self.gen_slider_caps(Piece::Rook, color, &mut moves);
        self.gen_slider_caps(Piece::Queen, color, &mut moves);
        self.gen_piece_caps(Piece::King, color, &mut moves);
        moves
    }

    pub fn generate_pseudo_legal_quiets(&self, color: Color) -> MoveList {
        let mut moves = MoveList::new();
        self.gen_pawn_quiets(color, &mut moves);
        self.gen_piece_quiets(Piece::Knight, color, &mut moves);
        self.gen_slider_quiets(Piece::Bishop, color, &mut moves);
        self.gen_slider_quiets(Piece::Rook, color, &mut moves);
        self.gen_slider_quiets(Piece::Queen, color, &mut moves);
        self.gen_piece_quiets(Piece::King, color, &mut moves);
        self.gen_castling(color, &mut moves);
        moves
    }

    pub fn generate_legal_moves(&self) -> MoveList {
        let mut legal = MoveList::new();
        let caps = self.generate_captures(self.stm);
        for i in 0..caps.len() {
            let mv = caps[i];
            if self.is_legal_move(mv) {
                legal.push(mv);
            }
        }
        let quiets = self.generate_pseudo_legal_quiets(self.stm);
        for i in 0..quiets.len() {
            let mv = quiets[i];
            if self.is_legal_move(mv) {
                legal.push(mv);
            }
        }
        legal
    }

    pub fn is_search_draw(&self, root_history: &[u64], path: &[u64], ply: usize) -> bool {
        if self.halfmove >= 100 {
            return true;
        }
        if self.is_insufficient_material() {
            return true;
        }
        let total = root_history.len() + path.len();
        if total < 4 {
            return false;
        }
        let check_len = usize::from(self.halfmove)
            .min(usize::from(self.plies_from_null))
            .min(total);
        let mut matches = 0;
        let mut i = 4;
        while i <= check_len {
            let hist = super::history_hash_at(root_history, path, total - i);
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

    fn is_insufficient_material(&self) -> bool {
        let total = count_bits(self.all_occupancy());
        if total == 2 {
            return true;
        }
        if total == 3 {
            for color in [Color::White, Color::Black] {
                if count_bits(self.piece_bb(Piece::Knight, color)) == 1
                    || count_bits(self.piece_bb(Piece::Bishop, color)) == 1
                {
                    return true;
                }
            }
        }
        false
    }

    fn gen_pawn_captures(&self, color: Color, moves: &mut MoveList) {
        let pawns = self.piece_bb(Piece::Pawn, color);
        let enemies = self.bb[color.opponent().index()];
        let promo_rank = color.promotion_rank();
        for from_idx in iter_bits(pawns) {
            let from = Square::from_index(from_idx);
            let atk = pawn_attacks(color.index(), from_idx);
            for to_idx in iter_bits(atk & enemies) {
                let to = Square::from_index(to_idx);
                if to.rank() == promo_rank {
                    for promo in [Piece::Queen, Piece::Rook, Piece::Bishop, Piece::Knight] {
                        moves.push(Move::promotion_capture(from, to, promo));
                    }
                } else {
                    moves.push(Move::capture(from, to));
                }
            }
            if self.ep < NONE_EP {
                let ep_sq = Square::from_index(self.ep as usize);
                if atk & ep_sq.bitboard() != 0 {
                    moves.push(Move::en_passant(from, ep_sq));
                }
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
            let to_idx = (from_idx as i32 + dir) as usize;
            if to_idx >= 64 {
                continue;
            }
            let to = Square::from_index(to_idx);
            if occ & to.bitboard() != 0 {
                continue;
            }
            if to.rank() == promo_rank {
                continue;
            }
            moves.push(Move::quiet(from, to));
            if from.rank() == start_rank {
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

    fn gen_piece_caps(&self, piece: Piece, color: Color, moves: &mut MoveList) {
        let pieces = self.piece_bb(piece, color);
        let enemies = self.bb[color.opponent().index()];
        for from_idx in iter_bits(pieces) {
            let from = Square::from_index(from_idx);
            let attacks = match piece {
                Piece::Knight => knight_attacks(from_idx),
                Piece::King => king_attacks(from_idx),
                _ => 0,
            } & enemies;
            for to_idx in iter_bits(attacks) {
                moves.push(Move::capture(from, Square::from_index(to_idx)));
            }
        }
    }

    fn gen_piece_quiets(&self, piece: Piece, color: Color, moves: &mut MoveList) {
        let pieces = self.piece_bb(piece, color);
        let empty = !self.all_occupancy();
        for from_idx in iter_bits(pieces) {
            let from = Square::from_index(from_idx);
            let attacks = match piece {
                Piece::Knight => knight_attacks(from_idx),
                Piece::King => king_attacks(from_idx),
                _ => 0,
            } & empty;
            for to_idx in iter_bits(attacks) {
                moves.push(Move::quiet(from, Square::from_index(to_idx)));
            }
        }
    }

    fn gen_slider_caps(&self, piece: Piece, color: Color, moves: &mut MoveList) {
        let pieces = self.piece_bb(piece, color);
        let enemies = self.bb[color.opponent().index()];
        let occ = self.all_occupancy();
        for from_idx in iter_bits(pieces) {
            let from = Square::from_index(from_idx);
            let attacks = slider_attacks(piece, from_idx, occ) & enemies;
            for to_idx in iter_bits(attacks) {
                moves.push(Move::capture(from, Square::from_index(to_idx)));
            }
        }
    }

    fn gen_slider_quiets(&self, piece: Piece, color: Color, moves: &mut MoveList) {
        let pieces = self.piece_bb(piece, color);
        let empty = !self.all_occupancy();
        let occ = self.all_occupancy();
        for from_idx in iter_bits(pieces) {
            let from = Square::from_index(from_idx);
            let attacks = slider_attacks(piece, from_idx, occ) & empty;
            for to_idx in iter_bits(attacks) {
                moves.push(Move::quiet(from, Square::from_index(to_idx)));
            }
        }
    }

    fn gen_castling(&self, color: Color, moves: &mut MoveList) {
        if self.rights == 0 {
            return;
        }
        let occ = self.all_occupancy();
        let enemy = color.opponent();
        match color {
            Color::White => {
                if self.rights & WHITE_KING_CASTLE != 0 {
                    let between = Square::F1.bitboard() | Square::G1.bitboard();
                    if occ & between == 0
                        && !self.is_square_attacked(Square::E1, enemy)
                        && !self.is_square_attacked(Square::F1, enemy)
                        && !self.is_square_attacked(Square::G1, enemy)
                    {
                        moves.push(Move::king_castle(Square::E1, Square::G1));
                    }
                }
                if self.rights & WHITE_QUEEN_CASTLE != 0 {
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
                if self.rights & BLACK_KING_CASTLE != 0 {
                    let between = Square::F8.bitboard() | Square::G8.bitboard();
                    if occ & between == 0
                        && !self.is_square_attacked(Square::E8, enemy)
                        && !self.is_square_attacked(Square::F8, enemy)
                        && !self.is_square_attacked(Square::G8, enemy)
                    {
                        moves.push(Move::king_castle(Square::E8, Square::G8));
                    }
                }
                if self.rights & BLACK_QUEEN_CASTLE != 0 {
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
}

#[inline]
fn slider_attacks(piece: Piece, sq: usize, occ: u64) -> u64 {
    match piece {
        Piece::Bishop => bishop_attacks(sq, occ),
        Piece::Rook => rook_attacks(sq, occ),
        Piece::Queen => queen_attacks(sq, occ),
        _ => 0,
    }
}

#[inline]
fn standard_rook_from(color: Color, kingside: bool) -> Square {
    match (color, kingside) {
        (Color::White, true) => Square::H1,
        (Color::White, false) => Square::A1,
        (Color::Black, true) => Square::H8,
        (Color::Black, false) => Square::A8,
    }
}

const _: () = assert!(std::mem::size_of::<AkimboPos>() <= 160);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Board;

    fn sorted_uci(moves: &MoveList) -> Vec<String> {
        let mut uci: Vec<String> = (0..moves.len()).map(|i| moves[i].to_uci()).collect();
        uci.sort();
        uci
    }

    #[test]
    fn akimbo_pos_is_at_most_160_bytes() {
        assert!(
            std::mem::size_of::<AkimboPos>() <= 160,
            "{}",
            std::mem::size_of::<AkimboPos>()
        );
    }

    #[test]
    fn piece_on_matches_board_mailbox_after_make() {
        crate::init();
        let mut board = Board::new();
        let pos = AkimboPos::from_board(&board);
        for sq in Square::ALL {
            assert_eq!(pos.piece_on(sq), board.piece_on(sq), "{}", sq);
        }
        let legal = board.generate_legal_moves();
        let mv = legal
            .iter()
            .copied()
            .find(|mv| mv.to_uci() == "e2e4")
            .expect("e2e4");
        board.make_move(mv);
        let mut child = pos;
        assert!(!child.make(mv));
        for sq in Square::ALL {
            assert_eq!(child.piece_on(sq), board.piece_on(sq), "{}", sq);
        }
    }

    #[test]
    fn from_board_matches_startpos_hash_and_check() {
        crate::init();
        let board = Board::new();
        let pos = AkimboPos::from_board(&board);
        assert_eq!(pos.hash(), board.hash);
        assert_eq!(pos.tt_hash(), board.tt_hash());
        assert_eq!(pos.in_check(), board.in_check());
        assert_eq!(pos.side_to_move(), board.side_to_move);
        assert_eq!(
            pos.king_square(Color::White),
            board.king_square(Color::White)
        );
    }

    #[test]
    fn make_matches_board_hash_and_check_on_startpos_and_kiwipete() {
        crate::init();
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
        ] {
            let mut board = Board::from_fen(fen).expect("fen");
            let pos = AkimboPos::from_board(&board);
            let legal = board.generate_legal_moves();
            for i in 0..legal.len() {
                let mv = legal[i];
                let mut child_board = board.clone();
                child_board.make_move(mv);
                let mut child = pos;
                let illegal = child.make(mv);
                assert!(!illegal, "{} {}", fen, mv.to_uci());
                assert_eq!(child.hash(), child_board.hash, "{} {}", fen, mv.to_uci());
                assert_eq!(
                    child.tt_hash(),
                    child_board.tt_hash(),
                    "{} {}",
                    fen,
                    mv.to_uci()
                );
                assert_eq!(
                    child.in_check(),
                    child_board.in_check(),
                    "{} {}",
                    fen,
                    mv.to_uci()
                );
            }
        }
    }

    #[test]
    fn generated_moves_match_board_on_startpos_and_ep() {
        crate::init();
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPP1PPP/RNBQKBNR w KQkq f6 0 3",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        ] {
            let mut board = Board::from_fen(fen).expect("fen");
            let pos = AkimboPos::from_board(&board);
            assert_eq!(
                sorted_uci(&pos.generate_captures(board.side_to_move)),
                sorted_uci(&board.generate_captures(board.side_to_move)),
                "captures {fen}"
            );
            assert_eq!(
                sorted_uci(&pos.generate_pseudo_legal_quiets(board.side_to_move)),
                sorted_uci(&board.generate_pseudo_legal_quiets(board.side_to_move)),
                "quiets {fen}"
            );
            assert_eq!(
                sorted_uci(&pos.generate_legal_moves()),
                sorted_uci(&board.generate_legal_moves()),
                "legal {fen}"
            );
        }
    }
}
