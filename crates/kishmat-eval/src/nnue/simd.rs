//! SIMD-accelerated operations for NNUE inference.
//!
//! Provides AVX2-optimized and scalar fallback paths for:
//! - SCReLU forward pass (flatten): `sum(screlu(acc) * weights)`
//! - Vector add/sub operations for accumulator updates

use super::network::HIDDEN;

/// Quantization upper bound for SCReLU: clamp(x, 0, QA).
const QA: i16 = 255;

// ═══════════════════════════════════════════════════════════════════
// Public API — dispatches to the best available SIMD implementation
// ═══════════════════════════════════════════════════════════════════

/// Compute SCReLU dot product: sum(screlu(acc[i]) * weights[i])
/// where screlu(x) = clamp(x, 0, QA)²
///
/// This is the hot path of NNUE inference — called twice per eval
/// (once for each perspective).
#[inline(always)]
pub fn flatten(acc: &[i16; HIDDEN], weights: &[i16; HIDDEN]) -> i32 {
    #[cfg(all(feature = "simd", target_feature = "avx2"))]
    unsafe { return avx2::flatten(acc, weights); }

    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    unsafe { return neon::flatten(acc, weights); }

    #[allow(unreachable_code)]
    scalar::flatten(acc, weights)
}

/// Update accumulator in-place: for each add index, add the weight row;
/// for each sub index, subtract the weight row.
#[inline]
pub fn vector_update(
    acc: &mut [i16; HIDDEN],
    all_weights: &[i16],
    adds: &[usize],
    subs: &[usize],
) {
    #[cfg(all(feature = "simd", target_feature = "avx2"))]
    unsafe { return avx2::vector_update(acc, all_weights, adds, subs); }

    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    unsafe { return neon::vector_update(acc, all_weights, adds, subs); }

    #[allow(unreachable_code)]
    scalar::vector_update(acc, all_weights, adds, subs)
}

/// Add src weights to dst accumulator: dst[i] += src[i].
/// Simpler than `vector_update` — no index multiplication overhead.
#[inline]
pub fn vector_add(dst: &mut [i16; HIDDEN], src: &[i16; HIDDEN]) {
    #[cfg(all(feature = "simd", target_feature = "avx2"))]
    unsafe { return avx2::vector_add(dst, src); }

    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    unsafe { return neon::vector_add(dst, src); }

    #[allow(unreachable_code)]
    scalar::vector_add(dst, src)
}

/// Subtract src weights from dst accumulator: dst[i] -= src[i].
/// Used for removing features in incremental accumulator updates.
#[inline]
pub fn vector_sub(dst: &mut [i16; HIDDEN], src: &[i16; HIDDEN]) {
    #[cfg(all(feature = "simd", target_feature = "avx2"))]
    unsafe { return avx2::vector_sub(dst, src); }

    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    unsafe { return neon::vector_sub(dst, src); }

    #[allow(unreachable_code)]
    scalar::vector_sub(dst, src)
}

// ═══════════════════════════════════════════════════════════════════
// Scalar fallback (works on all platforms)
// ═══════════════════════════════════════════════════════════════════

#[allow(dead_code)]
mod scalar {
    use super::*;

    /// Scalar SCReLU: clamp(x, 0, QA)²
    #[inline(always)]
    fn screlu(x: i16) -> i32 {
        let clamped = x.clamp(0, QA) as i32;
        clamped * clamped
    }

    /// Scalar flatten: straightforward loop.
    #[inline]
    pub fn flatten(acc: &[i16; HIDDEN], weights: &[i16; HIDDEN]) -> i32 {
        let mut sum = 0i32;
        for i in 0..HIDDEN {
            sum += screlu(acc[i]) * weights[i] as i32;
        }
        sum
    }

    /// Scalar accumulator add: dst[i] += src[i].
    pub fn vector_add(dst: &mut [i16; HIDDEN], src: &[i16; HIDDEN]) {
        const CHUNK: usize = 16;
        for chunk_start in (0..HIDDEN).step_by(CHUNK) {
            for j in 0..CHUNK {
                dst[chunk_start + j] = dst[chunk_start + j].wrapping_add(src[chunk_start + j]);
            }
        }
    }

    /// Scalar accumulator sub: dst[i] -= src[i].
    pub fn vector_sub(dst: &mut [i16; HIDDEN], src: &[i16; HIDDEN]) {
        const CHUNK: usize = 16;
        for chunk_start in (0..HIDDEN).step_by(CHUNK) {
            for j in 0..CHUNK {
                dst[chunk_start + j] = dst[chunk_start + j].wrapping_sub(src[chunk_start + j]);
            }
        }
    }

    /// Scalar accumulator update.
    pub fn vector_update(
        acc: &mut [i16; HIDDEN],
        all_weights: &[i16],
        adds: &[usize],
        subs: &[usize],
    ) {
        // Process in register-sized chunks for better autovectorization.
        const CHUNK: usize = 16;
        let mut regs = [0i16; CHUNK];

        for chunk_start in (0..HIDDEN).step_by(CHUNK) {
            // Load current values
            for j in 0..CHUNK {
                regs[j] = acc[chunk_start + j];
            }

            // Apply additions
            for &add_idx in adds {
                let offset = add_idx * HIDDEN + chunk_start;
                for j in 0..CHUNK {
                    regs[j] = regs[j].saturating_add(all_weights[offset + j]);
                }
            }

            // Apply subtractions
            for &sub_idx in subs {
                let offset = sub_idx * HIDDEN + chunk_start;
                for j in 0..CHUNK {
                    regs[j] = regs[j].saturating_sub(all_weights[offset + j]);
                }
            }

            // Store back
            for j in 0..CHUNK {
                acc[chunk_start + j] = regs[j];
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// AVX2 implementation (x86_64 with -C target-feature=+avx2)
// ═══════════════════════════════════════════════════════════════════

#[cfg(all(feature = "simd", target_feature = "avx2"))]
mod avx2 {
    #![allow(clippy::undocumented_unsafe_blocks)]
    use super::*;
    use std::arch::x86_64::*;

    /// Number of i16 values per AVX2 register.
    const CHUNK: usize = 16;

    /// AVX2 SCReLU flatten: processes 16 i16 values at a time.
    #[inline]
    pub unsafe fn flatten(acc: &[i16; HIDDEN], weights: &[i16; HIDDEN]) -> i32 {
        unsafe {
            let mut sum = _mm256_setzero_si256();
            let min = _mm256_setzero_si256();
            let max = _mm256_set1_epi16(QA);

            for i in (0..HIDDEN).step_by(CHUNK) {
                let mut v = _mm256_loadu_si256(acc.as_ptr().add(i).cast());
                v = _mm256_min_epi16(_mm256_max_epi16(v, min), max);
                let w = _mm256_loadu_si256(weights.as_ptr().add(i).cast());
                let vw = _mm256_mullo_epi16(v, w);
                let product = _mm256_madd_epi16(v, vw);
                sum = _mm256_add_epi32(sum, product);
            }

            horizontal_sum_i32(sum)
        }
    }

    /// AVX2 accumulator add: dst[i] += src[i].
    pub unsafe fn vector_add(dst: &mut [i16; HIDDEN], src: &[i16; HIDDEN]) {
        unsafe {
            for i in (0..HIDDEN).step_by(CHUNK) {
                let d = _mm256_loadu_si256(dst.as_ptr().add(i).cast());
                let s = _mm256_loadu_si256(src.as_ptr().add(i).cast());
                let r = _mm256_add_epi16(d, s);
                _mm256_storeu_si256(dst.as_mut_ptr().add(i).cast(), r);
            }
        }
    }

    /// AVX2 accumulator sub: dst[i] -= src[i].
    pub unsafe fn vector_sub(dst: &mut [i16; HIDDEN], src: &[i16; HIDDEN]) {
        unsafe {
            for i in (0..HIDDEN).step_by(CHUNK) {
                let d = _mm256_loadu_si256(dst.as_ptr().add(i).cast());
                let s = _mm256_loadu_si256(src.as_ptr().add(i).cast());
                let r = _mm256_sub_epi16(d, s);
                _mm256_storeu_si256(dst.as_mut_ptr().add(i).cast(), r);
            }
        }
    }

    /// AVX2 accumulator update: add/subtract weight rows using 256-bit ops.
    pub unsafe fn vector_update(
        acc: &mut [i16; HIDDEN],
        all_weights: &[i16],
        adds: &[usize],
        subs: &[usize],
    ) {
        unsafe {
            for i in (0..HIDDEN).step_by(CHUNK) {
                let mut v = _mm256_loadu_si256(acc.as_ptr().add(i).cast());

                for &add_idx in adds {
                    let offset = add_idx * HIDDEN + i;
                    let w = _mm256_loadu_si256(all_weights.as_ptr().add(offset).cast());
                    v = _mm256_adds_epi16(v, w);
                }

                for &sub_idx in subs {
                    let offset = sub_idx * HIDDEN + i;
                    let w = _mm256_loadu_si256(all_weights.as_ptr().add(offset).cast());
                    v = _mm256_subs_epi16(v, w);
                }

                _mm256_storeu_si256(acc.as_mut_ptr().add(i).cast(), v);
            }
        }
    }

    /// Horizontal sum of 8 × i32 in a __m256i register.
    #[inline(always)]
    unsafe fn horizontal_sum_i32(sum: __m256i) -> i32 {
        unsafe {
            let upper_128 = _mm256_extracti128_si256::<1>(sum);
            let lower_128 = _mm256_castsi256_si128(sum);
            let sum_128 = _mm_add_epi32(upper_128, lower_128);
            let upper_64 = _mm_unpackhi_epi64(sum_128, sum_128);
            let sum_64 = _mm_add_epi32(upper_64, sum_128);
            let upper_32 = _mm_shuffle_epi32::<0b00_00_00_01>(sum_64);
            let sum_32 = _mm_add_epi32(upper_32, sum_64);
            _mm_cvtsi128_si32(sum_32)
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// NEON implementation (aarch64 — Apple Silicon, ARM64 Linux)
// ═══════════════════════════════════════════════════════════════════

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
#[allow(dead_code)]
mod neon {
    #![allow(clippy::undocumented_unsafe_blocks)]
    use super::*;
    use std::arch::aarch64::*;

    /// Number of i16 values per NEON register (128-bit).
    const CHUNK: usize = 8;

    /// NEON SCReLU flatten: processes 8 i16 values at a time.
    /// Formula: sum(clamp(acc[i], 0, QA)² × weights[i])
    #[inline]
    pub unsafe fn flatten(acc: &[i16; HIDDEN], weights: &[i16; HIDDEN]) -> i32 {
        unsafe {
            let mut sum0 = vdupq_n_s32(0);
            let mut sum1 = vdupq_n_s32(0);
            let min = vdupq_n_s16(0);
            let max = vdupq_n_s16(QA);

            for i in (0..HIDDEN).step_by(CHUNK) {
                // Load 8 accumulator values and clamp to [0, QA]
                let v_raw = vld1q_s16(acc.as_ptr().add(i));
                let v = vminq_s16(vmaxq_s16(v_raw, min), max);

                // Load 8 weights
                let w = vld1q_s16(weights.as_ptr().add(i));

                // Compute v * w (element-wise i16 multiply, keeping low 16 bits)
                let vw = vmulq_s16(v, w);

                // Widening multiply-accumulate: v * vw → i32
                // Low half: multiply low 4 × i16 pairs, accumulate into i32
                let v_lo = vget_low_s16(v);
                let vw_lo = vget_low_s16(vw);
                sum0 = vmlal_s16(sum0, v_lo, vw_lo);

                // High half
                let v_hi = vget_high_s16(v);
                let vw_hi = vget_high_s16(vw);
                sum1 = vmlal_s16(sum1, v_hi, vw_hi);
            }

            // Combine the two accumulators
            let total = vaddq_s32(sum0, sum1);
            // Horizontal sum: 4 × i32 → scalar
            vaddvq_s32(total)
        }
    }

    /// NEON accumulator add: dst[i] += src[i].
    pub unsafe fn vector_add(dst: &mut [i16; HIDDEN], src: &[i16; HIDDEN]) {
        unsafe {
            for i in (0..HIDDEN).step_by(CHUNK) {
                let d = vld1q_s16(dst.as_ptr().add(i));
                let s = vld1q_s16(src.as_ptr().add(i));
                let r = vaddq_s16(d, s);
                vst1q_s16(dst.as_mut_ptr().add(i), r);
            }
        }
    }

    /// NEON accumulator sub: dst[i] -= src[i].
    pub unsafe fn vector_sub(dst: &mut [i16; HIDDEN], src: &[i16; HIDDEN]) {
        unsafe {
            for i in (0..HIDDEN).step_by(CHUNK) {
                let d = vld1q_s16(dst.as_ptr().add(i));
                let s = vld1q_s16(src.as_ptr().add(i));
                let r = vsubq_s16(d, s);
                vst1q_s16(dst.as_mut_ptr().add(i), r);
            }
        }
    }

    /// NEON accumulator update: add/subtract weight rows using 128-bit ops.
    pub unsafe fn vector_update(
        acc: &mut [i16; HIDDEN],
        all_weights: &[i16],
        adds: &[usize],
        subs: &[usize],
    ) {
        unsafe {
            for i in (0..HIDDEN).step_by(CHUNK) {
                let mut v = vld1q_s16(acc.as_ptr().add(i));

                for &add_idx in adds {
                    let offset = add_idx * HIDDEN + i;
                    let w = vld1q_s16(all_weights.as_ptr().add(offset));
                    v = vqaddq_s16(v, w); // saturating add
                }

                for &sub_idx in subs {
                    let offset = sub_idx * HIDDEN + i;
                    let w = vld1q_s16(all_weights.as_ptr().add(offset));
                    v = vqsubq_s16(v, w); // saturating sub
                }

                vst1q_s16(acc.as_mut_ptr().add(i), v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scalar_screlu() {
        // screlu(0) = 0
        assert_eq!(scalar::flatten(&[0; HIDDEN], &[1; HIDDEN]), 0);

        // screlu(255) = 255² = 65025; with weight 1, sum = 65025 * HIDDEN
        let mut acc = [0i16; HIDDEN];
        acc[0] = 255;
        let mut weights = [0i16; HIDDEN];
        weights[0] = 1;
        assert_eq!(scalar::flatten(&acc, &weights), 65025);

        // Negative values should be clamped to 0
        acc[0] = -100;
        assert_eq!(scalar::flatten(&acc, &weights), 0);

        // Values > QA should be clamped to QA
        acc[0] = 500;
        assert_eq!(scalar::flatten(&acc, &weights), 65025); // clamp(500, 0, 255) = 255, 255² = 65025
    }

    #[test]
    fn test_vector_update() {
        let mut acc = [0i16; HIDDEN];
        let mut weights = vec![0i16; 768 * HIDDEN]; // Fake feature weights

        // Set feature 0 weights to all 10s
        for i in 0..HIDDEN {
            weights[i] = 10;
        }
        // Set feature 1 weights to all 5s
        for i in 0..HIDDEN {
            weights[HIDDEN + i] = 5;
        }

        // Add feature 0
        scalar::vector_update(&mut acc, &weights, &[0], &[]);
        for i in 0..HIDDEN {
            assert_eq!(acc[i], 10, "After adding feature 0, acc[{i}] should be 10");
        }

        // Add feature 1
        scalar::vector_update(&mut acc, &weights, &[1], &[]);
        for i in 0..HIDDEN {
            assert_eq!(acc[i], 15, "After adding feature 1, acc[{i}] should be 15");
        }

        // Sub feature 0
        scalar::vector_update(&mut acc, &weights, &[], &[0]);
        for i in 0..HIDDEN {
            assert_eq!(acc[i], 5, "After subbing feature 0, acc[{i}] should be 5");
        }
    }

    #[test]
    fn test_flatten_consistency() {
        // The public `flatten` function should match scalar
        let mut acc = [0i16; HIDDEN];
        let mut weights = [0i16; HIDDEN];
        for i in 0..HIDDEN {
            acc[i] = (i % 300) as i16;
            weights[i] = ((i % 50) as i16).wrapping_sub(25);
        }

        let result = flatten(&acc, &weights);
        let expected = scalar::flatten(&acc, &weights);
        assert_eq!(result, expected, "flatten dispatch should match scalar: {result} != {expected}");
    }
}
