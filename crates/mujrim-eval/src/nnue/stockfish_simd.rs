//! Runtime-dispatched kernels for the current Stockfish NNUE adapter.

use super::stockfish_format::L1;

pub(crate) fn affine(input: &[u8], weights: &[i8], output: &mut [i32]) {
    debug_assert_eq!(weights.len(), input.len() * output.len());
    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    if std::arch::is_aarch64_feature_detected!("dotprod") {
        // SAFETY: the kernel is selected only after runtime DotProd detection; lengths were
        // validated above and every row has exactly `input.len()` bytes.
        unsafe { return neon::affine_dotprod(input, weights, output) };
    }
    scalar::affine(input, weights, output);
}

pub(crate) fn apply_i16_feature(
    accumulator: &mut [i16; L1],
    weights: &[i16],
    feature: usize,
    sign: i16,
) {
    let row = &weights[feature * L1..(feature + 1) * L1];
    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    {
        // SAFETY: NEON is part of the AArch64 baseline and both slices contain L1 elements.
        unsafe { neon::apply_i16(accumulator, row, sign) };
        return;
    }
    #[allow(unreachable_code)]
    scalar::apply_i16(accumulator, row, sign);
}

pub(crate) fn apply_i8_feature(
    accumulator: &mut [i16; L1],
    weights: &[i8],
    feature: usize,
    sign: i16,
) {
    let row = &weights[feature * L1..(feature + 1) * L1];
    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    {
        // SAFETY: NEON is part of the AArch64 baseline and both slices contain L1 elements.
        unsafe { neon::apply_i8(accumulator, row, sign) };
        return;
    }
    #[allow(unreachable_code)]
    scalar::apply_i8(accumulator, row, sign);
}

pub(crate) fn transform_pair(first: &[i16], second: &[i16], output: &mut [u8]) {
    debug_assert_eq!(first.len(), second.len());
    debug_assert_eq!(first.len(), output.len());
    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    {
        // SAFETY: NEON is part of the AArch64 baseline and the equal-length
        // slices contain a multiple of eight elements for Stockfish L1.
        unsafe { neon::transform_pair(first, second, output) };
        return;
    }
    #[allow(unreachable_code)]
    scalar::transform_pair(first, second, output);
}

pub(crate) fn selected_backend() -> &'static str {
    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    {
        if std::arch::is_aarch64_feature_detected!("dotprod") {
            return "NEON+DotProd";
        }
        return "NEON";
    }
    #[allow(unreachable_code)]
    "scalar"
}

mod scalar {
    pub(super) fn affine(input: &[u8], weights: &[i8], output: &mut [i32]) {
        for (row, value) in weights.chunks_exact(input.len()).zip(output) {
            for (&activation, &weight) in input.iter().zip(row) {
                *value += i32::from(activation) * i32::from(weight);
            }
        }
    }

    pub(super) fn apply_i16(accumulator: &mut [i16], row: &[i16], sign: i16) {
        for (target, &weight) in accumulator.iter_mut().zip(row) {
            *target = target.wrapping_add(weight.wrapping_mul(sign));
        }
    }

    pub(super) fn apply_i8(accumulator: &mut [i16], row: &[i8], sign: i16) {
        for (target, &weight) in accumulator.iter_mut().zip(row) {
            *target = target.wrapping_add(i16::from(weight).wrapping_mul(sign));
        }
    }

    pub(super) fn transform_pair(first: &[i16], second: &[i16], output: &mut [u8]) {
        for ((target, &lhs), &rhs) in output.iter_mut().zip(first).zip(second) {
            let lhs = i32::from(lhs.clamp(0, 255));
            let rhs = i32::from(rhs.clamp(0, 255));
            *target = ((lhs * rhs) / 512) as u8;
        }
    }
}

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
mod neon {
    use std::arch::aarch64::*;

    #[target_feature(enable = "dotprod")]
    pub(super) unsafe fn affine_dotprod(input: &[u8], weights: &[i8], output: &mut [i32]) {
        unsafe {
            debug_assert_eq!(input.len() % 16, 0);
            debug_assert_eq!(output.len() % 4, 0);
            let input_len = input.len();
            for (group, values) in output.chunks_exact_mut(4).enumerate() {
                let row_base = group * 4 * input_len;
                let mut sums = [vdupq_n_s32(0); 4];
                for index in (0..input.len()).step_by(16) {
                    let activations = vld1q_s8(input.as_ptr().add(index).cast());
                    for (row, sum) in sums.iter_mut().enumerate() {
                        let row_weights =
                            vld1q_s8(weights.as_ptr().add(row_base + row * input_len + index));
                        std::arch::asm!(
                            "sdot {sum:v}.4s, {activations:v}.16b, {weights:v}.16b",
                            sum = inout(vreg) *sum,
                            activations = in(vreg) activations,
                            weights = in(vreg) row_weights,
                            options(pure, nomem, nostack)
                        );
                    }
                }
                for (value, sum) in values.iter_mut().zip(sums) {
                    *value = value.wrapping_add(vaddvq_s32(sum));
                }
            }
        }
    }

    pub(super) unsafe fn apply_i16(accumulator: &mut [i16], row: &[i16], sign: i16) {
        unsafe {
            debug_assert_eq!(accumulator.len() % 8, 0);
            for index in (0..accumulator.len()).step_by(8) {
                let current = vld1q_s16(accumulator.as_ptr().add(index));
                let delta = vld1q_s16(row.as_ptr().add(index));
                let updated = if sign > 0 {
                    vaddq_s16(current, delta)
                } else {
                    vsubq_s16(current, delta)
                };
                vst1q_s16(accumulator.as_mut_ptr().add(index), updated);
            }
        }
    }

    pub(super) unsafe fn apply_i8(accumulator: &mut [i16], row: &[i8], sign: i16) {
        unsafe {
            debug_assert_eq!(accumulator.len() % 8, 0);
            for index in (0..accumulator.len()).step_by(8) {
                let current = vld1q_s16(accumulator.as_ptr().add(index));
                let delta = vmovl_s8(vld1_s8(row.as_ptr().add(index)));
                let updated = if sign > 0 {
                    vaddq_s16(current, delta)
                } else {
                    vsubq_s16(current, delta)
                };
                vst1q_s16(accumulator.as_mut_ptr().add(index), updated);
            }
        }
    }

    pub(super) unsafe fn transform_pair(first: &[i16], second: &[i16], output: &mut [u8]) {
        unsafe {
            debug_assert_eq!(first.len() % 8, 0);
            let zero = vdupq_n_s16(0);
            let maximum = vdupq_n_s16(255);
            for index in (0..first.len()).step_by(8) {
                let lhs = vminq_s16(
                    vmaxq_s16(vld1q_s16(first.as_ptr().add(index)), zero),
                    maximum,
                );
                let rhs = vminq_s16(
                    vmaxq_s16(vld1q_s16(second.as_ptr().add(index)), zero),
                    maximum,
                );
                let low = vshrq_n_s32::<9>(vmull_s16(vget_low_s16(lhs), vget_low_s16(rhs)));
                let high = vshrq_n_s32::<9>(vmull_s16(vget_high_s16(lhs), vget_high_s16(rhs)));
                let narrowed = vcombine_s16(vmovn_s32(low), vmovn_s32(high));
                vst1_u8(output.as_mut_ptr().add(index), vqmovun_s16(narrowed));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatched_affine_matches_scalar() {
        let input = (0..64).map(|index| (index % 128) as u8).collect::<Vec<_>>();
        let weights = (0..64 * 8)
            .map(|index| (index as i8).wrapping_mul(17))
            .collect::<Vec<_>>();
        let mut expected = [11_i32; 8];
        let mut actual = expected;
        scalar::affine(&input, &weights, &mut expected);
        affine(&input, &weights, &mut actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn dispatched_feature_updates_match_scalar() {
        let i16_weights = (0..L1 * 2)
            .map(|index| (index as i16).wrapping_mul(29))
            .collect::<Vec<_>>();
        let i8_weights = (0..L1 * 2)
            .map(|index| (index as i8).wrapping_mul(13))
            .collect::<Vec<_>>();
        let mut expected = [7_i16; L1];
        let mut actual = expected;
        scalar::apply_i16(&mut expected, &i16_weights[L1..], -1);
        apply_i16_feature(&mut actual, &i16_weights, 1, -1);
        scalar::apply_i8(&mut expected, &i8_weights[..L1], 1);
        apply_i8_feature(&mut actual, &i8_weights, 0, 1);
        assert_eq!(actual, expected);
    }

    #[test]
    fn dispatched_transform_pair_matches_scalar() {
        let first = (0..L1 / 2)
            .map(|index| index as i16 - 127)
            .collect::<Vec<_>>();
        let second = (0..L1 / 2)
            .map(|index| 383 - index as i16)
            .collect::<Vec<_>>();
        let mut expected = [0_u8; L1 / 2];
        let mut actual = expected;
        scalar::transform_pair(&first, &second, &mut expected);
        transform_pair(&first, &second, &mut actual);
        assert_eq!(actual, expected);
    }
}
