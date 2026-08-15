//! Runtime-dispatched PSQT accumulation for classical HCE.
//!
//! ARM `dotprod` is an NNUE op and is never selected here.

use std::sync::OnceLock;

use types::{Color, Piece};

use crate::psqt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HceSimdBackend {
    Scalar,
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    Avx2,
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    Avx512Bw,
    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    Neon,
}

impl HceSimdBackend {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Scalar => "scalar",
            #[cfg(all(feature = "simd", target_arch = "x86_64"))]
            Self::Avx2 => "AVX2",
            #[cfg(all(feature = "simd", target_arch = "x86_64"))]
            Self::Avx512Bw => "AVX-512BW",
            #[cfg(all(feature = "simd", target_arch = "aarch64"))]
            Self::Neon => "NEON",
        }
    }
}

struct CombinedTables {
    mg: [[i32; 64]; 6],
    eg: [[i32; 64]; 6],
}

fn combined_tables() -> &'static CombinedTables {
    static TABLES: OnceLock<CombinedTables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut mg = [[0i32; 64]; 6];
        let mut eg = [[0i32; 64]; 6];
        for piece in Piece::ALL {
            let idx = piece.index();
            for sq in 0..64 {
                let (piece_mg, piece_eg) = psqt::combined_value(piece, sq);
                mg[idx][sq] = piece_mg;
                eg[idx][sq] = piece_eg;
            }
        }
        CombinedTables { mg, eg }
    })
}

fn detect_backend() -> HceSimdBackend {
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx512bw") {
            return HceSimdBackend::Avx512Bw;
        }
        if is_x86_feature_detected!("avx2") {
            return HceSimdBackend::Avx2;
        }
    }
    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    {
        HceSimdBackend::Neon
    }
    HceSimdBackend::Scalar
}

fn backend() -> HceSimdBackend {
    static BACKEND: OnceLock<HceSimdBackend> = OnceLock::new();
    *BACKEND.get_or_init(detect_backend)
}

pub fn selected_backend() -> HceSimdBackend {
    backend()
}

pub fn accumulate_psqt(piece: Piece, color: Color, bb: u64) -> (i32, i32) {
    let tables = combined_tables();
    let idx = piece.index();
    let (mg_table, eg_table) = (&tables.mg[idx], &tables.eg[idx]);
    let (mask, sign) = if color == Color::White {
        (bb, 1)
    } else {
        (bb.swap_bytes(), -1)
    };
    (
        sign * sum_masked(mg_table, mask),
        sign * sum_masked(eg_table, mask),
    )
}

fn sum_masked(table: &[i32; 64], mask: u64) -> i32 {
    match backend() {
        #[cfg(all(feature = "simd", target_arch = "x86_64"))]
        HceSimdBackend::Avx512Bw => unsafe { sum_masked_avx512(table, mask) },
        #[cfg(all(feature = "simd", target_arch = "x86_64"))]
        HceSimdBackend::Avx2 => unsafe { sum_masked_avx2(table, mask) },
        #[cfg(all(feature = "simd", target_arch = "aarch64"))]
        HceSimdBackend::Neon => unsafe { sum_masked_neon(table, mask) },
        HceSimdBackend::Scalar => sum_masked_scalar(table, mask),
    }
}

pub fn sum_masked_scalar(table: &[i32; 64], mut mask: u64) -> i32 {
    let mut total = 0;
    while mask != 0 {
        let square = mask.trailing_zeros() as usize;
        total += table[square];
        mask &= mask - 1;
    }
    total
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn sum_masked_avx2(table: &[i32; 64], mask: u64) -> i32 {
    use core::arch::x86_64::*;
    unsafe {
        let mut acc = _mm256_setzero_si256();
        for chunk in 0..8 {
            let bits = ((mask >> (chunk * 8)) & 0xFF) as u32;
            if bits == 0 {
                continue;
            }
            let vals = _mm256_loadu_si256(table.as_ptr().add(chunk * 8).cast());
            let mut lanes = [0i32; 8];
            for (bit, lane) in lanes.iter_mut().enumerate() {
                if bits & (1 << bit) != 0 {
                    *lane = -1;
                }
            }
            let select = _mm256_loadu_si256(lanes.as_ptr().cast());
            acc = _mm256_add_epi32(acc, _mm256_and_si256(vals, select));
        }
        let hi = _mm256_extracti128_si256::<1>(acc);
        let lo = _mm256_castsi256_si128(acc);
        let sum128 = _mm_add_epi32(lo, hi);
        let shuf = _mm_shuffle_epi32::<0b01_00_11_10>(sum128);
        let sum64 = _mm_add_epi32(sum128, shuf);
        let shuf2 = _mm_shuffle_epi32::<0b00_00_00_01>(sum64);
        _mm_cvtsi128_si32(_mm_add_epi32(sum64, shuf2))
    }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
#[target_feature(enable = "avx512bw")]
unsafe fn sum_masked_avx512(table: &[i32; 64], mask: u64) -> i32 {
    use core::arch::x86_64::*;
    unsafe {
        let mut total = 0;
        for chunk in 0..4 {
            let bits = ((mask >> (chunk * 16)) & 0xFFFF) as u16;
            if bits == 0 {
                continue;
            }
            let vals = _mm512_loadu_si512(table.as_ptr().add(chunk * 16).cast());
            total += _mm512_mask_reduce_add_epi32(bits, vals);
        }
        total
    }
}

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
unsafe fn sum_masked_neon(table: &[i32; 64], mask: u64) -> i32 {
    use core::arch::aarch64::*;
    unsafe {
        let mut acc = vdupq_n_s32(0);
        for chunk in 0..16 {
            let bits = ((mask >> (chunk * 4)) & 0xF) as u32;
            if bits == 0 {
                continue;
            }
            let vals = vld1q_s32(table.as_ptr().add(chunk * 4));
            let lanes = [
                if bits & 1 != 0 { -1 } else { 0 },
                if bits & 2 != 0 { -1 } else { 0 },
                if bits & 4 != 0 { -1 } else { 0 },
                if bits & 8 != 0 { -1 } else { 0 },
            ];
            let select = vld1q_s32(lanes.as_ptr());
            acc = vaddq_s32(acc, vandq_s32(vals, select));
        }
        vaddvq_s32(acc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simd_sum_matches_scalar_on_startpos_pawns() {
        types::init();
        let board = types::Board::new();
        let white = board.piece_bb(Piece::Pawn, Color::White);
        let tables = combined_tables();
        let dispatched = accumulate_psqt(Piece::Pawn, Color::White, white);
        let scalar = (
            sum_masked_scalar(&tables.mg[Piece::Pawn.index()], white),
            sum_masked_scalar(&tables.eg[Piece::Pawn.index()], white),
        );
        assert_eq!(dispatched, scalar);
        let _ = selected_backend().name();
    }

    #[test]
    fn black_psqt_uses_vertical_flip() {
        types::init();
        let board = types::Board::new();
        let black = board.piece_bb(Piece::Pawn, Color::Black);
        let (mg, eg) = accumulate_psqt(Piece::Pawn, Color::Black, black);
        let tables = combined_tables();
        let flipped = black.swap_bytes();
        assert_eq!(mg, -sum_masked_scalar(&tables.mg[0], flipped));
        assert_eq!(eg, -sum_masked_scalar(&tables.eg[0], flipped));
    }
}
