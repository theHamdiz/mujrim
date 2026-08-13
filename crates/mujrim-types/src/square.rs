use std::fmt;
use std::str::FromStr;

/// Represents a square on the chess board (0 = A1, 63 = H8).
/// Layout: index = rank * 8 + file, where rank 0 = row 1, file 0 = column A.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Square {
    A1,
    B1,
    C1,
    D1,
    E1,
    F1,
    G1,
    H1,
    A2,
    B2,
    C2,
    D2,
    E2,
    F2,
    G2,
    H2,
    A3,
    B3,
    C3,
    D3,
    E3,
    F3,
    G3,
    H3,
    A4,
    B4,
    C4,
    D4,
    E4,
    F4,
    G4,
    H4,
    A5,
    B5,
    C5,
    D5,
    E5,
    F5,
    G5,
    H5,
    A6,
    B6,
    C6,
    D6,
    E6,
    F6,
    G6,
    H6,
    A7,
    B7,
    C7,
    D7,
    E7,
    F7,
    G7,
    H7,
    A8,
    B8,
    C8,
    D8,
    E8,
    F8,
    G8,
    H8,
}

impl Square {
    /// All 64 squares in order A1..H8.
    pub const ALL: [Square; 64] = [
        Square::A1,
        Square::B1,
        Square::C1,
        Square::D1,
        Square::E1,
        Square::F1,
        Square::G1,
        Square::H1,
        Square::A2,
        Square::B2,
        Square::C2,
        Square::D2,
        Square::E2,
        Square::F2,
        Square::G2,
        Square::H2,
        Square::A3,
        Square::B3,
        Square::C3,
        Square::D3,
        Square::E3,
        Square::F3,
        Square::G3,
        Square::H3,
        Square::A4,
        Square::B4,
        Square::C4,
        Square::D4,
        Square::E4,
        Square::F4,
        Square::G4,
        Square::H4,
        Square::A5,
        Square::B5,
        Square::C5,
        Square::D5,
        Square::E5,
        Square::F5,
        Square::G5,
        Square::H5,
        Square::A6,
        Square::B6,
        Square::C6,
        Square::D6,
        Square::E6,
        Square::F6,
        Square::G6,
        Square::H6,
        Square::A7,
        Square::B7,
        Square::C7,
        Square::D7,
        Square::E7,
        Square::F7,
        Square::G7,
        Square::H7,
        Square::A8,
        Square::B8,
        Square::C8,
        Square::D8,
        Square::E8,
        Square::F8,
        Square::G8,
        Square::H8,
    ];

    /// Creates a square from file (0-7) and rank (0-7) indices.
    #[inline(always)]
    pub const fn from_file_rank(file: u8, rank: u8) -> Self {
        debug_assert!(file < 8 && rank < 8, "file and rank must be 0..7");
        // SAFETY: the checked arithmetic produces one of the 64 contiguous
        // `Square` discriminants.
        unsafe { std::mem::transmute(rank * 8 + file) }
    }

    /// Returns the square index (0-63).
    #[inline(always)]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Creates a square from a 0-63 index.
    #[inline(always)]
    pub const fn from_index(index: usize) -> Self {
        debug_assert!(index < 64, "square index must be 0..63");
        // SAFETY: every value in 0..64 is a valid contiguous `Square`
        // discriminant.
        unsafe { std::mem::transmute(index as u8) }
    }

    /// Returns the file index (0 = A, 7 = H).
    #[inline(always)]
    pub const fn file(self) -> u8 {
        (self as u8) % 8
    }

    /// Returns the rank index (0 = rank 1, 7 = rank 8).
    #[inline(always)]
    pub const fn rank(self) -> u8 {
        (self as u8) / 8
    }

    /// Returns the file as a character ('a' - 'h').
    #[inline(always)]
    pub const fn file_char(self) -> char {
        (b'a' + self.file()) as char
    }

    /// Returns the rank as a character ('1' - '8').
    #[inline(always)]
    pub const fn rank_char(self) -> char {
        (b'1' + self.rank()) as char
    }

    /// Returns a bitboard with only this square set.
    #[inline(always)]
    pub const fn bitboard(self) -> u64 {
        1u64 << (self as u8)
    }
}

impl fmt::Display for Square {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.file_char(), self.rank_char())
    }
}

impl FromStr for Square {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 2 {
            return Err(format!("invalid square string: '{s}' (expected 2 chars)"));
        }
        let bytes = s.as_bytes();
        let file = bytes[0].wrapping_sub(b'a');
        let rank = bytes[1].wrapping_sub(b'1');
        if file >= 8 || rank >= 8 {
            return Err(format!("invalid square string: '{s}'"));
        }
        Ok(Square::from_file_rank(file, rank))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_square_layout() {
        assert_eq!(Square::A1.index(), 0);
        assert_eq!(Square::H1.index(), 7);
        assert_eq!(Square::A2.index(), 8);
        assert_eq!(Square::H8.index(), 63);
    }

    #[test]
    fn test_file_rank() {
        assert_eq!(Square::E4.file(), 4);
        assert_eq!(Square::E4.rank(), 3);
        assert_eq!(Square::A1.file(), 0);
        assert_eq!(Square::A1.rank(), 0);
        assert_eq!(Square::H8.file(), 7);
        assert_eq!(Square::H8.rank(), 7);
    }

    #[test]
    fn test_from_file_rank() {
        assert_eq!(Square::from_file_rank(4, 3), Square::E4);
        assert_eq!(Square::from_file_rank(0, 0), Square::A1);
        assert_eq!(Square::from_file_rank(7, 7), Square::H8);
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", Square::E4), "e4");
        assert_eq!(format!("{}", Square::A1), "a1");
        assert_eq!(format!("{}", Square::H8), "h8");
    }

    #[test]
    fn test_from_str() {
        assert_eq!("e4".parse::<Square>().unwrap(), Square::E4);
        assert_eq!("a1".parse::<Square>().unwrap(), Square::A1);
        assert_eq!("h8".parse::<Square>().unwrap(), Square::H8);
        assert!("z9".parse::<Square>().is_err());
        assert!("e44".parse::<Square>().is_err());
    }

    #[test]
    fn test_bitboard() {
        assert_eq!(Square::A1.bitboard(), 1);
        assert_eq!(Square::B1.bitboard(), 2);
        assert_eq!(Square::H8.bitboard(), 1u64 << 63);
    }
}
