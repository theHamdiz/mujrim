//! NNUE Network — Akimbo-compatible neural network evaluation.
//!
//! Architecture: 768 → HIDDEN×2 → 1 (perspective net with SCReLU)
//!   with 4 king buckets (mirrored to 8 effective).
//!
//! Binary-compatible with Akimbo's net.bin format (6.0 MB).
//! Trained weights are embedded at compile time via `include_bytes!`.
//!
//! Quantization: QA=255 (feature transformer), QB=64 (output layer)
//! Output formula: (sum / QA + bias) * SCALE / QAB

/// Hidden layer size (per perspective).
pub const HIDDEN: usize = 1024;

/// Number of king buckets in the feature transformer weight table.
/// With mirroring (via the BUCKETS table), 4 buckets → 8 effective.
pub const NUM_BUCKETS: usize = 4;

/// Output scale: converts quantized NNUE output to centipawns.
const SCALE: i32 = 400;

/// Quantization factor for feature transformer weights.
const QA: i32 = 255;

/// Quantization factor for output layer weights.
const QB: i32 = 64;

/// Combined quantization divisor: QA × QB.
const QAB: i32 = QA * QB;

/// King square → bucket mapping.
/// Values 0-3 for queen-side, 4-7 for king-side.
/// Horizontal mirroring is handled by `get_base_index` (which XORs the
/// king square with 7 if file > D), so the BUCKETS table itself encodes
/// both the original and mirrored bucket assignments.
#[rustfmt::skip]
pub static BUCKETS: [usize; 64] = [
    0, 0, 1, 1, 5, 5, 4, 4,
    2, 2, 2, 2, 6, 6, 6, 6,
    3, 3, 3, 3, 7, 7, 7, 7,
    3, 3, 3, 3, 7, 7, 7, 7,
    3, 3, 3, 3, 7, 7, 7, 7,
    3, 3, 3, 3, 7, 7, 7, 7,
    3, 3, 3, 3, 7, 7, 7, 7,
    3, 3, 3, 3, 7, 7, 7, 7,
];

// ═══════════════════════════════════════════════════════════════════
// Network struct — binary-compatible with Akimbo's net.bin
// ═══════════════════════════════════════════════════════════════════

/// A single accumulator: HIDDEN i16 values, 64-byte aligned for SIMD.
#[derive(Clone, Copy)]
#[repr(C, align(64))]
pub struct Accumulator {
    pub vals: [i16; HIDDEN],
}

impl Default for Accumulator {
    fn default() -> Self {
        net().feature_bias
    }
}

/// The NNUE network parameters.
/// Layout must match Akimbo's `repr(C)` struct exactly for `transmute`.
#[repr(C)]
pub struct Network {
    /// Feature transformer weights: [768 * NUM_BUCKETS] accumulators.
    /// Index: bucket * 768 + relative_color * 384 + piece * 64 + square
    pub feature_weights: [Accumulator; 768 * NUM_BUCKETS],
    /// Feature transformer biases: initial accumulator values.
    pub feature_bias: Accumulator,
    /// Output weights: [us_perspective, them_perspective].
    pub output_weights: [Accumulator; 2],
    /// Output bias (scalar).
    pub output_bias: i16,
}

/// Embedded trained network weights (Akimbo-compatible).
/// This is a 6.0 MB binary file loaded at compile time.
static NNUE: Network =
    unsafe { std::mem::transmute(*include_bytes!(concat!("../../resources/net.bin"))) };

// ═══════════════════════════════════════════════════════════════════
// Public API
// ═══════════════════════════════════════════════════════════════════

/// Get reference to the global NNUE parameters.
#[inline(always)]
pub fn net() -> &'static Network {
    &NNUE
}

/// Get the king bucket for a given square (with rank flip for black).
#[inline(always)]
pub fn get_bucket<const SIDE: usize>(mut ksq: usize) -> usize {
    if SIDE == 1 {
        ksq ^= 56; // Rank flip for black
    }
    BUCKETS[ksq]
}

/// Compute the feature base index for a piece from a given perspective.
///
/// # Arguments
/// - `SIDE`: 0 = white perspective, 1 = black perspective (const generic)
/// - `side`: the color of the piece (0 = white, 1 = black)
/// - `pc`: piece type (0=P, 1=N, 2=B, 3=R, 4=Q, 5=K)
/// - `ksq`: the king square of the perspective (0-63, absolute)
///
/// # Returns
/// Base feature index = bucket * 768 + relative_color_offset + piece * 64.
/// Add the piece's square (rank-flipped + mirrored) to get the full index.
#[inline(always)]
pub fn get_base_index<const SIDE: usize>(side: usize, pc: usize, mut ksq: usize) -> usize {
    // Horizontal mirror if king is on files E-H
    if ksq % 8 > 3 {
        ksq ^= 7;
    }

    // Color offset: friendly pieces at index 0, enemy at 384
    let color_offset = if SIDE == 0 {
        [0, 384][side]
    } else {
        [384, 0][side]
    };

    if SIDE == 0 {
        768 * get_bucket::<0>(ksq) + color_offset + 64 * pc
    } else {
        768 * get_bucket::<1>(ksq) + color_offset + 64 * pc
    }
}

/// Forward pass: compute the output from two perspective accumulators.
///
/// Returns centipawn score from the "boys" (side-to-move) perspective.
#[inline]
pub fn forward(boys: &Accumulator, opps: &Accumulator) -> i32 {
    forward_with_network(net(), boys, opps)
}

/// Forward pass using an explicit network instance.
#[inline]
pub fn forward_with_network(network: &Network, boys: &Accumulator, opps: &Accumulator) -> i32 {
    let weights = &network.output_weights;
    let sum = super::simd::flatten(&boys.vals, &weights[0].vals)
        + super::simd::flatten(&opps.vals, &weights[1].vals);
    (sum / QA + i32::from(network.output_bias)) * SCALE / QAB
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_size() {
        let expected = std::mem::size_of::<Network>();
        let actual = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/net.bin"))
            .unwrap()
            .len();
        assert_eq!(
            expected, actual,
            "Network struct size ({expected}) != net.bin size ({actual})"
        );
    }

    #[test]
    fn test_accumulator_default_is_bias() {
        let acc = Accumulator::default();
        let bias = &NNUE.feature_bias;
        assert_eq!(acc.vals, bias.vals);
    }

    #[test]
    fn test_bucket_range() {
        for sq in 0..64 {
            let b = BUCKETS[sq];
            assert!(b < 2 * NUM_BUCKETS, "Bucket {b} out of range for sq={sq}");
        }
    }

    #[test]
    fn test_get_base_index_range() {
        for side in 0..2 {
            for pc in 0..6 {
                for ksq in 0..64 {
                    let idx0 = get_base_index::<0>(side, pc, ksq);
                    let idx1 = get_base_index::<1>(side, pc, ksq);
                    assert!(
                        idx0 < 768 * NUM_BUCKETS * 2,
                        "Base index {idx0} out of range (SIDE=0, side={side}, pc={pc}, ksq={ksq})"
                    );
                    assert!(
                        idx1 < 768 * NUM_BUCKETS * 2,
                        "Base index {idx1} out of range (SIDE=1, side={side}, pc={pc}, ksq={ksq})"
                    );
                }
            }
        }
    }
}
