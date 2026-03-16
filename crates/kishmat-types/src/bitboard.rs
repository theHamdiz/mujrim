/// A bitboard is a 64-bit integer where each bit represents a square on the board.
/// Bit 0 = A1, Bit 7 = H1, Bit 56 = A8, Bit 63 = H8.
pub type Bitboard = u64;

// ── File masks ──────────────────────────────────────────────────────────────
pub const FILE_A: Bitboard = 0x0101_0101_0101_0101;
pub const FILE_B: Bitboard = FILE_A << 1;
pub const FILE_C: Bitboard = FILE_A << 2;
pub const FILE_D: Bitboard = FILE_A << 3;
pub const FILE_E: Bitboard = FILE_A << 4;
pub const FILE_F: Bitboard = FILE_A << 5;
pub const FILE_G: Bitboard = FILE_A << 6;
pub const FILE_H: Bitboard = FILE_A << 7;

pub const NOT_FILE_A: Bitboard = !FILE_A;
pub const NOT_FILE_H: Bitboard = !FILE_H;
pub const NOT_FILE_AB: Bitboard = !(FILE_A | FILE_B);
pub const NOT_FILE_GH: Bitboard = !(FILE_G | FILE_H);

// ── Rank masks ──────────────────────────────────────────────────────────────
pub const RANK_1: Bitboard = 0x0000_0000_0000_00FF;
pub const RANK_2: Bitboard = RANK_1 << 8;
pub const RANK_3: Bitboard = RANK_1 << 16;
pub const RANK_4: Bitboard = RANK_1 << 24;
pub const RANK_5: Bitboard = RANK_1 << 32;
pub const RANK_6: Bitboard = RANK_1 << 40;
pub const RANK_7: Bitboard = RANK_1 << 48;
pub const RANK_8: Bitboard = RANK_1 << 56;

pub const FULL_BOARD: Bitboard = 0xFFFF_FFFF_FFFF_FFFF;

/// File mask for a given file index (0-7).
pub const FILES: [Bitboard; 8] = [
    FILE_A, FILE_B, FILE_C, FILE_D, FILE_E, FILE_F, FILE_G, FILE_H,
];

/// Rank mask for a given rank index (0-7).
pub const RANKS: [Bitboard; 8] = [
    RANK_1, RANK_2, RANK_3, RANK_4, RANK_5, RANK_6, RANK_7, RANK_8,
];

// ── Bit operations ──────────────────────────────────────────────────────────

#[inline(always)]
pub fn set_bit(board: &mut Bitboard, square: usize) {
    *board |= 1u64 << square;
}

#[inline(always)]
pub fn clear_bit(board: &mut Bitboard, square: usize) {
    *board &= !(1u64 << square);
}

#[inline(always)]
pub fn is_bit_set(board: Bitboard, square: usize) -> bool {
    (board & (1u64 << square)) != 0
}

#[inline(always)]
pub fn count_bits(board: Bitboard) -> u32 {
    board.count_ones()
}

#[inline(always)]
pub fn get_lsb(board: Bitboard) -> usize {
    debug_assert!(board != 0, "get_lsb called on empty bitboard");
    board.trailing_zeros() as usize
}

/// Pops the least significant set bit, returning its index and mutating the bitboard.
#[inline(always)]
pub fn pop_lsb(board: &mut Bitboard) -> usize {
    let sq = get_lsb(*board);
    *board &= *board - 1; // clears the LSB
    sq
}

/// Iterator over set bits in a bitboard.
pub struct BitboardIter(pub Bitboard);

impl Iterator for BitboardIter {
    type Item = usize;

    #[inline(always)]
    fn next(&mut self) -> Option<usize> {
        if self.0 == 0 {
            None
        } else {
            Some(pop_lsb(&mut self.0))
        }
    }
}

/// Convenience: iterate over all set bits of a bitboard.
#[inline(always)]
pub fn iter_bits(bb: Bitboard) -> BitboardIter {
    BitboardIter(bb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_clear_bit() {
        let mut bb: Bitboard = 0;
        set_bit(&mut bb, 0);
        assert!(is_bit_set(bb, 0));
        clear_bit(&mut bb, 0);
        assert!(!is_bit_set(bb, 0));
    }

    #[test]
    fn test_count_bits() {
        assert_eq!(count_bits(0), 0);
        assert_eq!(count_bits(FULL_BOARD), 64);
        assert_eq!(count_bits(RANK_1), 8);
    }

    #[test]
    fn test_pop_lsb() {
        let mut bb: Bitboard = 0b1010;
        assert_eq!(pop_lsb(&mut bb), 1);
        assert_eq!(bb, 0b1000);
        assert_eq!(pop_lsb(&mut bb), 3);
        assert_eq!(bb, 0);
    }

    #[test]
    fn test_iter_bits() {
        let bb: Bitboard = (1u64 << 5) | (1u64 << 20) | (1u64 << 63);
        let bits: Vec<usize> = iter_bits(bb).collect();
        assert_eq!(bits, vec![5, 20, 63]);
    }

    #[test]
    fn test_file_rank_masks() {
        assert_eq!(count_bits(FILE_A), 8);
        assert_eq!(count_bits(RANK_1), 8);
        assert_eq!(FILE_A & RANK_1, 1); // A1
    }
}
