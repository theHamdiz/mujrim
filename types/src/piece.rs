use std::fmt;

/// The six chess piece types.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Piece {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

impl Piece {
    /// All piece types in order.
    pub const ALL: [Piece; 6] = [
        Piece::Pawn,
        Piece::Knight,
        Piece::Bishop,
        Piece::Rook,
        Piece::Queen,
        Piece::King,
    ];

    /// Number of distinct piece types.
    pub const COUNT: usize = 6;

    /// Converts a u8 index (0-5) to a Piece.
    #[inline(always)]
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// Converts a FEN character to a Piece (case-insensitive for the piece type).
    pub fn from_char(c: char) -> Option<Self> {
        match c.to_ascii_uppercase() {
            'P' => Some(Piece::Pawn),
            'N' => Some(Piece::Knight),
            'B' => Some(Piece::Bishop),
            'R' => Some(Piece::Rook),
            'Q' => Some(Piece::Queen),
            'K' => Some(Piece::King),
            _ => None,
        }
    }

    /// Returns the FEN character for this piece (uppercase).
    pub const fn to_char(self) -> char {
        match self {
            Piece::Pawn => 'P',
            Piece::Knight => 'N',
            Piece::Bishop => 'B',
            Piece::Rook => 'R',
            Piece::Queen => 'Q',
            Piece::King => 'K',
        }
    }

    /// Returns the piece index (0-5).
    #[inline(always)]
    pub const fn index(self) -> usize {
        self as usize
    }
}

impl fmt::Display for Piece {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_char())
    }
}

/// The two colors in chess.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    White,
    Black,
}

impl Color {
    /// Returns the opponent's color.
    #[inline(always)]
    pub const fn opponent(self) -> Self {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }

    /// Returns the color index (0 for White, 1 for Black).
    #[inline(always)]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Pawn push direction: +8 for White (up), -8 for Black (down).
    #[inline(always)]
    pub const fn pawn_direction(self) -> i32 {
        match self {
            Color::White => 8,
            Color::Black => -8,
        }
    }

    /// The rank from which pawns start (rank index 1 for White, 6 for Black).
    #[inline(always)]
    pub const fn pawn_start_rank(self) -> u8 {
        match self {
            Color::White => 1,
            Color::Black => 6,
        }
    }

    /// The promotion rank (rank index 7 for White, 0 for Black).
    #[inline(always)]
    pub const fn promotion_rank(self) -> u8 {
        match self {
            Color::White => 7,
            Color::Black => 0,
        }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Color::White => write!(f, "White"),
            Color::Black => write!(f, "Black"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_piece_from_char() {
        assert_eq!(Piece::from_char('N'), Some(Piece::Knight));
        assert_eq!(Piece::from_char('n'), Some(Piece::Knight));
        assert_eq!(Piece::from_char('x'), None);
    }

    #[test]
    fn test_piece_to_char() {
        assert_eq!(Piece::Queen.to_char(), 'Q');
        assert_eq!(Piece::Pawn.to_char(), 'P');
    }

    #[test]
    fn test_color_opponent() {
        assert_eq!(Color::White.opponent(), Color::Black);
        assert_eq!(Color::Black.opponent(), Color::White);
    }

    #[test]
    fn test_pawn_direction() {
        assert_eq!(Color::White.pawn_direction(), 8);
        assert_eq!(Color::Black.pawn_direction(), -8);
    }
}
