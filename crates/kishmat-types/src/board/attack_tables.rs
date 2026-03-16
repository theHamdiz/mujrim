//! Precomputed attack tables for all piece types.
//! Knight and king use simple lookup tables.
//! Bishop and rook use magic bitboard lookup.
//! Pawn attacks are computed per-color.
//!
//! All tables are stored in a single heap-allocated struct behind OnceLock —
//! no `static mut`, fully safe, zero-cost after init.

use crate::bitboard::*;

// ── Attack table aggregate ──────────────────────────────────────────────────

/// All precomputed attack lookup tables, heap-allocated.
pub struct AttackTables {
    pub pawn_attacks: [[Bitboard; 64]; 2],
    pub knight_attacks: [Bitboard; 64],
    pub king_attacks: [Bitboard; 64],
    pub bishop_masks: [Bitboard; 64],
    pub rook_masks: [Bitboard; 64],
    pub bishop_table: Box<[[Bitboard; 512]; 64]>,
    pub rook_table: Box<[[Bitboard; 4096]; 64]>,
}

static TABLES: std::sync::OnceLock<AttackTables> = std::sync::OnceLock::new();

/// Returns a reference to the global attack tables (initialized once).
#[inline(always)]
pub fn tables() -> &'static AttackTables {
    TABLES.get_or_init(AttackTables::init)
}

/// Initialize all attack tables. Safe to call multiple times (idempotent).
pub fn init_attack_tables() {
    let _ = tables();
}

impl AttackTables {
    fn init() -> Self {
        let mut t = Self {
            pawn_attacks: [[0; 64]; 2],
            knight_attacks: [0; 64],
            king_attacks: [0; 64],
            bishop_masks: [0; 64],
            rook_masks: [0; 64],
            bishop_table: Box::new([[0; 512]; 64]),
            rook_table: Box::new([[0; 4096]; 64]),
        };
        t.init_pawn_attacks();
        t.init_knight_attacks();
        t.init_king_attacks();
        t.init_magic_tables();
        t
    }

    fn init_pawn_attacks(&mut self) {
        for sq in 0..64 {
            let bb = 1u64 << sq;
            let file = sq % 8;
            // White pawn attacks (upward diagonals)
            let mut white = 0u64;
            if file > 0 { white |= bb << 7; }
            if file < 7 { white |= bb << 9; }
            if sq >= 56 { white = 0; }

            // Black pawn attacks (downward diagonals)
            let mut black = 0u64;
            if file > 0 && sq >= 9 { black |= bb >> 9; }
            if file < 7 && sq >= 7 { black |= bb >> 7; }
            if sq < 8 { black = 0; }

            self.pawn_attacks[0][sq] = white;
            self.pawn_attacks[1][sq] = black;
        }
    }

    fn init_knight_attacks(&mut self) {
        for sq in 0..64usize {
            let bb = 1u64 << sq;
            let mut attacks = 0u64;

            if bb & NOT_FILE_A != 0 {
                attacks |= bb << 15;
                if sq >= 17 { attacks |= bb >> 17; }
            }
            if bb & NOT_FILE_H != 0 {
                attacks |= bb << 17;
                if sq >= 15 { attacks |= bb >> 15; }
            }
            if bb & NOT_FILE_AB != 0 {
                attacks |= bb << 6;
                if sq >= 10 { attacks |= bb >> 10; }
            }
            if bb & NOT_FILE_GH != 0 {
                attacks |= bb << 10;
                if sq >= 6 { attacks |= bb >> 6; }
            }

            self.knight_attacks[sq] = attacks & FULL_BOARD;
        }
    }

    fn init_king_attacks(&mut self) {
        for sq in 0..64usize {
            let bb = 1u64 << sq;
            let mut attacks = 0u64;

            attacks |= bb << 8;
            if sq >= 8 { attacks |= bb >> 8; }

            if bb & NOT_FILE_A != 0 {
                if sq >= 1 { attacks |= bb >> 1; }
                attacks |= bb << 7;
                if sq >= 9 { attacks |= bb >> 9; }
            }
            if bb & NOT_FILE_H != 0 {
                attacks |= bb << 1;
                attacks |= bb << 9;
                if sq >= 7 { attacks |= bb >> 7; }
            }

            self.king_attacks[sq] = attacks & FULL_BOARD;
        }
    }

    fn init_magic_tables(&mut self) {
        for sq in 0..64 {
            // Bishop
            let b_mask = bishop_mask(sq);
            self.bishop_masks[sq] = b_mask;
            let b_bits = BISHOP_BITS[sq];
            let b_magic = BISHOP_MAGICS[sq];

            let mut subset: Bitboard = 0;
            loop {
                let index = (subset.wrapping_mul(b_magic) >> (64 - b_bits)) as usize;
                self.bishop_table[sq][index] = bishop_attacks_slow(sq, subset);
                subset = subset.wrapping_sub(b_mask) & b_mask;
                if subset == 0 { break; }
            }

            // Rook
            let r_mask = rook_mask(sq);
            self.rook_masks[sq] = r_mask;
            let r_bits = ROOK_BITS[sq];
            let r_magic = ROOK_MAGICS[sq];

            subset = 0;
            loop {
                let index = (subset.wrapping_mul(r_magic) >> (64 - r_bits)) as usize;
                self.rook_table[sq][index] = rook_attacks_slow(sq, subset);
                subset = subset.wrapping_sub(r_mask) & r_mask;
                if subset == 0 { break; }
            }
        }
    }
}

// ── Public attack lookup functions ──────────────────────────────────────────

/// Returns the bishop attacks for a given square and occupancy.
#[inline(always)]
pub fn bishop_attacks(sq: usize, occupancy: Bitboard) -> Bitboard {
    let t = tables();
    let mask = t.bishop_masks[sq];
    let index = ((occupancy & mask).wrapping_mul(BISHOP_MAGICS[sq]) >> (64 - BISHOP_BITS[sq])) as usize;
    t.bishop_table[sq][index]
}

/// Returns the rook attacks for a given square and occupancy.
#[inline(always)]
pub fn rook_attacks(sq: usize, occupancy: Bitboard) -> Bitboard {
    let t = tables();
    let mask = t.rook_masks[sq];
    let index = ((occupancy & mask).wrapping_mul(ROOK_MAGICS[sq]) >> (64 - ROOK_BITS[sq])) as usize;
    t.rook_table[sq][index]
}

/// Returns the queen attacks (bishop + rook combined).
#[inline(always)]
pub fn queen_attacks(sq: usize, occupancy: Bitboard) -> Bitboard {
    bishop_attacks(sq, occupancy) | rook_attacks(sq, occupancy)
}

/// Returns the knight attacks for a given square.
#[inline(always)]
pub fn knight_attacks(sq: usize) -> Bitboard {
    tables().knight_attacks[sq]
}

/// Returns the king attacks for a given square.
#[inline(always)]
pub fn king_attacks(sq: usize) -> Bitboard {
    tables().king_attacks[sq]
}

/// Returns pawn attacks for a given square and color.
#[inline(always)]
pub fn pawn_attacks(color: usize, sq: usize) -> Bitboard {
    tables().pawn_attacks[color][sq]
}

/// Returns all pieces attacking a given square for a board.
/// Used by SEE (Static Exchange Evaluation).
#[inline]
pub fn all_attackers(sq: usize, occupancy: Bitboard, white: &[Bitboard; 6], black: &[Bitboard; 6]) -> Bitboard {
    let t = tables();
    let mut attackers = 0u64;
    // Pawns
    attackers |= t.pawn_attacks[1][sq] & white[0]; // White pawns attack like black captures
    attackers |= t.pawn_attacks[0][sq] & black[0]; // Black pawns attack like white captures
    // Knights
    attackers |= t.knight_attacks[sq] & (white[1] | black[1]);
    // Bishops/Queens (diagonal)
    let diag = bishop_attacks(sq, occupancy);
    attackers |= diag & (white[2] | black[2] | white[4] | black[4]);
    // Rooks/Queens (straight)
    let orth = rook_attacks(sq, occupancy);
    attackers |= orth & (white[3] | black[3] | white[4] | black[4]);
    // Kings
    attackers |= t.king_attacks[sq] & (white[5] | black[5]);
    attackers
}

// ── Magic constants ─────────────────────────────────────────────────────────

/// Number of relevant bits for bishop magic at each square.
pub const BISHOP_BITS: [u32; 64] = [
    6, 5, 5, 5, 5, 5, 5, 6,
    5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 7, 7, 7, 7, 5, 5,
    5, 5, 7, 9, 9, 7, 5, 5,
    5, 5, 7, 9, 9, 7, 5, 5,
    5, 5, 7, 7, 7, 7, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5,
    6, 5, 5, 5, 5, 5, 5, 6,
];

/// Number of relevant bits for rook magic at each square.
pub const ROOK_BITS: [u32; 64] = [
    12, 11, 11, 11, 11, 11, 11, 12,
    11, 10, 10, 10, 10, 10, 10, 11,
    11, 10, 10, 10, 10, 10, 10, 11,
    11, 10, 10, 10, 10, 10, 10, 11,
    11, 10, 10, 10, 10, 10, 10, 11,
    11, 10, 10, 10, 10, 10, 10, 11,
    11, 10, 10, 10, 10, 10, 10, 11,
    12, 11, 11, 11, 11, 11, 11, 12,
];

/// Pre-found bishop magic numbers (from CPW / Stockfish).
pub const BISHOP_MAGICS: [u64; 64] = [
    0x0002020202020200, 0x0002020202020000, 0x0004010202000000, 0x0004040080000000,
    0x0001104000000000, 0x0000821040000000, 0x0000410410400000, 0x0000104104104000,
    0x0000040404040400, 0x0000020202020200, 0x0000040102020000, 0x0000040400800000,
    0x0000011040000000, 0x0000008210400000, 0x0000004104104000, 0x0000002082082000,
    0x0004000808080800, 0x0002000404040400, 0x0001000202020200, 0x0000800802004000,
    0x0000800400A00000, 0x0000200100884000, 0x0000400082082000, 0x0000200041041000,
    0x0002080010101000, 0x0001040008080800, 0x0000208004010400, 0x0000404004010200,
    0x0000840000802000, 0x0000404002011000, 0x0000808001041000, 0x0000404000820800,
    0x0001041000202000, 0x0000820800101000, 0x0000104400080800, 0x0000020080080080,
    0x0000404040040100, 0x0000808100020100, 0x0001010100020800, 0x0000808080010400,
    0x0000820820004000, 0x0000410410002000, 0x0000082088001000, 0x0000002011000800,
    0x0000080100400400, 0x0001010101000200, 0x0002020202000400, 0x0001010101000200,
    0x0000410410400000, 0x0000208208200000, 0x0000002084100000, 0x0000000020880000,
    0x0000001002020000, 0x0000040408020000, 0x0004040404040000, 0x0002020202020000,
    0x0000104104104000, 0x0000002082082000, 0x0000000020841000, 0x0000000000208800,
    0x0000000010020200, 0x0000000404080200, 0x0000040404040400, 0x0002020202020200,
];

/// Pre-found rook magic numbers.
pub const ROOK_MAGICS: [u64; 64] = [
    0x0080001020400080, 0x0040001000200040, 0x0080081000200080, 0x0080040800100080,
    0x0080020400080080, 0x0080010200040080, 0x0080008001000200, 0x0080002040800100,
    0x0000800020400080, 0x0000400020005000, 0x0000801000200080, 0x0000800800100080,
    0x0000800400080080, 0x0000800200040080, 0x0000800100020080, 0x0000800040800100,
    0x0000208000400080, 0x0000404000201000, 0x0000808010002000, 0x0000808008001000,
    0x0000808004000800, 0x0000808002000400, 0x0000010100020004, 0x0000020000408104,
    0x0000208080004000, 0x0000200040005000, 0x0000100080200080, 0x0000080080100080,
    0x0000040080080080, 0x0000020080040080, 0x0000010080800200, 0x0000800080004100,
    0x0000204000800080, 0x0000200040401000, 0x0000100080802000, 0x0000080080801000,
    0x0000040080800800, 0x0000020080800400, 0x0000020001010004, 0x0000800040800100,
    0x0000204000808000, 0x0000200040008080, 0x0000100020008080, 0x0000080010008080,
    0x0000040008008080, 0x0000020004008080, 0x0000010002008080, 0x0000004081020004,
    0x0000204000800080, 0x0000200040008080, 0x0000100020008080, 0x0000080010008080,
    0x0000040008008080, 0x0000020004008080, 0x0000800100020080, 0x0000800041000080,
    0x00FFFCDDFCED714A, 0x007FFCDDFCED714A, 0x003FFFCDFFD88096, 0x0000040810002101,
    0x0001000204080011, 0x0001000204000801, 0x0001000082000401, 0x0001FFFAABFAD1A2,
];

// ── Slow attack computation (for table init only) ───────────────────────────

fn bishop_mask(sq: usize) -> Bitboard {
    let (r, f) = (sq / 8, sq % 8);
    let mut mask = 0u64;
    let dirs: [(i32, i32); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
    for (dr, df) in dirs {
        let (mut cr, mut cf) = (r as i32 + dr, f as i32 + df);
        while cr > 0 && cr < 7 && cf > 0 && cf < 7 {
            mask |= 1u64 << (cr * 8 + cf);
            cr += dr;
            cf += df;
        }
    }
    mask
}

fn rook_mask(sq: usize) -> Bitboard {
    let (r, f) = (sq / 8, sq % 8);
    let mut mask = 0u64;
    for cr in (r as i32 + 1)..7 { mask |= 1u64 << (cr * 8 + f as i32); }
    for cr in (1..r as i32).rev() { mask |= 1u64 << (cr * 8 + f as i32); }
    for cf in (f as i32 + 1)..7 { mask |= 1u64 << (r as i32 * 8 + cf); }
    for cf in (1..f as i32).rev() { mask |= 1u64 << (r as i32 * 8 + cf); }
    mask
}

fn bishop_attacks_slow(sq: usize, occupancy: Bitboard) -> Bitboard {
    let (r, f) = (sq / 8, sq % 8);
    let mut attacks = 0u64;
    let dirs: [(i32, i32); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
    for (dr, df) in dirs {
        let (mut cr, mut cf) = (r as i32 + dr, f as i32 + df);
        while cr >= 0 && cr <= 7 && cf >= 0 && cf <= 7 {
            let bit = 1u64 << (cr * 8 + cf);
            attacks |= bit;
            if occupancy & bit != 0 { break; }
            cr += dr;
            cf += df;
        }
    }
    attacks
}

fn rook_attacks_slow(sq: usize, occupancy: Bitboard) -> Bitboard {
    let (r, f) = (sq / 8, sq % 8);
    let mut attacks = 0u64;
    let dirs: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    for (dr, df) in dirs {
        let (mut cr, mut cf) = (r as i32 + dr, f as i32 + df);
        while cr >= 0 && cr <= 7 && cf >= 0 && cf <= 7 {
            let bit = 1u64 << (cr * 8 + cf);
            attacks |= bit;
            if occupancy & bit != 0 { break; }
            cr += dr;
            cf += df;
        }
    }
    attacks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::square::Square;

    fn setup() {
        init_attack_tables();
    }

    #[test]
    fn test_knight_attacks_center() {
        setup();
        let attacks = knight_attacks(28);
        assert_eq!(count_bits(attacks), 8);
    }

    #[test]
    fn test_knight_attacks_corner() {
        setup();
        let attacks = knight_attacks(0);
        assert_eq!(count_bits(attacks), 2);
    }

    #[test]
    fn test_king_attacks_center() {
        setup();
        let attacks = king_attacks(28);
        assert_eq!(count_bits(attacks), 8);
    }

    #[test]
    fn test_king_attacks_corner() {
        setup();
        let attacks = king_attacks(0);
        assert_eq!(count_bits(attacks), 3);
    }

    #[test]
    fn test_bishop_attacks_empty_board() {
        setup();
        let attacks = bishop_attacks(27, 0);
        assert_eq!(count_bits(attacks), 13);
    }

    #[test]
    fn test_rook_attacks_empty_board() {
        setup();
        let attacks = rook_attacks(27, 0);
        assert_eq!(count_bits(attacks), 14);
    }

    #[test]
    fn test_queen_attacks_empty_board() {
        setup();
        let q = queen_attacks(27, 0);
        let b = bishop_attacks(27, 0);
        let r = rook_attacks(27, 0);
        assert_eq!(q, b | r);
    }

    #[test]
    fn test_pawn_attacks() {
        setup();
        let attacks = pawn_attacks(0, 28);
        assert!(attacks & Square::D5.bitboard() != 0);
        assert!(attacks & Square::F5.bitboard() != 0);
        assert_eq!(count_bits(attacks), 2);
    }

    #[test]
    fn test_rook_attacks_with_blockers() {
        setup();
        let blockers = 1u64 << Square::A4.index();
        let attacks = rook_attacks(Square::A1.index(), blockers);
        assert!(attacks & Square::A2.bitboard() != 0);
        assert!(attacks & Square::A3.bitboard() != 0);
        assert!(attacks & Square::A4.bitboard() != 0);
        assert!(attacks & Square::A5.bitboard() == 0);
    }
}
