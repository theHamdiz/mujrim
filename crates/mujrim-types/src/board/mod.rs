pub mod attack_tables;
mod move_gen;
pub mod zobrist;

use self::zobrist::zobrist;
use crate::bitboard::*;
use crate::chess_move::{Move, MoveFlag};
use crate::piece::{Color, Piece};
use crate::square::Square;
use std::fmt;

#[inline(always)]
const fn evaluation_material_value(piece: Piece) -> i32 {
    match piece {
        Piece::Pawn => 109,
        Piece::Knight => 403,
        Piece::Bishop => 435,
        Piece::Rook => 679,
        Piece::Queen => 1242,
        Piece::King => 0,
    }
}

// ── Castling rights bit flags ───────────────────────────────────────────────
pub const WHITE_KING_CASTLE: u8 = 0b0001;
pub const WHITE_QUEEN_CASTLE: u8 = 0b0010;
pub const BLACK_KING_CASTLE: u8 = 0b0100;
pub const BLACK_QUEEN_CASTLE: u8 = 0b1000;
pub const ALL_CASTLING: u8 = 0b1111;
const EMPTY_PIECE_ID: u8 = u8::MAX;

/// Information needed to undo a move — stored on a stack.
#[derive(Copy, Clone, Debug)]
pub struct UndoInfo {
    pub captured_piece: Option<Piece>,
    pub castling_rights: u8,
    pub en_passant: Option<Square>,
    pub halfmove_clock: u32,
    pub plies_from_null: usize,
    pub hash: u64,
}

/// The chess board state.
#[derive(Clone)]
pub struct Board {
    /// pieces[color][piece_type] — one bitboard per color per piece.
    pub pieces: [[Bitboard; 6]; 2],
    /// Occupancy per color.
    pub occupancy: [Bitboard; 2],
    /// Piece/color id by square (`piece * 2 + color`), or `u8::MAX` when empty.
    piece_at: [u8; 64],
    /// Side to move.
    pub side_to_move: Color,
    /// Castling rights encoded as 4 bits.
    pub castling_rights: u8,
    /// When set, castling is encoded as king-takes-rook (UCI Chess960).
    chess960: bool,
    /// King start files used to revoke Chess960 castling rights `[white, black]`.
    castling_king_file: [u8; 2],
    /// Rook start files `[white KS, white QS, black KS, black QS]`.
    castling_rook_file: [u8; 4],
    /// En passant target square (the square behind the double-pushed pawn).
    pub en_passant: Option<Square>,
    /// Halfmove clock (for 50-move rule).
    pub halfmove_clock: u32,
    /// Fullmove number.
    pub fullmove_number: u32,
    /// Zobrist hash of the current position.
    pub hash: u64,
    /// Incrementally maintained non-king material used for evaluation scaling.
    total_material: i32,
    /// Number of real moves since the last null move.
    plies_from_null: usize,
    /// Undo stack for unmake_move.
    history: Vec<UndoInfo>,
    /// Hash history for repetition detection.
    /// Each entry is the hash BEFORE the move was made.
    pub hash_history: Vec<u64>,
}

/// Receives piece mutations while a move is applied.
///
/// The generic observer keeps network-specific accumulator work out of the
/// board representation and is monomorphized away for the no-op path.
pub trait BoardObserver {
    fn on_piece_change(
        &mut self,
        board: &Board,
        piece: Piece,
        color: Color,
        square: Square,
        add: bool,
    );

    fn on_piece_move(
        &mut self,
        board: &Board,
        piece: Piece,
        color: Color,
        from: Square,
        to: Square,
    );

    fn on_piece_mutate(
        &mut self,
        board: &Board,
        old_piece: Piece,
        old_color: Color,
        new_piece: Piece,
        new_color: Color,
        square: Square,
    );
}

pub struct NullBoardObserver;

impl BoardObserver for NullBoardObserver {
    #[inline(always)]
    fn on_piece_change(&mut self, _: &Board, _: Piece, _: Color, _: Square, _: bool) {}

    #[inline(always)]
    fn on_piece_move(&mut self, _: &Board, _: Piece, _: Color, _: Square, _: Square) {}

    #[inline(always)]
    fn on_piece_mutate(&mut self, _: &Board, _: Piece, _: Color, _: Piece, _: Color, _: Square) {}
}

impl Default for Board {
    fn default() -> Self {
        Board::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")
            .expect("starting position FEN is valid")
    }
}

impl Board {
    /// Creates a new board in the standard starting position.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty board (no pieces).
    pub fn empty() -> Self {
        Self {
            pieces: [[0; 6]; 2],
            occupancy: [0; 2],
            piece_at: [EMPTY_PIECE_ID; 64],
            side_to_move: Color::White,
            castling_rights: 0,
            chess960: false,
            castling_king_file: [4, 4],
            castling_rook_file: [7, 0, 7, 0],
            en_passant: None,
            halfmove_clock: 0,
            fullmove_number: 1,
            hash: 0,
            total_material: 0,
            plies_from_null: 0,
            history: Vec::with_capacity(256),
            hash_history: Vec::with_capacity(256),
        }
    }

    // ── Piece access ────────────────────────────────────────────────────────

    /// Returns the bitboard for a given piece type and color.
    #[inline(always)]
    pub fn piece_bb(&self, piece: Piece, color: Color) -> Bitboard {
        self.pieces[color.index()][piece.index()]
    }

    /// Returns the total occupancy (all pieces of both colors).
    #[inline(always)]
    pub fn all_occupancy(&self) -> Bitboard {
        self.occupancy[0] | self.occupancy[1]
    }

    /// Returns the occupancy for a given color.
    #[inline(always)]
    pub fn color_occupancy(&self, color: Color) -> Bitboard {
        self.occupancy[color.index()]
    }

    /// Recomputes occupancy bitboards from piece bitboards.
    pub fn update_occupancy(&mut self) {
        self.piece_at.fill(EMPTY_PIECE_ID);
        for color in 0..2 {
            self.occupancy[color] = self.pieces[color].iter().fold(0, |acc, &bb| acc | bb);
            for piece in 0..Piece::COUNT {
                let mut occupied = self.pieces[color][piece];
                while occupied != 0 {
                    let square = occupied.trailing_zeros() as usize;
                    occupied &= occupied - 1;
                    self.piece_at[square] = (piece * 2 + color) as u8;
                }
            }
        }
    }

    /// Places a piece on the board (used during setup / FEN parsing).
    pub fn put_piece(&mut self, piece: Piece, color: Color, square: Square) {
        set_bit(
            &mut self.pieces[color.index()][piece.index()],
            square.index(),
        );
        set_bit(&mut self.occupancy[color.index()], square.index());
        self.piece_at[square.index()] = (piece.index() * 2 + color.index()) as u8;
        self.total_material += evaluation_material_value(piece);
        // Update hash
        self.hash ^= zobrist().piece_keys[color.index()][piece.index()][square.index()];
    }

    /// Removes a piece from the board.
    pub fn remove_piece(&mut self, piece: Piece, color: Color, square: Square) {
        clear_bit(
            &mut self.pieces[color.index()][piece.index()],
            square.index(),
        );
        clear_bit(&mut self.occupancy[color.index()], square.index());
        self.piece_at[square.index()] = EMPTY_PIECE_ID;
        self.total_material -= evaluation_material_value(piece);
        self.hash ^= zobrist().piece_keys[color.index()][piece.index()][square.index()];
    }

    #[inline(always)]
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

    /// Total non-king material in the evaluation network's native units.
    #[inline(always)]
    pub const fn total_material(&self) -> i32 {
        self.total_material
    }

    /// Returns what piece (if any) is on a given square.
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

    /// Returns the piece of `color` on `square` when the color is already known.
    #[inline(always)]
    pub fn piece_of_color_on(&self, square: Square, color: Color) -> Option<Piece> {
        let id = self.piece_at[square.index()];
        (id & 1 == color.index() as u8)
            .then(|| Piece::from_index(usize::from(id) / 2))
            .flatten()
    }

    /// Compact piece/color ids used by incremental evaluators.
    #[inline(always)]
    pub const fn piece_ids(&self) -> &[u8; 64] {
        &self.piece_at
    }

    /// Returns the square of the king for the given color.
    #[inline(always)]
    pub fn king_square(&self, color: Color) -> Square {
        let king_bb = self.piece_bb(Piece::King, color);
        debug_assert!(king_bb != 0, "king must exist on the board");
        Square::from_index(get_lsb(king_bb))
    }

    #[inline(always)]
    pub const fn is_chess960(&self) -> bool {
        self.chess960
    }

    #[inline(always)]
    pub fn set_chess960(&mut self, enabled: bool) {
        self.chess960 = enabled;
        if enabled {
            self.refresh_castling_origins();
        }
    }

    #[inline(always)]
    pub fn castling_king_landing(color: Color, kingside: bool) -> Square {
        match (color, kingside) {
            (Color::White, true) => Square::G1,
            (Color::White, false) => Square::C1,
            (Color::Black, true) => Square::G8,
            (Color::Black, false) => Square::C8,
        }
    }

    #[inline(always)]
    pub fn castling_rook_landing(color: Color, kingside: bool) -> Square {
        match (color, kingside) {
            (Color::White, true) => Square::F1,
            (Color::White, false) => Square::D1,
            (Color::Black, true) => Square::F8,
            (Color::Black, false) => Square::D8,
        }
    }

    #[inline(always)]
    pub fn castling_rook_from(&self, color: Color, kingside: bool) -> Square {
        let file = self.castling_rook_file[Self::rook_file_index(color, kingside)];
        let rank = if color == Color::White { 0 } else { 7 };
        Square::from_file_rank(file, rank)
    }

    #[inline(always)]
    pub fn castle_uci_to(&self, color: Color, kingside: bool) -> Square {
        if self.chess960 {
            self.castling_rook_from(color, kingside)
        } else {
            Self::castling_king_landing(color, kingside)
        }
    }

    #[inline(always)]
    const fn rook_file_index(color: Color, kingside: bool) -> usize {
        match (color, kingside) {
            (Color::White, true) => 0,
            (Color::White, false) => 1,
            (Color::Black, true) => 2,
            (Color::Black, false) => 3,
        }
    }

    fn refresh_castling_origins(&mut self) {
        for color in [Color::White, Color::Black] {
            if self.piece_bb(Piece::King, color) != 0 {
                self.castling_king_file[color.index()] = self.king_square(color).file();
            }
        }
        if !self.chess960 {
            self.castling_rook_file = [7, 0, 7, 0];
            return;
        }
        for color in [Color::White, Color::Black] {
            let king_file = self.castling_king_file[color.index()];
            let rank = if color == Color::White { 0 } else { 7 };
            let mut rooks = self.piece_bb(Piece::Rook, color);
            let mut queenside = None;
            let mut kingside = None;
            while rooks != 0 {
                let sq = Square::from_index(rooks.trailing_zeros() as usize);
                rooks &= rooks - 1;
                if sq.rank() != rank {
                    continue;
                }
                if sq.file() > king_file {
                    kingside = Some(kingside.map_or(sq.file(), |file: u8| file.max(sq.file())));
                } else if sq.file() < king_file {
                    queenside = Some(queenside.map_or(sq.file(), |file: u8| file.min(sq.file())));
                }
            }
            if let Some(file) = kingside {
                self.castling_rook_file[Self::rook_file_index(color, true)] = file;
            }
            if let Some(file) = queenside {
                self.castling_rook_file[Self::rook_file_index(color, false)] = file;
            }
        }
    }

    /// Returns the count of a specific piece type for a given color.
    #[inline(always)]
    pub fn piece_count(&self, piece: Piece, color: Color) -> u32 {
        count_bits(self.piece_bb(piece, color))
    }

    /// Returns a list of squares occupied by a given piece/color.
    pub fn piece_squares(&self, piece: Piece, color: Color) -> Vec<Square> {
        iter_bits(self.piece_bb(piece, color))
            .map(Square::from_index)
            .collect()
    }

    /// Returns the pawn bitboard for a color.
    #[inline(always)]
    pub fn pawns(&self, color: Color) -> Bitboard {
        self.piece_bb(Piece::Pawn, color)
    }

    /// Transposition-table key with coarse rule-50 history separation.
    /// The raw [`Self::hash`] remains unchanged for repetition detection.
    #[inline(always)]
    pub fn tt_hash(&self) -> u64 {
        let bucket = (self.halfmove_clock.saturating_sub(8) as usize / 8).min(15);
        self.hash ^ zobrist().fiftymove_keys[bucket]
    }

    /// Computes the transposition-table key after `mv` without mutating the board.
    #[inline(always)]
    pub fn tt_hash_after(&self, mv: Move) -> u64 {
        let z = zobrist();
        let us = self.side_to_move;
        let them = us.opponent();
        let from = mv.from;
        let to = mv.to;
        let piece = self
            .piece_of_color_on(from, us)
            .expect("tt_hash_after: no piece on source square");
        let mut hash = self.hash;

        if let Some(ep) = self.en_passant {
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
            let king_to = Self::castling_king_landing(us, kingside);
            if king_to != to {
                hash ^= z.piece_keys[us.index()][piece.index()][to.index()];
                hash ^= z.piece_keys[us.index()][piece.index()][king_to.index()];
            }
            let rook_from = self.castling_rook_from(us, kingside);
            let rook_to = Self::castling_rook_landing(us, kingside);
            hash ^= z.piece_keys[us.index()][Piece::Rook.index()][rook_from.index()];
            hash ^= z.piece_keys[us.index()][Piece::Rook.index()][rook_to.index()];
        }

        if mv.flag == MoveFlag::DoublePawn {
            let ep_rank = (from.rank() as i32 + (to.rank() as i32 - from.rank() as i32) / 2) as u8;
            let ep_square = Square::from_file_rank(from.file(), ep_rank);
            hash ^= z.en_passant_keys[ep_square.file() as usize];
        }

        let old_castling = self.castling_rights;
        let new_castling = self.castling_rights_after(from, to);
        if old_castling != new_castling {
            hash ^= z.castling_keys[old_castling as usize];
            hash ^= z.castling_keys[new_castling as usize];
        }

        let halfmove_clock = if piece == Piece::Pawn || captured.is_some() {
            0
        } else {
            self.halfmove_clock + 1
        };
        let bucket = (halfmove_clock.saturating_sub(8) as usize / 8).min(15);
        hash ^ z.side_to_move_key ^ z.fiftymove_keys[bucket]
    }

    // ── Make / Unmake move ───────────────────────────────────────────────────

    /// Applies a move without observing its piece mutations.
    #[inline]
    pub fn make_move(&mut self, mv: Move) {
        self.make_move_observed(mv, &mut NullBoardObserver);
    }

    /// Applies a move and reports its piece mutations to `observer`.
    pub fn make_move_observed<O: BoardObserver>(&mut self, mv: Move, observer: &mut O) {
        // Track hash for repetition detection
        self.hash_history.push(self.hash);
        let z = zobrist();
        let us = self.side_to_move;
        let them = us.opponent();
        let from = mv.from;
        let to = mv.to;

        // Save undo info
        let undo = UndoInfo {
            captured_piece: None, // will be set below if capture
            castling_rights: self.castling_rights,
            en_passant: self.en_passant,
            halfmove_clock: self.halfmove_clock,
            plies_from_null: self.plies_from_null,
            hash: self.hash,
        };
        self.history.push(undo);

        // Determine what piece is moving
        let piece = self
            .piece_of_color_on(from, us)
            .expect("make_move: no piece on source square");

        // Clear en passant from hash (will set new one if applicable)
        if let Some(ep) = self.en_passant {
            self.hash ^= z.en_passant_keys[ep.file() as usize];
        }
        self.en_passant = None;

        // Handle captures. Ordinary captures follow remove/mutate ordering so
        // incremental threat evaluators can update only the affected rays.
        let mut captured = None;
        match mv.flag {
            MoveFlag::Capture | MoveFlag::PromotionCapture => {
                if let Some(cap_piece) = self.piece_of_color_on(to, them) {
                    self.remove_piece(piece, us, from);
                    observer.on_piece_change(self, piece, us, from, false);
                    self.remove_piece(cap_piece, them, to);
                    self.put_piece(piece, us, to);
                    observer.on_piece_mutate(self, cap_piece, them, piece, us, to);
                    captured = Some(cap_piece);
                }
            }
            _ => {}
        }

        // Update captured piece in undo info
        if let Some(last) = self.history.last_mut() {
            last.captured_piece = captured;
        }

        // Non-captures relocate normally. Castling moves the king to c/g even
        // when UCI Chess960 encodes the move as king-takes-rook.
        if captured.is_none() && !mv.is_castling() {
            self.relocate_piece(piece, us, from, to);
            observer.on_piece_move(self, piece, us, from, to);
        }

        if mv.flag == MoveFlag::EnPassant {
            let cap_sq = Square::from_file_rank(to.file(), from.rank());
            self.remove_piece(Piece::Pawn, them, cap_sq);
            observer.on_piece_change(self, Piece::Pawn, them, cap_sq, false);
            captured = Some(Piece::Pawn);
            if let Some(last) = self.history.last_mut() {
                last.captured_piece = captured;
            }
        }

        if let Some(promotion) = mv.promotion {
            self.remove_piece(piece, us, to);
            self.put_piece(promotion, us, to);
            observer.on_piece_mutate(self, piece, us, promotion, us, to);
        }

        if mv.is_castling() {
            let kingside = mv.flag == MoveFlag::KingCastle;
            let king_to = Self::castling_king_landing(us, kingside);
            let rook_from = self.castling_rook_from(us, kingside);
            let rook_to = Self::castling_rook_landing(us, kingside);
            if from != king_to {
                self.relocate_piece(piece, us, from, king_to);
                observer.on_piece_move(self, piece, us, from, king_to);
            }
            if rook_from != rook_to {
                self.relocate_piece(Piece::Rook, us, rook_from, rook_to);
                observer.on_piece_move(self, Piece::Rook, us, rook_from, rook_to);
            }
        }

        // Set en passant square for double pawn push (the square between from and to)
        if mv.flag == MoveFlag::DoublePawn {
            let ep_rank = (from.rank() as i32 + (to.rank() as i32 - from.rank() as i32) / 2) as u8;
            let ep_sq = Square::from_file_rank(from.file(), ep_rank);
            self.en_passant = Some(ep_sq);
            self.hash ^= z.en_passant_keys[ep_sq.file() as usize];
        }

        // Update castling rights
        let old_castling = self.castling_rights;
        self.castling_rights = self.castling_rights_after(from, to);
        if old_castling != self.castling_rights {
            self.hash ^= z.castling_keys[old_castling as usize];
            self.hash ^= z.castling_keys[self.castling_rights as usize];
        }

        // Update halfmove clock
        if piece == Piece::Pawn || captured.is_some() {
            self.halfmove_clock = 0;
        } else {
            self.halfmove_clock += 1;
        }
        self.plies_from_null += 1;

        // Update fullmove number
        if us == Color::Black {
            self.fullmove_number += 1;
        }

        // Switch side
        self.side_to_move = them;
        self.hash ^= z.side_to_move_key;
    }

    /// Undoes the last move, restoring the board to its previous state.
    pub fn unmake_move(&mut self, mv: Move) {
        self.hash_history.pop();
        let them = self.side_to_move; // The side that just moved is now "them"
        let us = them.opponent();

        self.side_to_move = us;

        let undo = self
            .history
            .pop()
            .expect("unmake_move: empty history stack");

        let from = mv.from;
        let to = mv.to;

        if mv.is_castling() {
            let kingside = mv.flag == MoveFlag::KingCastle;
            let king_to = Self::castling_king_landing(us, kingside);
            let rook_from = self.castling_rook_from(us, kingside);
            let rook_to = Self::castling_rook_landing(us, kingside);
            if rook_from != rook_to {
                self.relocate_piece(Piece::Rook, us, rook_to, rook_from);
            }
            if from != king_to {
                self.relocate_piece(Piece::King, us, king_to, from);
            }
        } else {
            // Determine the piece that was placed (after promotion, it might differ)
            let placed_piece = mv.promotion.unwrap_or_else(|| {
                self.piece_of_color_on(to, us)
                    .expect("unmake_move: no piece on target square")
            });

            // Restore the moving piece. Promotions reverse their material change;
            // ordinary moves only relocate the existing piece.
            if mv.is_promotion() {
                self.remove_piece(placed_piece, us, to);
                self.put_piece(Piece::Pawn, us, from);
            } else {
                self.relocate_piece(placed_piece, us, to, from);
            }

            // Restore captured piece
            match mv.flag {
                MoveFlag::Capture | MoveFlag::PromotionCapture => {
                    if let Some(cap_piece) = undo.captured_piece {
                        self.put_piece(cap_piece, them, to);
                    }
                }
                MoveFlag::EnPassant => {
                    let cap_sq = Square::from_file_rank(to.file(), from.rank());
                    self.put_piece(Piece::Pawn, them, cap_sq);
                }
                _ => {}
            }
        }

        // Restore state
        self.castling_rights = undo.castling_rights;
        self.en_passant = undo.en_passant;
        self.halfmove_clock = undo.halfmove_clock;
        self.plies_from_null = undo.plies_from_null;
        self.hash = undo.hash;

        if us == Color::Black {
            self.fullmove_number -= 1;
        }
    }

    /// Performs a null move (skips the current side's turn).
    pub fn make_null_move(&mut self) {
        self.hash_history.push(self.hash);
        let z = zobrist();
        let undo = UndoInfo {
            captured_piece: None,
            castling_rights: self.castling_rights,
            en_passant: self.en_passant,
            halfmove_clock: self.halfmove_clock,
            plies_from_null: self.plies_from_null,
            hash: self.hash,
        };
        self.history.push(undo);

        if let Some(ep) = self.en_passant {
            self.hash ^= z.en_passant_keys[ep.file() as usize];
        }
        self.en_passant = None;
        self.plies_from_null = 0;
        self.side_to_move = self.side_to_move.opponent();
        self.hash ^= z.side_to_move_key;
    }

    /// Undoes a null move.
    pub fn unmake_null_move(&mut self) {
        self.hash_history.pop();
        let undo = self.history.pop().expect("unmake_null_move: empty history");
        self.side_to_move = self.side_to_move.opponent();
        self.en_passant = undo.en_passant;
        self.plies_from_null = undo.plies_from_null;
        self.hash = undo.hash;
    }

    // ── FEN ─────────────────────────────────────────────────────────────────

    /// Parses a FEN string into a Board.
    pub fn from_fen(fen: &str) -> Result<Self, String> {
        let parts: Vec<&str> = fen.split_whitespace().collect();
        if parts.len() != 6 {
            return Err(format!("FEN must have 6 fields, got {}", parts.len()));
        }

        let mut board = Self::empty();

        // 1. Piece placement (rank 8 first in FEN, which is our rank index 7)
        let rows: Vec<&str> = parts[0].split('/').collect();
        if rows.len() != 8 {
            return Err("FEN piece placement must have 8 ranks".into());
        }
        for (row_idx, row) in rows.iter().enumerate() {
            let rank = 7 - row_idx; // FEN starts from rank 8 (top)
            let mut file = 0usize;
            for ch in row.chars() {
                if let Some(skip) = ch.to_digit(10) {
                    file += skip as usize;
                } else {
                    let color = if ch.is_uppercase() {
                        Color::White
                    } else {
                        Color::Black
                    };
                    let piece = Piece::from_char(ch)
                        .ok_or_else(|| format!("invalid piece char in FEN: '{ch}'"))?;
                    if file >= 8 {
                        return Err(format!("FEN file overflow at rank {rank}"));
                    }
                    board.put_piece(piece, color, Square::from_file_rank(file as u8, rank as u8));
                    file += 1;
                }
            }
        }

        // 2. Active color
        board.side_to_move = match parts[1] {
            "w" => Color::White,
            "b" => {
                board.hash ^= zobrist().side_to_move_key;
                Color::Black
            }
            _ => return Err(format!("invalid active color: '{}'", parts[1])),
        };

        // 3. Castling availability (KQkq or Shredder-FEN file letters)
        if board.piece_bb(Piece::King, Color::White) != 0 {
            board.castling_king_file[0] = board.king_square(Color::White).file();
        }
        if board.piece_bb(Piece::King, Color::Black) != 0 {
            board.castling_king_file[1] = board.king_square(Color::Black).file();
        }
        board.castling_rights = 0;
        for ch in parts[2].chars() {
            match ch {
                'K' => {
                    board.castling_rights |= WHITE_KING_CASTLE;
                    board.castling_rook_file[0] = 7;
                }
                'Q' => {
                    board.castling_rights |= WHITE_QUEEN_CASTLE;
                    board.castling_rook_file[1] = 0;
                }
                'k' => {
                    board.castling_rights |= BLACK_KING_CASTLE;
                    board.castling_rook_file[2] = 7;
                }
                'q' => {
                    board.castling_rights |= BLACK_QUEEN_CASTLE;
                    board.castling_rook_file[3] = 0;
                }
                'A'..='H' => {
                    board.chess960 = true;
                    let file = ch as u8 - b'A';
                    if file > board.castling_king_file[0] {
                        board.castling_rights |= WHITE_KING_CASTLE;
                        board.castling_rook_file[0] = file;
                    } else {
                        board.castling_rights |= WHITE_QUEEN_CASTLE;
                        board.castling_rook_file[1] = file;
                    }
                }
                'a'..='h' => {
                    board.chess960 = true;
                    let file = ch as u8 - b'a';
                    if file > board.castling_king_file[1] {
                        board.castling_rights |= BLACK_KING_CASTLE;
                        board.castling_rook_file[2] = file;
                    } else {
                        board.castling_rights |= BLACK_QUEEN_CASTLE;
                        board.castling_rook_file[3] = file;
                    }
                }
                '-' => {}
                _ => return Err(format!("invalid castling char: '{ch}'")),
            }
        }
        board.hash ^= zobrist().castling_keys[board.castling_rights as usize];

        // 4. En passant target square
        if parts[3] != "-" {
            let ep_sq: Square = parts[3]
                .parse()
                .map_err(|e: String| format!("invalid en passant square: {e}"))?;
            board.en_passant = Some(ep_sq);
            board.hash ^= zobrist().en_passant_keys[ep_sq.file() as usize];
        }

        // 5. Halfmove clock
        board.halfmove_clock = parts[4]
            .parse()
            .map_err(|_| format!("invalid halfmove clock: '{}'", parts[4]))?;

        // 6. Fullmove number
        board.fullmove_number = parts[5]
            .parse()
            .map_err(|_| format!("invalid fullmove number: '{}'", parts[5]))?;

        Ok(board)
    }

    /// Serializes the board to a FEN string.
    pub fn to_fen(&self) -> String {
        let mut fen = String::with_capacity(80);

        // 1. Piece placement
        for rank in (0..8).rev() {
            let mut empty_count = 0u32;
            for file in 0..8 {
                let sq = Square::from_file_rank(file, rank);
                if let Some((piece, color)) = self.piece_on(sq) {
                    if empty_count > 0 {
                        fen.push(char::from_digit(empty_count, 10).unwrap());
                        empty_count = 0;
                    }
                    let ch = piece.to_char();
                    if color == Color::Black {
                        fen.push(ch.to_ascii_lowercase());
                    } else {
                        fen.push(ch);
                    }
                } else {
                    empty_count += 1;
                }
            }
            if empty_count > 0 {
                fen.push(char::from_digit(empty_count, 10).unwrap());
            }
            if rank > 0 {
                fen.push('/');
            }
        }

        // 2. Active color
        fen.push(' ');
        fen.push(if self.side_to_move == Color::White {
            'w'
        } else {
            'b'
        });

        // 3. Castling
        fen.push(' ');
        self.write_castling_fen(&mut fen);

        // 4. En passant
        fen.push(' ');
        match self.en_passant {
            Some(sq) => fen.push_str(&sq.to_string()),
            None => fen.push('-'),
        }

        // 5 & 6. Halfmove clock and fullmove number
        fen.push_str(&format!(
            " {} {}",
            self.halfmove_clock, self.fullmove_number
        ));

        fen
    }

    // ── Game state queries ──────────────────────────────────────────────────

    /// Returns true if the given side's king is in check.
    pub fn is_in_check(&self, color: Color) -> bool {
        let king_sq = self.king_square(color);
        self.is_square_attacked(king_sq, color.opponent())
    }

    /// Returns true if the current side to move is in check.
    pub fn in_check(&self) -> bool {
        self.is_in_check(self.side_to_move)
    }

    /// Returns true if the position is checkmate (current side has no legal moves and is in check).
    pub fn is_checkmate(&mut self) -> bool {
        self.in_check() && self.generate_legal_moves().is_empty()
    }

    /// Returns true if the position is stalemate (no legal moves, not in check).
    pub fn is_stalemate(&mut self) -> bool {
        !self.in_check() && self.generate_legal_moves().is_empty()
    }

    /// Returns true if the game is over (checkmate, stalemate, or draw).
    pub fn is_game_over(&mut self) -> bool {
        self.is_checkmate() || self.is_stalemate() || self.is_draw()
    }

    #[inline]
    fn draw_by_fifty_move_rule(&self) -> bool {
        if self.halfmove_clock >= 100 {
            if !self.in_check() {
                return true;
            }
            let mut board = self.clone();
            if !board.generate_legal_moves().is_empty() {
                return true;
            }
        }
        false
    }

    /// Draw detection for an actual game position.
    pub fn is_draw(&self) -> bool {
        self.draw_by_fifty_move_rule()
            || self.has_threefold_repetition()
            || self.is_insufficient_material()
    }

    /// Draw detection inside search.
    ///
    /// A repetition is sufficient when its prior occurrence is strictly below
    /// the root. Positions from the played game still require three occurrences.
    pub fn is_search_draw(&self, ply: usize) -> bool {
        self.draw_by_fifty_move_rule()
            || self.is_repetition_draw(ply)
            || self.is_insufficient_material()
    }

    /// Returns true if the current position has occurred before in the game.
    /// Checks only reversible positions (since last pawn move or capture).
    pub fn has_repetition(&self) -> bool {
        self.repetition_matches(1, None)
    }

    /// Returns true once the current position has occurred at least three times.
    pub fn has_threefold_repetition(&self) -> bool {
        self.repetition_matches(2, None)
    }

    fn is_repetition_draw(&self, ply: usize) -> bool {
        self.repetition_matches(2, Some(ply))
    }

    #[inline]
    fn repetition_matches(&self, required: usize, search_ply: Option<usize>) -> bool {
        let len = self.hash_history.len();
        // A legal chess position, including side to move, cannot repeat in
        // fewer than four plies.
        if len < 4 {
            return false;
        }

        // Never cross an irreversible move or a null-move boundary.
        let check_len = (self.halfmove_clock as usize)
            .min(self.plies_from_null)
            .min(len);

        let mut matches = 0;
        let mut i = 4;
        while i <= check_len {
            if self.hash_history[len - i] == self.hash {
                if search_ply.is_some_and(|ply| i < ply) {
                    return true;
                }
                matches += 1;
                if matches >= required {
                    return true;
                }
            }
            i += 2;
        }
        false
    }

    fn is_insufficient_material(&self) -> bool {
        let all_pieces = self.all_occupancy();
        let total = count_bits(all_pieces);
        if total == 2 {
            return true; // K vs K
        }
        if total == 3 {
            // K+minor vs K
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

    /// Simple endgame detection.
    pub fn is_endgame(&self) -> bool {
        let total_major = self.piece_count(Piece::Queen, Color::White)
            + self.piece_count(Piece::Queen, Color::Black)
            + self.piece_count(Piece::Rook, Color::White)
            + self.piece_count(Piece::Rook, Color::Black);
        total_major <= 2
    }

    /// Whether `color` has any piece other than pawns and king.
    #[inline]
    pub fn has_non_pawn_material(&self, color: Color) -> bool {
        let side_occ = self.color_occupancy(color);
        let pawn_king = self.piece_bb(Piece::Pawn, color) | self.piece_bb(Piece::King, color);
        (side_occ & !pawn_king) != 0
    }

    /// Material count (positive = White advantage).
    pub fn material_count(&self) -> i32 {
        const VALUES: [i32; 6] = [100, 320, 330, 500, 900, 20000];
        let mut score = 0i32;
        for piece in Piece::ALL {
            let count_w = self.piece_count(piece, Color::White) as i32;
            let count_b = self.piece_count(piece, Color::Black) as i32;
            score += VALUES[piece.index()] * (count_w - count_b);
        }
        score
    }

    /// Mobility for a given color (number of legal moves).
    pub fn mobility(&self, color: Color) -> i32 {
        // We generate pseudo-legal moves here for speed in evaluation
        self.generate_pseudo_legal_moves(color).len() as i32
    }

    /// Total piece count on the board.
    pub fn total_piece_count(&self) -> u32 {
        count_bits(self.all_occupancy())
    }

    /// Pawn shield in front of the king.
    pub fn pawn_shield(&self, color: Color, king_square: Square) -> Bitboard {
        let king_file = king_square.file() as i32;
        let king_rank = king_square.rank() as i32;
        let shield_rank = king_rank + color.pawn_direction() / 8;

        if !(0..=7).contains(&shield_rank) {
            return 0;
        }

        let mut mask = 0u64;
        for df in -1..=1 {
            let f = king_file + df;
            if (0..8).contains(&f) {
                let sq = Square::from_file_rank(f as u8, shield_rank as u8);
                mask |= sq.bitboard();
            }
        }

        self.pawns(color) & mask
    }
}

// ── Castling rights update table ────────────────────────────────────────────
// When a piece moves from or to a square, AND the castling rights with this value.
impl Board {
    #[inline(always)]
    fn castling_rights_after(&self, from: Square, to: Square) -> u8 {
        if !self.chess960 {
            return self.castling_rights
                & CASTLING_RIGHTS_UPDATE[from.index()]
                & CASTLING_RIGHTS_UPDATE[to.index()];
        }
        let mut rights = self.castling_rights;
        let white_king = Square::from_file_rank(self.castling_king_file[0], 0);
        let black_king = Square::from_file_rank(self.castling_king_file[1], 7);
        if from == white_king || to == white_king {
            rights &= !(WHITE_KING_CASTLE | WHITE_QUEEN_CASTLE);
        }
        if from == black_king || to == black_king {
            rights &= !(BLACK_KING_CASTLE | BLACK_QUEEN_CASTLE);
        }
        let wk = self.castling_rook_from(Color::White, true);
        let wq = self.castling_rook_from(Color::White, false);
        let bk = self.castling_rook_from(Color::Black, true);
        let bq = self.castling_rook_from(Color::Black, false);
        if from == wk || to == wk {
            rights &= !WHITE_KING_CASTLE;
        }
        if from == wq || to == wq {
            rights &= !WHITE_QUEEN_CASTLE;
        }
        if from == bk || to == bk {
            rights &= !BLACK_KING_CASTLE;
        }
        if from == bq || to == bq {
            rights &= !BLACK_QUEEN_CASTLE;
        }
        rights
    }

    fn write_castling_fen(&self, fen: &mut String) {
        if self.castling_rights == 0 {
            fen.push('-');
            return;
        }
        if self.chess960 {
            if self.castling_rights & WHITE_KING_CASTLE != 0 {
                fen.push((b'A' + self.castling_rook_file[0]) as char);
            }
            if self.castling_rights & WHITE_QUEEN_CASTLE != 0 {
                fen.push((b'A' + self.castling_rook_file[1]) as char);
            }
            if self.castling_rights & BLACK_KING_CASTLE != 0 {
                fen.push((b'a' + self.castling_rook_file[2]) as char);
            }
            if self.castling_rights & BLACK_QUEEN_CASTLE != 0 {
                fen.push((b'a' + self.castling_rook_file[3]) as char);
            }
            return;
        }
        if self.castling_rights & WHITE_KING_CASTLE != 0 {
            fen.push('K');
        }
        if self.castling_rights & WHITE_QUEEN_CASTLE != 0 {
            fen.push('Q');
        }
        if self.castling_rights & BLACK_KING_CASTLE != 0 {
            fen.push('k');
        }
        if self.castling_rights & BLACK_QUEEN_CASTLE != 0 {
            fen.push('q');
        }
    }
}

const CASTLING_RIGHTS_UPDATE: [u8; 64] = {
    let mut table = [ALL_CASTLING; 64];
    // White king or rooks move → lose white castling
    table[Square::E1 as usize] = ALL_CASTLING & !WHITE_KING_CASTLE & !WHITE_QUEEN_CASTLE;
    table[Square::A1 as usize] = ALL_CASTLING & !WHITE_QUEEN_CASTLE;
    table[Square::H1 as usize] = ALL_CASTLING & !WHITE_KING_CASTLE;
    // Black king or rooks move → lose black castling
    table[Square::E8 as usize] = ALL_CASTLING & !BLACK_KING_CASTLE & !BLACK_QUEEN_CASTLE;
    table[Square::A8 as usize] = ALL_CASTLING & !BLACK_QUEEN_CASTLE;
    table[Square::H8 as usize] = ALL_CASTLING & !BLACK_KING_CASTLE;
    table
};

// ── Display ─────────────────────────────────────────────────────────────────

impl fmt::Display for Board {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f)?;
        for rank in (0..8).rev() {
            write!(f, "  {} ", rank + 1)?;
            for file in 0..8u8 {
                let sq = Square::from_file_rank(file, rank);
                let ch = match self.piece_on(sq) {
                    Some((piece, color)) => {
                        let c = piece.to_char();
                        if color == Color::Black {
                            c.to_ascii_lowercase()
                        } else {
                            c
                        }
                    }
                    None => '.',
                };
                write!(f, " {ch}")?;
            }
            writeln!(f)?;
        }
        writeln!(f, "\n     a b c d e f g h")?;
        writeln!(f, "\n  Side: {}", self.side_to_move)?;
        write!(f, "  Castling: ")?;
        if self.castling_rights == 0 {
            write!(f, "-")?;
        } else {
            if self.castling_rights & WHITE_KING_CASTLE != 0 {
                write!(f, "K")?;
            }
            if self.castling_rights & WHITE_QUEEN_CASTLE != 0 {
                write!(f, "Q")?;
            }
            if self.castling_rights & BLACK_KING_CASTLE != 0 {
                write!(f, "k")?;
            }
            if self.castling_rights & BLACK_QUEEN_CASTLE != 0 {
                write!(f, "q")?;
            }
        }
        writeln!(f)?;
        write!(f, "  En passant: ")?;
        match self.en_passant {
            Some(sq) => writeln!(f, "{sq}")?,
            None => writeln!(f, "-")?,
        }
        writeln!(f, "  FEN: {}", self.to_fen())?;
        Ok(())
    }
}

impl fmt::Debug for Board {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() {
        crate::init();
    }

    fn play_uci(board: &mut Board, moves: &[&str]) {
        for uci in moves {
            let mv = board
                .generate_legal_moves()
                .iter()
                .find(|mv| mv.to_uci() == *uci)
                .copied()
                .unwrap_or_else(|| panic!("expected legal move {uci}"));
            board.make_move(mv);
        }
    }

    // ── FEN parsing / serialization ─────────────────────────────────────────

    #[test]
    fn test_starting_position_fen_roundtrip() {
        setup();
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let board = Board::from_fen(fen).unwrap();
        assert_eq!(board.to_fen(), fen);
    }

    #[test]
    fn test_fen_with_position() {
        setup();
        let fen = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
        let board = Board::from_fen(fen).unwrap();
        assert_eq!(board.to_fen(), fen);
    }

    #[test]
    fn test_fen_various_positions() {
        setup();
        let fens = [
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1",
            "rnbqkbnr/pp1ppppp/8/2p5/4P3/8/PPPP1PPP/RNBQKBNR w KQkq c6 0 2",
            "8/8/8/8/8/8/8/4K2R w K - 0 1",
            "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "r2q1rk1/pP1p2pp/Q4n2/bbp1p3/Np6/1B3NBn/pPPP1PPP/R3K2R b KQ - 0 1",
        ];
        for fen in fens {
            let board =
                Board::from_fen(fen).unwrap_or_else(|e| panic!("Failed to parse FEN '{fen}': {e}"));
            assert_eq!(board.to_fen(), fen, "FEN roundtrip failed for: {fen}");
        }
    }

    #[test]
    fn test_fen_error_handling() {
        setup();
        assert!(Board::from_fen("").is_err());
        assert!(Board::from_fen("garbage").is_err());
        assert!(Board::from_fen("8/8/8/8/8/8/8/8 w KQkq -").is_err()); // missing fields
        assert!(Board::from_fen("8/8/8/8/8/8/8/8 x KQkq - 0 1").is_err()); // invalid color
        assert!(Board::from_fen("8/8/8/8/8/8/8/8 w XQkq - 0 1").is_err()); // invalid castling
    }

    // ── Piece access ────────────────────────────────────────────────────────

    #[test]
    fn test_piece_counts_starting_position() {
        setup();
        let board = Board::new();
        assert_eq!(board.piece_count(Piece::Pawn, Color::White), 8);
        assert_eq!(board.piece_count(Piece::Pawn, Color::Black), 8);
        assert_eq!(board.piece_count(Piece::Rook, Color::White), 2);
        assert_eq!(board.piece_count(Piece::Knight, Color::White), 2);
        assert_eq!(board.piece_count(Piece::Bishop, Color::White), 2);
        assert_eq!(board.piece_count(Piece::Queen, Color::White), 1);
        assert_eq!(board.piece_count(Piece::King, Color::White), 1);
        assert_eq!(board.total_piece_count(), 32);
    }

    #[test]
    fn test_king_square() {
        setup();
        let board = Board::new();
        assert_eq!(board.king_square(Color::White), Square::E1);
        assert_eq!(board.king_square(Color::Black), Square::E8);
    }

    #[test]
    fn test_piece_on_starting_position() {
        setup();
        let board = Board::new();
        assert_eq!(
            board.piece_on(Square::E1),
            Some((Piece::King, Color::White))
        );
        assert_eq!(
            board.piece_on(Square::D8),
            Some((Piece::Queen, Color::Black))
        );
        assert_eq!(
            board.piece_on(Square::A2),
            Some((Piece::Pawn, Color::White))
        );
        assert_eq!(board.piece_on(Square::E4), None);
    }

    #[test]
    fn test_piece_of_known_color_on() {
        setup();
        let board = Board::new();
        assert_eq!(
            board.piece_of_color_on(Square::G1, Color::White),
            Some(Piece::Knight)
        );
        assert_eq!(board.piece_of_color_on(Square::G1, Color::Black), None);
        assert_eq!(board.piece_of_color_on(Square::E4, Color::White), None);
    }

    #[test]
    fn mailbox_tracks_bitboards_through_make_unmake() {
        setup();
        let mut board =
            Board::from_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1")
                .unwrap();

        let assert_consistent = |board: &Board| {
            for square in 0..64 {
                let square = Square::from_index(square);
                let expected = [Color::White, Color::Black].into_iter().find_map(|color| {
                    Piece::ALL.into_iter().find_map(|piece| {
                        (board.piece_bb(piece, color) & square.bitboard() != 0)
                            .then_some((piece, color))
                    })
                });
                assert_eq!(board.piece_on(square), expected, "{square}");
            }
        };

        assert_consistent(&board);
        let moves = board.generate_legal_moves();
        for &mv in &moves {
            board.make_move(mv);
            assert_consistent(&board);
            board.unmake_move(mv);
            assert_consistent(&board);
        }
    }

    // ── Make / Unmake move ──────────────────────────────────────────────────

    #[test]
    fn test_make_unmake_preserves_board() {
        setup();
        let mut board = Board::new();
        let original_fen = board.to_fen();

        let mv = Move::double_pawn(Square::E2, Square::E4);
        board.make_move(mv);
        assert_ne!(board.to_fen(), original_fen);

        board.unmake_move(mv);
        assert_eq!(board.to_fen(), original_fen);
    }

    #[test]
    fn test_make_unmake_all_legal_moves_starting() {
        setup();
        let mut board = Board::new();
        let original_fen = board.to_fen();
        let original_hash = board.hash;

        let moves = board.generate_legal_moves();
        for mv in &moves {
            board.make_move(*mv);
            board.unmake_move(*mv);
            assert_eq!(
                board.to_fen(),
                original_fen,
                "FEN mismatch after make/unmake of {mv}"
            );
            assert_eq!(
                board.hash, original_hash,
                "Zobrist hash mismatch after make/unmake of {mv}"
            );
        }
    }

    #[test]
    fn test_make_unmake_captures_restore_piece() {
        setup();
        // Position where white can capture: Nxe5
        let fen = "rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 0 3";
        let mut board = Board::from_fen(fen).unwrap();
        let original_fen = board.to_fen();
        let original_hash = board.hash;

        let moves = board.generate_legal_moves();
        for mv in &moves {
            board.make_move(*mv);
            board.unmake_move(*mv);
            assert_eq!(
                board.to_fen(),
                original_fen,
                "FEN mismatch after make/unmake of {mv}"
            );
            assert_eq!(
                board.hash, original_hash,
                "Hash mismatch after make/unmake of {mv}"
            );
        }
    }

    #[test]
    fn test_make_unmake_stress_kiwipete() {
        setup();
        // KiwiPete — complex position with many special moves
        let fen = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
        let mut board = Board::from_fen(fen).unwrap();
        let original_fen = board.to_fen();
        let original_hash = board.hash;

        let moves = board.generate_legal_moves();
        for mv in &moves {
            board.make_move(*mv);
            board.unmake_move(*mv);
            assert_eq!(
                board.to_fen(),
                original_fen,
                "FEN mismatch after make/unmake of {mv} in KiwiPete"
            );
            assert_eq!(
                board.hash, original_hash,
                "Hash mismatch after make/unmake of {mv} in KiwiPete"
            );
        }
    }

    #[test]
    fn test_make_unmake_deep_sequence() {
        setup();
        let mut board = Board::new();
        let original_fen = board.to_fen();

        // Play a sequence of moves and then undo all of them
        let mut played = Vec::new();
        for _ in 0..10 {
            let moves = board.generate_legal_moves();
            if moves.is_empty() {
                break;
            }
            let mv = moves[0];
            board.make_move(mv);
            played.push(mv);
        }

        // Undo all moves in reverse
        for mv in played.iter().rev() {
            board.unmake_move(*mv);
        }

        assert_eq!(
            board.to_fen(),
            original_fen,
            "Board not restored after deep sequence"
        );
    }

    // ── En passant ──────────────────────────────────────────────────────────

    #[test]
    fn test_en_passant_after_double_push() {
        setup();
        let mut board = Board::new();
        let mv = Move::double_pawn(Square::E2, Square::E4);
        board.make_move(mv);
        assert_eq!(board.en_passant, Some(Square::E3));
    }

    #[test]
    fn test_en_passant_cleared_after_non_double_push() {
        setup();
        let mut board = Board::new();
        // 1. e4 e5 2. Nf3 — en passant should be cleared
        board.make_move(Move::double_pawn(Square::E2, Square::E4));
        assert_eq!(board.en_passant, Some(Square::E3));
        board.make_move(Move::double_pawn(Square::E7, Square::E5));
        assert_eq!(board.en_passant, Some(Square::E6));
        board.make_move(Move::quiet(Square::G1, Square::F3)); // knight move
        assert_eq!(
            board.en_passant, None,
            "EP should be cleared after non-double-push"
        );
    }

    // ── Castling rights ─────────────────────────────────────────────────────

    #[test]
    fn test_castling_rights_revoked_on_king_move() {
        setup();
        let fen = "r3k2r/pppppppp/8/8/8/8/PPPPPPPP/R3K2R w KQkq - 0 1";
        let mut board = Board::from_fen(fen).unwrap();
        assert_eq!(board.castling_rights, ALL_CASTLING);

        // Move white king
        board.make_move(Move::quiet(Square::E1, Square::F1));
        assert_eq!(
            board.castling_rights & (WHITE_KING_CASTLE | WHITE_QUEEN_CASTLE),
            0,
            "White castling should be revoked after king move"
        );
        // Black castling should be preserved
        assert_ne!(
            board.castling_rights & (BLACK_KING_CASTLE | BLACK_QUEEN_CASTLE),
            0
        );
    }

    #[test]
    fn test_castling_rights_revoked_on_rook_move() {
        setup();
        let fen = "r3k2r/pppppppp/8/8/8/8/PPPPPPPP/R3K2R w KQkq - 0 1";
        let mut board = Board::from_fen(fen).unwrap();

        board.make_move(Move::quiet(Square::H1, Square::G1)); // White kingside rook
        assert_eq!(
            board.castling_rights & WHITE_KING_CASTLE,
            0,
            "White kingside castling should be revoked"
        );
        assert_ne!(
            board.castling_rights & WHITE_QUEEN_CASTLE,
            0,
            "White queenside castling should be preserved"
        );
    }

    #[test]
    fn test_castling_rights_revoked_on_rook_capture() {
        setup();
        // Black rook captures white's a1 rook directly
        let fen = "r3k2r/pppppppp/8/8/8/8/1PPPPPPP/r3K2R w Kkq - 0 1";
        let board = Board::from_fen(fen).unwrap();

        // White's a1 rook is already captured (replaced by black rook)
        // So white queenside castling should already be gone? No — the FEN says only K.
        // Let's test: black rook is ON a1, meaning the white rook was captured.
        assert_eq!(
            board.castling_rights & WHITE_QUEEN_CASTLE,
            0,
            "White queenside castling should not be possible with no rook on a1"
        );
    }

    // ── Zobrist hashing ─────────────────────────────────────────────────────

    #[test]
    fn test_zobrist_incremental_consistency() {
        setup();
        let mut board = Board::new();
        let recalculate_hash = |b: &Board| -> u64 {
            // Rebuild hash from scratch
            let z = zobrist();
            let mut h = 0u64;
            for color in [Color::White, Color::Black] {
                for piece in Piece::ALL {
                    for sq_idx in iter_bits(b.piece_bb(piece, color)) {
                        h ^= z.piece_keys[color.index()][piece.index()][sq_idx];
                    }
                }
            }
            h ^= z.castling_keys[b.castling_rights as usize];
            if let Some(ep) = b.en_passant {
                h ^= z.en_passant_keys[ep.file() as usize];
            }
            if b.side_to_move == Color::Black {
                h ^= z.side_to_move_key;
            }
            h
        };

        // Verify initial hash
        assert_eq!(
            board.hash,
            recalculate_hash(&board),
            "Initial hash mismatch"
        );

        // Play 20 random-ish moves and verify hash at each step
        for _ in 0..20 {
            let moves = board.generate_legal_moves();
            if moves.is_empty() {
                break;
            }
            board.make_move(moves[0]);
            assert_eq!(
                board.hash,
                recalculate_hash(&board),
                "Hash mismatch after move, FEN: {}",
                board.to_fen()
            );
        }
    }

    #[test]
    fn test_tt_hash_separates_rule_fifty_buckets() {
        setup();
        let early = Board::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let same_bucket = Board::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 15 1").unwrap();
        let later = Board::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 16 1").unwrap();

        assert_eq!(early.hash, later.hash);
        assert_eq!(early.tt_hash(), same_bucket.tt_hash());
        assert_ne!(early.tt_hash(), later.tt_hash());
    }

    #[test]
    fn tt_hash_after_matches_make_move_for_special_moves() {
        setup();
        let positions = [
            Board::new(),
            Board::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 7 1").unwrap(),
            Board::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 12 1").unwrap(),
            Board::from_fen("4k2r/P6P/8/8/8/8/8/4K3 w - - 15 1").unwrap(),
        ];

        for mut board in positions {
            for &mv in board.generate_legal_moves().iter() {
                let predicted = board.tt_hash_after(mv);
                let mut after = board.clone();
                after.make_move(mv);
                assert_eq!(
                    predicted,
                    after.tt_hash(),
                    "predicted wrong TT key for {} from {}",
                    mv.to_uci(),
                    board.to_fen()
                );
            }
        }
    }

    // ── Null move ───────────────────────────────────────────────────────────

    #[test]
    fn test_null_move_round_trip() {
        setup();
        let mut board = Board::new();
        let original_fen = board.to_fen();
        let original_hash = board.hash;

        board.make_null_move();
        assert_eq!(board.side_to_move, Color::Black);
        assert_eq!(board.en_passant, None);

        board.unmake_null_move();
        assert_eq!(board.to_fen(), original_fen);
        assert_eq!(board.hash, original_hash);
    }

    #[test]
    fn repetition_does_not_cross_null_move_boundaries() {
        setup();
        let mut board =
            Board::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 20 1").unwrap();
        let original_hash = board.hash;

        board.make_null_move();
        board.make_null_move();

        assert_eq!(board.hash, original_hash);
        assert!(!board.has_repetition());
        assert!(!board.is_search_draw(2));
    }

    // ── Game state ──────────────────────────────────────────────────────────

    #[test]
    fn test_not_in_check_starting() {
        setup();
        let board = Board::new();
        assert!(!board.in_check());
    }

    #[test]
    fn game_and_search_repetition_rules_are_distinct() {
        setup();
        let cycle = ["g1f3", "g8f6", "f3g1", "f6g8"];
        let mut board = Board::new();

        play_uci(&mut board, &cycle[..2]);
        assert!(!board.has_repetition());
        play_uci(&mut board, &cycle[2..]);
        assert!(board.has_repetition());
        assert!(!board.has_threefold_repetition());
        assert!(!board.is_draw());
        assert!(!board.is_search_draw(4));

        play_uci(&mut board, &["g1f3"]);
        assert!(board.is_search_draw(5));

        let mut threefold = Board::new();
        play_uci(&mut threefold, &cycle);
        play_uci(&mut threefold, &cycle);
        assert!(threefold.has_threefold_repetition());
        assert!(threefold.is_draw());
    }

    #[test]
    fn test_material_count_starting() {
        setup();
        let board = Board::new();
        assert_eq!(board.material_count(), 0);
        assert_eq!(board.total_material(), 10_296);
    }

    #[test]
    fn total_material_tracks_captures_promotions_and_unmake() {
        setup();
        let mut capture = Board::from_fen("4k3/8/8/3p4/4P3/8/8/4K3 w - - 0 1").unwrap();
        let capture_move = capture
            .generate_legal_moves()
            .iter()
            .copied()
            .find(|mv| mv.to_uci() == "e4d5")
            .unwrap();
        assert_eq!(capture.total_material(), 218);
        capture.make_move(capture_move);
        assert_eq!(capture.total_material(), 109);
        capture.unmake_move(capture_move);
        assert_eq!(capture.total_material(), 218);

        let mut promotion = Board::from_fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let promotion_move = promotion
            .generate_legal_moves()
            .iter()
            .copied()
            .find(|mv| mv.to_uci() == "a7a8q")
            .unwrap();
        assert_eq!(promotion.total_material(), 109);
        promotion.make_move(promotion_move);
        assert_eq!(promotion.total_material(), 1242);
        promotion.unmake_move(promotion_move);
        assert_eq!(promotion.total_material(), 109);
    }

    #[test]
    fn test_has_non_pawn_material() {
        setup();

        let start = Board::new();
        assert!(start.has_non_pawn_material(Color::White));
        assert!(start.has_non_pawn_material(Color::Black));

        let kp_only = Board::from_fen("8/8/4k3/8/8/8/4p3/4K3 w - - 0 1").unwrap();
        assert!(!kp_only.has_non_pawn_material(Color::White));
        assert!(!kp_only.has_non_pawn_material(Color::Black));

        let kq = Board::from_fen("4k3/8/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
        assert!(kq.has_non_pawn_material(Color::White));
        assert!(!kq.has_non_pawn_material(Color::Black));
    }

    #[test]
    fn test_insufficient_material_kk() {
        setup();
        let board = Board::from_fen("8/8/4k3/8/8/4K3/8/8 w - - 0 1").unwrap();
        assert!(board.is_draw(), "K vs K should be a draw");
    }

    #[test]
    fn test_insufficient_material_kbk() {
        setup();
        let board = Board::from_fen("8/8/4k3/8/8/4K3/8/3B4 w - - 0 1").unwrap();
        assert!(board.is_draw(), "K+B vs K should be a draw");
    }

    #[test]
    fn test_stalemate() {
        setup();
        // Classic stalemate: black king trapped by queen
        let mut board = Board::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").unwrap();
        assert!(!board.in_check());
        assert!(board.is_stalemate(), "This should be stalemate");
    }

    #[test]
    fn test_checkmate_precedes_fifty_move_draw() {
        setup();
        let mut mate = Board::from_fen("7k/6Q1/6K1/8/8/8/8/8 b - - 100 1").unwrap();
        assert!(mate.in_check());
        assert!(mate.generate_legal_moves().is_empty());
        assert!(!mate.is_draw());

        let drawable = Board::from_fen("7k/8/6KQ/8/8/8/8/8 w - - 100 1").unwrap();
        assert!(drawable.is_draw());
    }

    #[test]
    fn test_display_does_not_panic() {
        setup();
        let board = Board::new();
        let _ = format!("{board}");

        let board2 =
            Board::from_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1")
                .unwrap();
        let _ = format!("{board2}");
    }

    #[test]
    fn chess960_cleared_back_rank_encodes_king_takes_rook() {
        setup();
        let mut board = Board::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        board.set_chess960(true);
        let moves = board.generate_legal_moves();
        assert!(
            moves
                .iter()
                .any(|mv| mv.to_uci() == "e1h1" && mv.flag == MoveFlag::KingCastle)
        );
        assert!(
            moves
                .iter()
                .any(|mv| mv.to_uci() == "e1a1" && mv.flag == MoveFlag::QueenCastle)
        );
        assert!(!moves.iter().any(|mv| mv.to_uci() == "e1g1"));
        assert!(!moves.iter().any(|mv| mv.to_uci() == "e1c1"));
    }

    #[test]
    fn chess960_castle_make_unmake_restores_startpos() {
        setup();
        let mut board = Board::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        board.set_chess960(true);
        let hash = board.hash;
        let fen = board.to_fen();
        let castle = *board
            .generate_legal_moves()
            .iter()
            .find(|mv| mv.to_uci() == "e1h1")
            .expect("FRC kingside");
        board.make_move(castle);
        assert_eq!(
            board.piece_on(Square::G1),
            Some((Piece::King, Color::White))
        );
        assert_eq!(
            board.piece_on(Square::F1),
            Some((Piece::Rook, Color::White))
        );
        assert_eq!(board.piece_on(Square::E1), None);
        assert_eq!(board.piece_on(Square::H1), None);
        board.unmake_move(castle);
        assert_eq!(board.hash, hash);
        assert_eq!(board.to_fen(), fen);
    }

    #[test]
    fn shredder_fen_parses_rook_files() {
        setup();
        let board = Board::from_fen("4k3/8/8/8/8/8/8/4KR2 w F - 0 1").unwrap();
        assert!(board.is_chess960());
        let mut playable = board;
        let moves = playable.generate_legal_moves();
        assert!(
            moves
                .iter()
                .any(|mv| mv.to_uci() == "e1f1" && mv.flag == MoveFlag::KingCastle)
        );
    }

    #[test]
    fn standard_startpos_fen_unchanged_without_chess960() {
        setup();
        let board = Board::new();
        assert!(!board.is_chess960());
        assert_eq!(
            board.to_fen(),
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        );
    }
}
