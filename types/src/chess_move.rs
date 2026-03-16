use crate::{Piece, Square};
use std::fmt;

/// Flags that encode special move types.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MoveFlag {
    Quiet = 0,
    DoublePawn = 1,
    KingCastle = 2,
    QueenCastle = 3,
    Capture = 4,
    EnPassant = 5,
    Promotion = 6,
    PromotionCapture = 7,
}

/// A chess move, encoding source, destination, promotion piece, and flags.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Move {
    pub from: Square,
    pub to: Square,
    pub promotion: Option<Piece>,
    pub flag: MoveFlag,
}

impl Move {
    /// Creates a quiet (non-capture, non-special) move.
    #[inline(always)]
    pub const fn quiet(from: Square, to: Square) -> Self {
        Self {
            from,
            to,
            promotion: None,
            flag: MoveFlag::Quiet,
        }
    }

    /// Creates a capture move.
    #[inline(always)]
    pub const fn capture(from: Square, to: Square) -> Self {
        Self {
            from,
            to,
            promotion: None,
            flag: MoveFlag::Capture,
        }
    }

    /// Creates a double pawn push.
    #[inline(always)]
    pub const fn double_pawn(from: Square, to: Square) -> Self {
        Self {
            from,
            to,
            promotion: None,
            flag: MoveFlag::DoublePawn,
        }
    }

    /// Creates an en passant capture.
    #[inline(always)]
    pub const fn en_passant(from: Square, to: Square) -> Self {
        Self {
            from,
            to,
            promotion: None,
            flag: MoveFlag::EnPassant,
        }
    }

    /// Creates a kingside castling move (from king square to castled king square).
    #[inline(always)]
    pub const fn king_castle(from: Square, to: Square) -> Self {
        Self {
            from,
            to,
            promotion: None,
            flag: MoveFlag::KingCastle,
        }
    }

    /// Creates a queenside castling move.
    #[inline(always)]
    pub const fn queen_castle(from: Square, to: Square) -> Self {
        Self {
            from,
            to,
            promotion: None,
            flag: MoveFlag::QueenCastle,
        }
    }

    /// Creates a pawn promotion (non-capture).
    #[inline(always)]
    pub const fn promotion(from: Square, to: Square, piece: Piece) -> Self {
        Self {
            from,
            to,
            promotion: Some(piece),
            flag: MoveFlag::Promotion,
        }
    }

    /// Creates a pawn promotion with capture.
    #[inline(always)]
    pub const fn promotion_capture(from: Square, to: Square, piece: Piece) -> Self {
        Self {
            from,
            to,
            promotion: Some(piece),
            flag: MoveFlag::PromotionCapture,
        }
    }

    /// Is this move a capture of any kind?
    #[inline(always)]
    pub const fn is_capture(self) -> bool {
        matches!(
            self.flag,
            MoveFlag::Capture | MoveFlag::EnPassant | MoveFlag::PromotionCapture
        )
    }

    /// Is this a promotion of any kind?
    #[inline(always)]
    pub const fn is_promotion(self) -> bool {
        self.promotion.is_some()
    }

    /// Is this a castling move?
    #[inline(always)]
    pub const fn is_castling(self) -> bool {
        matches!(self.flag, MoveFlag::KingCastle | MoveFlag::QueenCastle)
    }

    /// Format as UCI long algebraic notation (e.g., "e2e4", "e7e8q").
    pub fn to_uci(self) -> String {
        let mut s = format!("{}{}", self.from, self.to);
        if let Some(promo) = self.promotion {
            s.push(promo.to_char().to_ascii_lowercase());
        }
        s
    }

    /// Parse a UCI move string (e.g., "e2e4", "e7e8q").
    /// Returns None for invalid strings.
    pub fn from_uci(s: &str) -> Option<Self> {
        if s.len() < 4 || s.len() > 5 {
            return None;
        }
        let from = s[0..2].parse::<Square>().ok()?;
        let to = s[2..4].parse::<Square>().ok()?;

        if s.len() == 5 {
            let promo = Piece::from_char(s.chars().nth(4)?)?;
            // We don't know if it's a capture from just the UCI string,
            // so mark as Promotion; the board will disambiguate.
            Some(Move::promotion(from, to, promo))
        } else {
            // Basic move; the board will set the correct flag.
            Some(Move::quiet(from, to))
        }
    }
}

impl fmt::Display for Move {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_uci())
    }
}

/// A null/sentinel move for use in move ordering arrays etc.
pub const NULL_MOVE: Move = Move {
    from: Square::A1,
    to: Square::A1,
    promotion: None,
    flag: MoveFlag::Quiet,
};

// ═══════════════════════════════════════════════════════════════════════════
// Stack-allocated move list — avoids heap allocation on every move gen call.
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum number of moves per position (theoretical max is ~218, we use 256).
const MAX_MOVES: usize = 256;

/// A fixed-capacity, stack-allocated move list.
/// This is the primary data structure for move generation — zero heap allocation.
#[derive(Clone)]
pub struct MoveList {
    moves: [Move; MAX_MOVES],
    len: usize,
}

impl Default for MoveList {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

impl MoveList {
    /// Creates an empty move list.
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            // SAFETY: Move is Copy, so uninitialized is fine as long as we only
            // access indices < self.len. We use NULL_MOVE as the init value.
            moves: [NULL_MOVE; MAX_MOVES],
            len: 0,
        }
    }

    /// Returns the number of moves in the list.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the list is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Adds a move to the list.
    #[inline(always)]
    pub fn push(&mut self, mv: Move) {
        debug_assert!(self.len < MAX_MOVES, "MoveList overflow");
        self.moves[self.len] = mv;
        self.len += 1;
    }

    /// Returns a slice of the valid moves.
    #[inline(always)]
    pub fn as_slice(&self) -> &[Move] {
        &self.moves[..self.len]
    }

    /// Returns a mutable slice of the valid moves.
    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [Move] {
        &mut self.moves[..self.len]
    }

    /// Iterates over the moves.
    #[inline(always)]
    pub fn iter(&self) -> std::slice::Iter<'_, Move> {
        self.as_slice().iter()
    }

    /// Sort moves by a comparator.
    #[inline]
    pub fn sort_by<F: FnMut(&Move, &Move) -> std::cmp::Ordering>(&mut self, compare: F) {
        self.as_mut_slice().sort_unstable_by(compare);
    }

    /// Swaps two moves by index.
    #[inline(always)]
    pub fn swap(&mut self, a: usize, b: usize) {
        self.moves.swap(a, b);
    }
}

impl std::ops::Index<usize> for MoveList {
    type Output = Move;
    #[inline(always)]
    fn index(&self, idx: usize) -> &Move {
        debug_assert!(idx < self.len);
        &self.moves[idx]
    }
}

impl<'a> IntoIterator for &'a MoveList {
    type Item = &'a Move;
    type IntoIter = std::slice::Iter<'a, Move>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quiet_move() {
        let m = Move::quiet(Square::E2, Square::E4);
        assert_eq!(m.to_uci(), "e2e4");
        assert!(!m.is_capture());
        assert!(!m.is_promotion());
    }

    #[test]
    fn test_promotion() {
        let m = Move::promotion(Square::E7, Square::E8, Piece::Queen);
        assert_eq!(m.to_uci(), "e7e8q");
        assert!(m.is_promotion());
    }

    #[test]
    fn test_from_uci() {
        let m = Move::from_uci("e2e4").unwrap();
        assert_eq!(m.from, Square::E2);
        assert_eq!(m.to, Square::E4);

        let m = Move::from_uci("e7e8q").unwrap();
        assert_eq!(m.from, Square::E7);
        assert_eq!(m.to, Square::E8);
        assert_eq!(m.promotion, Some(Piece::Queen));

        assert!(Move::from_uci("xyz").is_none());
    }

    #[test]
    fn test_move_list_push_and_iter() {
        let mut ml = MoveList::new();
        assert!(ml.is_empty());
        ml.push(Move::quiet(Square::E2, Square::E4));
        ml.push(Move::quiet(Square::D2, Square::D4));
        assert_eq!(ml.len(), 2);
        assert_eq!(ml[0].to_uci(), "e2e4");
        assert_eq!(ml[1].to_uci(), "d2d4");
    }
}
