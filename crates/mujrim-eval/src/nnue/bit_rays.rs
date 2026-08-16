//! Official Viridithas BitRays threat updates.
//!
//! Port of `viridithas/src/nnue/geometry.rs` + `network/threat_updates.rs`
//! `on_change` / `on_move`. King threats are excluded. AVX2 permute matches
//! `geometry/avx2.rs` when the host supports it.

use super::dirty_threats::{ThreatDelta, ThreatDeltaSink, ThreatSnapshot};
use types::chess_move::MoveFlag;
use types::{Move, Piece};

const WHITE_PAWN: u8 = 0x01;
const BLACK_PAWN: u8 = 0x02;
const KNIGHT: u8 = 0x04;
const BISHOP: u8 = 0x08;
const ROOK: u8 = 0x10;
const QUEEN: u8 = 0x20;
const KING: u8 = 0x40;

const NON_KNIGHT: u64 = 0xFEFE_FEFE_FEFE_FEFE;

const PIECE_TO_BIT: [u8; 16] = [
    WHITE_PAWN, BLACK_PAWN, KNIGHT, KNIGHT, BISHOP, BISHOP, ROOK, ROOK, QUEEN, QUEEN, KING, KING,
    0, 0, 0, 0,
];

#[derive(Clone, Copy)]
struct BitRays(u64);

impl BitRays {
    fn flip(self) -> Self {
        Self(self.0.rotate_right(32))
    }

    fn count_ones(self) -> u32 {
        self.0.count_ones()
    }
}

impl std::ops::BitAnd for BitRays {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl std::ops::Not for BitRays {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

impl IntoIterator for BitRays {
    type Item = usize;
    type IntoIter = BitRaysIter;
    fn into_iter(self) -> Self::IntoIter {
        BitRaysIter(self.0)
    }
}

struct BitRaysIter(u64);

impl Iterator for BitRaysIter {
    type Item = usize;
    fn next(&mut self) -> Option<Self::Item> {
        if self.0 == 0 {
            return None;
        }
        let lsb = self.0.trailing_zeros() as usize;
        self.0 &= self.0 - 1;
        Some(lsb)
    }
}

const PERMUTATION: [[u8; 64]; 64] = {
    let offsets: [i32; 64] = [
        0x1F, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x21, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66,
        0x77, 0x12, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0xF2, 0xF1, 0xE2, 0xD3, 0xC4, 0xB5,
        0xA6, 0x97, 0xE1, 0xF0, 0xE0, 0xD0, 0xC0, 0xB0, 0xA0, 0x90, 0xDF, 0xEF, 0xDE, 0xCD, 0xBC,
        0xAB, 0x9A, 0x89, 0xEE, 0xFF, 0xFE, 0xFD, 0xFC, 0xFB, 0xFA, 0xF9, 0x0E, 0x0F, 0x1E, 0x2D,
        0x3C, 0x4B, 0x5A, 0x69,
    ];
    let mut permutations = [[0u8; 64]; 64];
    let mut focus = 0;
    while focus < 64 {
        let mut i = 0;
        while i < 64 {
            let wide_focus = (focus + (focus & 0x38)) as i32;
            let wide_result = offsets[i] + wide_focus;
            let result = ((wide_result & 0x70) >> 1) | (wide_result & 0x07);
            permutations[focus][i] = if wide_result & 0x88 == 0 {
                result as u8
            } else {
                0x80
            };
            i += 1;
        }
        focus += 1;
    }
    permutations
};

const OUTGOING_THREATS: [u64; 12] = [
    0x02_00_00_00_00_00_02_00,
    0x00_00_02_00_02_00_00_00,
    0x01_01_01_01_01_01_01_01,
    0x01_01_01_01_01_01_01_01,
    0xFE_00_FE_00_FE_00_FE_00,
    0xFE_00_FE_00_FE_00_FE_00,
    0x00_FE_00_FE_00_FE_00_FE,
    0x00_FE_00_FE_00_FE_00_FE,
    0xFE_FE_FE_FE_FE_FE_FE_FE,
    0xFE_FE_FE_FE_FE_FE_FE_FE,
    0,
    0,
];

const INCOMING_THREATS_MASK: [u8; 64] = {
    const HORS: u8 = KNIGHT;
    const ORTH: u8 = QUEEN | ROOK;
    const DIAG: u8 = QUEEN | BISHOP;
    const WPNR: u8 = DIAG | WHITE_PAWN;
    const BPNR: u8 = DIAG | BLACK_PAWN;
    [
        HORS, ORTH, ORTH, ORTH, ORTH, ORTH, ORTH, ORTH, HORS, BPNR, DIAG, DIAG, DIAG, DIAG, DIAG,
        DIAG, HORS, ORTH, ORTH, ORTH, ORTH, ORTH, ORTH, ORTH, HORS, WPNR, DIAG, DIAG, DIAG, DIAG,
        DIAG, DIAG, HORS, ORTH, ORTH, ORTH, ORTH, ORTH, ORTH, ORTH, HORS, WPNR, DIAG, DIAG, DIAG,
        DIAG, DIAG, DIAG, HORS, ORTH, ORTH, ORTH, ORTH, ORTH, ORTH, ORTH, HORS, BPNR, DIAG, DIAG,
        DIAG, DIAG, DIAG, DIAG,
    ]
};

const INCOMING_SLIDERS_MASK: [u8; 64] = {
    const ORTH: u8 = QUEEN | ROOK;
    const DIAG: u8 = QUEEN | BISHOP;
    const NULL: u8 = 0x80;
    [
        NULL, ORTH, ORTH, ORTH, ORTH, ORTH, ORTH, ORTH, NULL, DIAG, DIAG, DIAG, DIAG, DIAG, DIAG,
        DIAG, NULL, ORTH, ORTH, ORTH, ORTH, ORTH, ORTH, ORTH, NULL, DIAG, DIAG, DIAG, DIAG, DIAG,
        DIAG, DIAG, NULL, ORTH, ORTH, ORTH, ORTH, ORTH, ORTH, ORTH, NULL, DIAG, DIAG, DIAG, DIAG,
        DIAG, DIAG, DIAG, NULL, ORTH, ORTH, ORTH, ORTH, ORTH, ORTH, ORTH, NULL, DIAG, DIAG, DIAG,
        DIAG, DIAG, DIAG, DIAG,
    ]
};

#[inline(always)]
fn piece_bit(id: u8) -> u8 {
    if id >= 12 {
        0
    } else {
        PIECE_TO_BIT[usize::from(id)]
    }
}

fn permute(focus: usize, mailbox: &[u8; 64], ignore: Option<usize>) -> ([u8; 64], [u8; 64]) {
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    {
        if avx512_vbmi_ready() {
            return unsafe { permute_avx512_vbmi(focus, mailbox, ignore) };
        }
        if avx2_ready() {
            return unsafe { permute_avx2(focus, mailbox, ignore) };
        }
    }
    permute_scalar(focus, mailbox, ignore)
}

fn permute_scalar(focus: usize, mailbox: &[u8; 64], ignore: Option<usize>) -> ([u8; 64], [u8; 64]) {
    let mut indexes = [0x80u8; 64];
    let mut bits = [0u8; 64];
    let perm = &PERMUTATION[focus];
    for i in 0..64 {
        let square = perm[i];
        indexes[i] = square;
        if square >= 64 {
            continue;
        }
        let sq = usize::from(square);
        if ignore == Some(sq) {
            continue;
        }
        bits[i] = piece_bit(mailbox[sq]);
    }
    (indexes, bits)
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx2_ready() -> bool {
    use std::sync::OnceLock;
    static READY: OnceLock<bool> = OnceLock::new();
    *READY.get_or_init(|| std::arch::is_x86_feature_detected!("avx2"))
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx512_vbmi_ready() -> bool {
    use std::sync::OnceLock;
    static READY: OnceLock<bool> = OnceLock::new();
    *READY.get_or_init(|| {
        std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
            && std::arch::is_x86_feature_detected!("avx512vbmi")
    })
}

/// 64-byte VBMI permute of the mailbox, then the official piece-bit LUT.
#[cfg(all(feature = "simd", target_arch = "x86_64"))]
#[target_feature(enable = "avx512f,avx512bw,avx512vbmi")]
unsafe fn permute_avx512_vbmi(
    focus: usize,
    mailbox: &[u8; 64],
    ignore: Option<usize>,
) -> ([u8; 64], [u8; 64]) {
    use std::arch::x86_64::{
        _mm_loadu_si128, _mm512_broadcast_i32x4, _mm512_cmpeq_epi8_mask, _mm512_loadu_si512,
        _mm512_maskz_shuffle_epi8, _mm512_permutexvar_epi8, _mm512_set1_epi8, _mm512_storeu_si512,
    };

    unsafe {
        let mut mb = *mailbox;
        if let Some(sq) = ignore {
            mb[sq] = u8::MAX;
        }
        let bytes = _mm512_loadu_si512(mb.as_ptr().cast());
        let idxs = _mm512_loadu_si512(PERMUTATION[focus].as_ptr().cast());
        let invalid = _mm512_cmpeq_epi8_mask(idxs, _mm512_set1_epi8(0x80u8 as i8));
        let permuted = _mm512_permutexvar_epi8(idxs, bytes);
        let lut = _mm512_broadcast_i32x4(_mm_loadu_si128(PIECE_TO_BIT.as_ptr().cast()));
        let bits = _mm512_maskz_shuffle_epi8(!invalid, lut, permuted);
        let mut indexes = [0x80u8; 64];
        let mut bit_bytes = [0u8; 64];
        _mm512_storeu_si512(indexes.as_mut_ptr().cast(), idxs);
        _mm512_storeu_si512(bit_bytes.as_mut_ptr().cast(), bits);
        (indexes, bit_bytes)
    }
}

/// Official `Vector::mask` / `!unoccupied`: one bit per byte via `pmovmskb`.
#[cfg(all(feature = "simd", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn mask_nonzero_avx2(bits: &[u8; 64]) -> BitRays {
    use std::arch::x86_64::{
        __m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_setzero_si256,
    };
    unsafe {
        let zero = _mm256_setzero_si256();
        let low = _mm256_loadu_si256(bits.as_ptr().cast::<__m256i>());
        let high = _mm256_loadu_si256(bits.as_ptr().cast::<__m256i>().add(1));
        let unoccupied = u64::from(_mm256_movemask_epi8(_mm256_cmpeq_epi8(low, zero)) as u32)
            | (u64::from(_mm256_movemask_epi8(_mm256_cmpeq_epi8(high, zero)) as u32) << 32);
        BitRays(!unoccupied)
    }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn mask_bit_avx2(bits: &[u8; 64], bit: u8) -> BitRays {
    use std::arch::x86_64::{
        __m256i, _mm256_and_si256, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8,
        _mm256_set1_epi8, _mm256_setzero_si256,
    };
    unsafe {
        let zero = _mm256_setzero_si256();
        let needle = _mm256_set1_epi8(bit as i8);
        let low = _mm256_and_si256(_mm256_loadu_si256(bits.as_ptr().cast::<__m256i>()), needle);
        let high = _mm256_and_si256(
            _mm256_loadu_si256(bits.as_ptr().cast::<__m256i>().add(1)),
            needle,
        );
        let absent = u64::from(_mm256_movemask_epi8(_mm256_cmpeq_epi8(low, zero)) as u32)
            | (u64::from(_mm256_movemask_epi8(_mm256_cmpeq_epi8(high, zero)) as u32) << 32);
        BitRays(!absent)
    }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn mask_and_table_avx2(bits: &[u8; 64], table: &[u8; 64]) -> BitRays {
    use std::arch::x86_64::{
        __m256i, _mm256_and_si256, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8,
        _mm256_setzero_si256,
    };
    unsafe {
        let zero = _mm256_setzero_si256();
        let low = _mm256_and_si256(
            _mm256_loadu_si256(bits.as_ptr().cast::<__m256i>()),
            _mm256_loadu_si256(table.as_ptr().cast::<__m256i>()),
        );
        let high = _mm256_and_si256(
            _mm256_loadu_si256(bits.as_ptr().cast::<__m256i>().add(1)),
            _mm256_loadu_si256(table.as_ptr().cast::<__m256i>().add(1)),
        );
        let absent = u64::from(_mm256_movemask_epi8(_mm256_cmpeq_epi8(low, zero)) as u32)
            | (u64::from(_mm256_movemask_epi8(_mm256_cmpeq_epi8(high, zero)) as u32) << 32);
        BitRays(!absent)
    }
}

/// Official `geometry/avx2.rs` mailbox permute: 64-byte pshufb + PIECE_TO_BIT LUT.
#[cfg(all(feature = "simd", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn permute_avx2(
    focus: usize,
    mailbox: &[u8; 64],
    ignore: Option<usize>,
) -> ([u8; 64], [u8; 64]) {
    use std::arch::x86_64::{
        __m256i, _mm_loadu_si128, _mm256_andnot_si256, _mm256_blendv_epi8,
        _mm256_broadcastsi128_si256, _mm256_cmpeq_epi8, _mm256_loadu_si256,
        _mm256_permute2x128_si256, _mm256_set1_epi8, _mm256_shuffle_epi8, _mm256_slli_epi64,
        _mm256_storeu_si256,
    };

    unsafe {
        let mut mb = *mailbox;
        if let Some(sq) = ignore {
            mb[sq] = u8::MAX;
        }
        let bytes0 = _mm256_loadu_si256(mb.as_ptr().cast::<__m256i>());
        let bytes1 = _mm256_loadu_si256(mb.as_ptr().cast::<__m256i>().add(1));
        let idxs0 = _mm256_loadu_si256(PERMUTATION[focus].as_ptr().cast::<__m256i>());
        let idxs1 = _mm256_loadu_si256(PERMUTATION[focus].as_ptr().cast::<__m256i>().add(1));
        let sentinel = _mm256_set1_epi8(0x80u8 as i8);

        let half_swizzle = |bytes0: __m256i, bytes1: __m256i, idxs: __m256i| -> __m256i {
            let mask0 = _mm256_slli_epi64(idxs, 2);
            let mask1 = _mm256_slli_epi64(idxs, 3);
            let lolo0 = _mm256_shuffle_epi8(_mm256_permute2x128_si256(bytes0, bytes0, 0x00), idxs);
            let hihi0 = _mm256_shuffle_epi8(_mm256_permute2x128_si256(bytes0, bytes0, 0x11), idxs);
            let x = _mm256_blendv_epi8(lolo0, hihi0, mask1);
            let lolo1 = _mm256_shuffle_epi8(_mm256_permute2x128_si256(bytes1, bytes1, 0x00), idxs);
            let hihi1 = _mm256_shuffle_epi8(_mm256_permute2x128_si256(bytes1, bytes1, 0x11), idxs);
            let y = _mm256_blendv_epi8(lolo1, hihi1, mask1);
            _mm256_blendv_epi8(x, y, mask0)
        };

        let permuted0 = half_swizzle(bytes0, bytes1, idxs0);
        let permuted1 = half_swizzle(bytes0, bytes1, idxs1);
        let lut = _mm256_broadcastsi128_si256(_mm_loadu_si128(PIECE_TO_BIT.as_ptr().cast()));
        let invalid0 = _mm256_cmpeq_epi8(idxs0, sentinel);
        let invalid1 = _mm256_cmpeq_epi8(idxs1, sentinel);
        let bits0 = _mm256_andnot_si256(invalid0, _mm256_shuffle_epi8(lut, permuted0));
        let bits1 = _mm256_andnot_si256(invalid1, _mm256_shuffle_epi8(lut, permuted1));

        let mut indexes = [0x80u8; 64];
        let mut bits = [0u8; 64];
        _mm256_storeu_si256(indexes.as_mut_ptr().cast::<__m256i>(), idxs0);
        _mm256_storeu_si256(indexes.as_mut_ptr().cast::<__m256i>().add(1), idxs1);
        _mm256_storeu_si256(bits.as_mut_ptr().cast::<__m256i>(), bits0);
        _mm256_storeu_si256(bits.as_mut_ptr().cast::<__m256i>().add(1), bits1);
        (indexes, bits)
    }
}

fn occupied_mask(bits: &[u8; 64]) -> BitRays {
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    if avx2_ready() {
        return unsafe { mask_nonzero_avx2(bits) };
    }
    occupied_mask_scalar(bits)
}

fn occupied_mask_scalar(bits: &[u8; 64]) -> BitRays {
    let mut mask = 0u64;
    for (i, &bit) in bits.iter().enumerate() {
        if bit != 0 {
            mask |= 1u64 << i;
        }
    }
    BitRays(mask)
}

fn closest_occupied(bits: &[u8; 64]) -> BitRays {
    let occupied = occupied_mask(bits);
    let o = occupied.0 | 0x8181_8181_8181_8181;
    BitRays(o ^ o.wrapping_sub(0x0303_0303_0303_0303)) & occupied
}

fn test_king(bits: &[u8; 64]) -> BitRays {
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    if avx2_ready() {
        return unsafe { mask_bit_avx2(bits, KING) };
    }
    test_king_scalar(bits)
}

fn test_king_scalar(bits: &[u8; 64]) -> BitRays {
    let mut mask = 0u64;
    for (i, &bit) in bits.iter().enumerate() {
        if bit & KING != 0 {
            mask |= 1u64 << i;
        }
    }
    BitRays(mask)
}

fn incoming_attackers(bits: &[u8; 64], closest: BitRays) -> BitRays {
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    if avx2_ready() {
        return unsafe { mask_and_table_avx2(bits, &INCOMING_THREATS_MASK) } & closest;
    }
    incoming_attackers_scalar(bits, closest)
}

fn incoming_attackers_scalar(bits: &[u8; 64], closest: BitRays) -> BitRays {
    let mut mask = 0u64;
    for (i, &bit) in bits.iter().enumerate() {
        if bit & INCOMING_THREATS_MASK[i] != 0 {
            mask |= 1u64 << i;
        }
    }
    BitRays(mask) & closest
}

fn incoming_sliders(bits: &[u8; 64], closest: BitRays) -> BitRays {
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    if avx2_ready() {
        return unsafe { mask_and_table_avx2(bits, &INCOMING_SLIDERS_MASK) }
            & closest
            & BitRays(NON_KNIGHT);
    }
    incoming_sliders_scalar(bits, closest)
}

fn incoming_sliders_scalar(bits: &[u8; 64], closest: BitRays) -> BitRays {
    let mut mask = 0u64;
    for (i, &bit) in bits.iter().enumerate() {
        if bit & INCOMING_SLIDERS_MASK[i] != 0 {
            mask |= 1u64 << i;
        }
    }
    BitRays(mask) & closest & BitRays(NON_KNIGHT)
}

fn outgoing_threats(piece: u8, closest: BitRays) -> BitRays {
    BitRays(OUTGOING_THREATS[usize::from(piece)]) & closest
}

fn ray_fill(br: BitRays) -> BitRays {
    let filled = (br.0.wrapping_add(0x7E_7E_7E_7E_7E_7E_7E_7E)) & 0x80_80_80_80_80_80_80_80;
    BitRays(filled.wrapping_sub(filled >> 7))
}

fn push_focus<const ADD: bool, const OUTGOING: bool>(
    sink: &mut impl ThreatDeltaSink,
    indexes: &[u8; 64],
    mailbox: &[u8; 64],
    rays: BitRays,
    piece: u8,
    square: usize,
) {
    for i in rays {
        let other_sq = usize::from(indexes[i]);
        if other_sq >= 64 {
            continue;
        }
        let other = mailbox[other_sq];
        if other == u8::MAX {
            continue;
        }
        let (attacker, from, victim, to) = if OUTGOING {
            (piece, square, other, other_sq)
        } else {
            (other, other_sq, piece, square)
        };
        sink.push_threat_delta(ThreatDelta::new(attacker, from, victim, to, ADD));
    }
}

fn push_discovered<const ADD: bool>(
    sink: &mut impl ThreatDeltaSink,
    indexes: &[u8; 64],
    mailbox: &[u8; 64],
    sliders: BitRays,
    victims: BitRays,
) {
    debug_assert_eq!(sliders.count_ones(), victims.count_ones());
    for (slider_idx, victim_idx) in sliders.into_iter().zip(victims) {
        let from = usize::from(indexes[slider_idx]);
        let to = usize::from(indexes[(victim_idx + 32) % 64]);
        if from >= 64 || to >= 64 {
            continue;
        }
        let attacker = mailbox[from];
        let victim = mailbox[to];
        if attacker == u8::MAX || victim == u8::MAX {
            continue;
        }
        sink.push_threat_delta(ThreatDelta::new(attacker, from, victim, to, !ADD));
    }
}

fn on_change(
    sink: &mut impl ThreatDeltaSink,
    mailbox: &[u8; 64],
    piece: u8,
    square: usize,
    add: bool,
) {
    let (indexes, bits) = permute(square, mailbox, None);
    let non_king = !test_king(&bits);
    let closest = closest_occupied(&bits);
    let outgoing = outgoing_threats(piece, closest & non_king);
    let incoming = incoming_attackers(&bits, closest);
    let sliders = incoming_sliders(&bits, closest);
    if usize::from(piece) / 2 != Piece::King.index() {
        if add {
            push_focus::<true, true>(sink, &indexes, mailbox, outgoing, piece, square);
            push_focus::<true, false>(sink, &indexes, mailbox, incoming, piece, square);
        } else {
            push_focus::<false, true>(sink, &indexes, mailbox, outgoing, piece, square);
            push_focus::<false, false>(sink, &indexes, mailbox, incoming, piece, square);
        }
    }
    let victim_mask = (closest & non_king & BitRays(NON_KNIGHT)).flip();
    let valid = ray_fill(victim_mask) & ray_fill(sliders);
    if add {
        push_discovered::<true>(
            sink,
            &indexes,
            mailbox,
            sliders & valid,
            victim_mask & valid,
        );
    } else {
        push_discovered::<false>(
            sink,
            &indexes,
            mailbox,
            sliders & valid,
            victim_mask & valid,
        );
    }
}

/// Official `on_move`: mailbox is already after the move (src empty, dest occupied).
fn on_move(
    sink: &mut impl ThreatDeltaSink,
    mailbox: &[u8; 64],
    old_piece: u8,
    src: usize,
    new_piece: u8,
    dst: usize,
) {
    let (src_indexes, src_bits) = permute(src, mailbox, Some(dst));
    let (dst_indexes, dst_bits) = permute(dst, mailbox, None);
    let src_non_king = !test_king(&src_bits);
    let dst_non_king = !test_king(&dst_bits);
    let src_closest = closest_occupied(&src_bits);
    let dst_closest = closest_occupied(&dst_bits);
    let src_outgoing = outgoing_threats(old_piece, src_closest & src_non_king);
    let dst_outgoing = outgoing_threats(new_piece, dst_closest & dst_non_king);
    let src_incoming = incoming_attackers(&src_bits, src_closest);
    let dst_incoming = incoming_attackers(&dst_bits, dst_closest);
    let src_sliders = incoming_sliders(&src_bits, src_closest);
    let dst_sliders = incoming_sliders(&dst_bits, dst_closest);
    if usize::from(old_piece) / 2 != Piece::King.index() {
        push_focus::<false, true>(sink, &src_indexes, mailbox, src_outgoing, old_piece, src);
        push_focus::<true, true>(sink, &dst_indexes, mailbox, dst_outgoing, new_piece, dst);
        push_focus::<false, false>(sink, &src_indexes, mailbox, src_incoming, old_piece, src);
        push_focus::<true, false>(sink, &dst_indexes, mailbox, dst_incoming, new_piece, dst);
    }
    let src_victim_mask = (src_closest & src_non_king & BitRays(NON_KNIGHT)).flip();
    let dst_victim_mask = (dst_closest & dst_non_king & BitRays(NON_KNIGHT)).flip();
    let src_valid = ray_fill(src_victim_mask) & ray_fill(src_sliders);
    let dst_valid = ray_fill(dst_victim_mask) & ray_fill(dst_sliders);
    push_discovered::<false>(
        sink,
        &src_indexes,
        mailbox,
        src_sliders & src_valid,
        src_victim_mask & src_valid,
    );
    push_discovered::<true>(
        sink,
        &dst_indexes,
        mailbox,
        dst_sliders & dst_valid,
        dst_victim_mask & dst_valid,
    );
}

fn place(mailbox: &mut [u8; 64], square: usize, id: u8) {
    mailbox[square] = id;
}

fn clear(mailbox: &mut [u8; 64], square: usize) {
    mailbox[square] = u8::MAX;
}

/// Official-style incremental threat diff for one legal move.
pub(super) fn collect_bit_ray_move_deltas(
    sink: &mut impl ThreatDeltaSink,
    snapshot: ThreatSnapshot,
    mv: Move,
) {
    let mut mailbox = snapshot.mailbox();
    let color = snapshot.color();
    let from = mv.from.index();
    let to = mv.to.index();
    let mover = mailbox[from];
    debug_assert_ne!(mover, u8::MAX);
    debug_assert_eq!(usize::from(mover) & 1, color);

    match mv.flag {
        MoveFlag::KingCastle | MoveFlag::QueenCastle => {
            let (rook_from, rook_to) = match (color, mv.flag) {
                (0, MoveFlag::KingCastle) => (7, 5),
                (0, MoveFlag::QueenCastle) => (0, 3),
                (1, MoveFlag::KingCastle) => (63, 61),
                (1, MoveFlag::QueenCastle) => (56, 59),
                _ => unreachable!(),
            };
            let rook = mailbox[rook_from];
            // Official viridithas board::make castle: Sub after each remove, Add after each place.
            clear(&mut mailbox, from);
            on_change(sink, &mailbox, mover, from, false);
            clear(&mut mailbox, rook_from);
            on_change(sink, &mailbox, rook, rook_from, false);
            place(&mut mailbox, to, mover);
            on_change(sink, &mailbox, mover, to, true);
            place(&mut mailbox, rook_to, rook);
            on_change(sink, &mailbox, rook, rook_to, true);
        }
        MoveFlag::EnPassant => {
            let captured_square =
                types::Square::from_file_rank(mv.to.file(), mv.from.rank()).index();
            let captured = mailbox[captured_square];
            on_change(sink, &mailbox, captured, captured_square, false);
            clear(&mut mailbox, captured_square);
            clear(&mut mailbox, from);
            place(&mut mailbox, to, mover);
            on_move(sink, &mailbox, mover, from, mover, to);
        }
        MoveFlag::Promotion | MoveFlag::PromotionCapture => {
            let promoted =
                (mv.promotion.expect("promotion move has a piece").index() * 2 + color) as u8;
            if mv.is_capture() {
                let captured = mailbox[to];
                on_change(sink, &mailbox, captured, to, false);
                clear(&mut mailbox, to);
                on_change(sink, &mailbox, mover, from, false);
                clear(&mut mailbox, from);
                place(&mut mailbox, to, promoted);
                on_change(sink, &mailbox, promoted, to, true);
            } else {
                clear(&mut mailbox, from);
                place(&mut mailbox, to, promoted);
                on_move(sink, &mailbox, mover, from, promoted, to);
            }
        }
        _ if mv.is_capture() => {
            let captured = mailbox[to];
            on_change(sink, &mailbox, mover, from, false);
            on_change(sink, &mailbox, captured, to, false);
            clear(&mut mailbox, from);
            clear(&mut mailbox, to);
            place(&mut mailbox, to, mover);
            on_change(sink, &mailbox, mover, to, true);
        }
        _ => {
            clear(&mut mailbox, from);
            place(&mut mailbox, to, mover);
            on_move(sink, &mailbox, mover, from, mover, to);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PERMUTATION, Piece, ThreatDelta, ThreatDeltaSink, ThreatSnapshot, closest_occupied,
        collect_bit_ray_move_deltas, permute, permute_scalar,
    };
    use types::Board;

    struct DeltaList(Vec<ThreatDelta>);

    impl ThreatDeltaSink for DeltaList {
        fn push_threat_delta(&mut self, delta: ThreatDelta) {
            self.0.push(delta);
        }
    }

    #[test]
    fn permutation_marks_offboard_slots() {
        let e2 = PERMUTATION[12];
        assert!(e2.contains(&0x80));
        assert!(e2.iter().any(|&sq| sq < 64));
        assert_eq!(e2[1], 20, "north ray from e2 starts at e3");
    }

    #[test]
    fn closest_occupied_picks_the_first_piece_on_each_ray() {
        let mut mailbox = [u8::MAX; 64];
        mailbox[12] = 0;
        mailbox[20] = 2;
        mailbox[28] = 4;
        let (_indexes, bits) = permute(12, &mailbox, None);
        let closest = closest_occupied(&bits);
        assert!(closest.0 != 0);
    }

    #[test]
    fn bit_rays_king_walk_emits_no_threat_deltas() {
        types::init();
        let mut board = Board::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").expect("fen");
        let mv = board
            .generate_legal_moves()
            .iter()
            .find(|candidate| candidate.to_uci() == "e1e2")
            .copied()
            .expect("e1e2 is legal");
        let mut sink = DeltaList(Vec::new());
        collect_bit_ray_move_deltas(&mut sink, ThreatSnapshot::from_board(&board), mv);
        assert_eq!(
            sink.0
                .iter()
                .map(|d| (d.attacker(), d.source(), d.attacked(), d.target(), d.add()))
                .collect::<Vec<_>>(),
            Vec::<(usize, usize, usize, usize, bool)>::new()
        );
    }

    #[test]
    fn bit_rays_castle_excludes_king_threats() {
        types::init();
        let mut board = Board::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").expect("fen");
        let mv = board
            .generate_legal_moves()
            .iter()
            .find(|candidate| candidate.to_uci() == "e1g1")
            .copied()
            .expect("e1g1 is legal");
        let mut sink = DeltaList(Vec::new());
        collect_bit_ray_move_deltas(&mut sink, ThreatSnapshot::from_board(&board), mv);
        assert!(!sink.0.is_empty());
        for delta in &sink.0 {
            assert_ne!(delta.attacker() / 2, Piece::King.index());
            assert_ne!(delta.attacked() / 2, Piece::King.index());
        }
    }

    #[test]
    fn bit_rays_e2e4_excludes_king_threats() {
        types::init();
        let mut board = Board::new();
        let mv = board
            .generate_legal_moves()
            .iter()
            .find(|candidate| candidate.to_uci() == "e2e4")
            .copied()
            .expect("e2e4 is legal");
        let mut sink = DeltaList(Vec::new());
        collect_bit_ray_move_deltas(&mut sink, ThreatSnapshot::from_board(&board), mv);
        assert!(!sink.0.is_empty());
        for delta in &sink.0 {
            assert_ne!(delta.attacker() / 2, Piece::King.index());
            assert_ne!(delta.attacked() / 2, Piece::King.index());
        }
    }

    #[test]
    fn mailbox_snapshot_matches_full_snapshot_for_bit_rays() {
        types::init();
        let mut board = Board::new();
        let full = ThreatSnapshot::from_board(&board);
        let cheap = ThreatSnapshot::from_mailbox(&board);
        assert_eq!(full.mailbox(), cheap.mailbox());
        assert_eq!(full.color(), cheap.color());
        let mv = board
            .generate_legal_moves()
            .iter()
            .find(|candidate| candidate.to_uci() == "e2e4")
            .copied()
            .expect("e2e4");
        let mut a = DeltaList(Vec::new());
        let mut b = DeltaList(Vec::new());
        collect_bit_ray_move_deltas(&mut a, full, mv);
        collect_bit_ray_move_deltas(&mut b, cheap, mv);
        assert_eq!(a.0, b.0);
    }

    #[test]
    fn avx512_vbmi_permute_matches_scalar_when_detected() {
        types::init();
        let mailbox = *Board::new().piece_ids();
        #[cfg(all(feature = "simd", target_arch = "x86_64"))]
        if super::avx512_vbmi_ready() {
            for focus in 0..64 {
                let scalar = permute_scalar(focus, &mailbox, Some(28));
                let vbmi = unsafe { super::permute_avx512_vbmi(focus, &mailbox, Some(28)) };
                assert_eq!(scalar.0, vbmi.0, "vbmi indexes focus={focus}");
                assert_eq!(scalar.1, vbmi.1, "vbmi bits focus={focus}");
            }
            return;
        }
        let _ = mailbox;
    }

    #[test]
    fn avx2_permute_matches_scalar_on_startpos() {
        types::init();
        let mailbox = *Board::new().piece_ids();
        for focus in 0..64 {
            let scalar = permute_scalar(focus, &mailbox, None);
            let dispatched = permute(focus, &mailbox, None);
            assert_eq!(scalar.0, dispatched.0, "indexes focus={focus}");
            assert_eq!(scalar.1, dispatched.1, "bits focus={focus}");
            let ignored = permute_scalar(focus, &mailbox, Some(12));
            let ignored_dispatched = permute(focus, &mailbox, Some(12));
            assert_eq!(
                ignored.0, ignored_dispatched.0,
                "ignore indexes focus={focus}"
            );
            assert_eq!(ignored.1, ignored_dispatched.1, "ignore bits focus={focus}");
        }
    }

    #[test]
    fn bit_ray_masks_match_scalar_scans() {
        types::init();
        let mailbox = *Board::new().piece_ids();
        for focus in [0, 12, 28, 36, 63] {
            let (_, bits) = permute_scalar(focus, &mailbox, None);
            assert_eq!(
                super::occupied_mask(&bits).0,
                super::occupied_mask_scalar(&bits).0
            );
            assert_eq!(super::test_king(&bits).0, super::test_king_scalar(&bits).0);
            let closest = super::closest_occupied(&bits);
            assert_eq!(
                super::incoming_attackers(&bits, closest).0,
                super::incoming_attackers_scalar(&bits, closest).0
            );
            assert_eq!(
                super::incoming_sliders(&bits, closest).0,
                super::incoming_sliders_scalar(&bits, closest).0
            );
        }
    }
}
