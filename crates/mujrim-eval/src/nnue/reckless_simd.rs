//! Architecture-specific kernels for the embedded threat-aware network adapter.

use super::reckless_format::{HIDDEN_SIZE, L2_SIZE, L3_SIZE};
use std::sync::OnceLock;

const FT_QUANT: i16 = 255;
const FT_SHIFT: i32 = 9;

pub(crate) struct ForwardWeights<'a> {
    pub l1: &'a [u8],
    pub l1_biases: &'a [f32],
    pub l2: &'a [f32],
    pub l2_biases: &'a [f32],
    pub l3: &'a [f32],
    pub l3_bias: f32,
}

type ApplyI16Kernel = fn(&mut [i16; HIDDEN_SIZE], &[i16], &[usize], &[usize]);
type ApplyI8Kernel = fn(&mut [i16; HIDDEN_SIZE], &[u8], &[usize], &[usize]);
type ForwardKernel = for<'a> fn(
    [&'a [i16; HIDDEN_SIZE]; 2],
    [&'a [i16; HIDDEN_SIZE]; 2],
    usize,
    ForwardWeights<'a>,
) -> f32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecklessSimdBackend {
    Scalar,
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    X86Avx2Fma,
    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    ArmNeon,
    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    ArmNeonDotprod,
}

impl RecklessSimdBackend {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Scalar => "scalar",
            #[cfg(all(feature = "simd", target_arch = "x86_64"))]
            Self::X86Avx2Fma => "AVX2+FMA",
            #[cfg(all(feature = "simd", target_arch = "aarch64"))]
            Self::ArmNeon => "NEON",
            #[cfg(all(feature = "simd", target_arch = "aarch64"))]
            Self::ArmNeonDotprod => "NEON+DotProd",
        }
    }
}

struct KernelDispatch {
    apply_i16: ApplyI16Kernel,
    apply_i8: ApplyI8Kernel,
    forward: ForwardKernel,
    backend: RecklessSimdBackend,
}

static KERNEL_DISPATCH: OnceLock<KernelDispatch> = OnceLock::new();

#[inline(always)]
fn kernels() -> &'static KernelDispatch {
    KERNEL_DISPATCH.get_or_init(detect_kernels)
}

pub(crate) fn selected_backend() -> RecklessSimdBackend {
    kernels().backend
}

fn detect_kernels() -> KernelDispatch {
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    {
        if std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma")
        {
            return KernelDispatch {
                apply_i16: apply_i16_avx2,
                apply_i8: apply_i8_avx2,
                forward: forward_avx2,
                backend: RecklessSimdBackend::X86Avx2Fma,
            };
        }
    }

    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    {
        if std::arch::is_aarch64_feature_detected!("dotprod") {
            return KernelDispatch {
                apply_i16: apply_i16_neon,
                apply_i8: apply_i8_neon,
                forward: forward_neon_dotprod,
                backend: RecklessSimdBackend::ArmNeonDotprod,
            };
        }
        return KernelDispatch {
            apply_i16: apply_i16_neon,
            apply_i8: apply_i8_neon,
            forward: forward_neon,
            backend: RecklessSimdBackend::ArmNeon,
        };
    }

    #[allow(unreachable_code)]
    KernelDispatch {
        apply_i16: scalar::apply_i16_rows,
        apply_i8: scalar::apply_i8_rows,
        forward: scalar::forward,
        backend: RecklessSimdBackend::Scalar,
    }
}

#[inline]
pub(crate) fn apply_i16_rows(
    accumulator: &mut [i16; HIDDEN_SIZE],
    weights: &[i16],
    adds: &[usize],
    subs: &[usize],
) {
    #[cfg(all(feature = "simd", target_arch = "aarch64", target_feature = "neon"))]
    {
        // SAFETY: NEON is enabled for this compilation target.
        unsafe { neon::apply_i16_rows(accumulator, weights, adds, subs) }
        return;
    }
    #[cfg(all(feature = "simd", target_arch = "x86_64", target_feature = "avx2"))]
    {
        // SAFETY: AVX2 is enabled for this compilation target.
        unsafe { avx2::apply_i16_rows(accumulator, weights, adds, subs) }
        return;
    }
    #[allow(unreachable_code)]
    (kernels().apply_i16)(accumulator, weights, adds, subs);
}

/// Copy `src` into `dst` while applying feature-transformer row deltas in one pass.
#[inline]
pub(crate) fn apply_i16_rows_from(
    dst: &mut [i16; HIDDEN_SIZE],
    src: &[i16; HIDDEN_SIZE],
    weights: &[i16],
    adds: &[usize],
    subs: &[usize],
) {
    #[cfg(all(feature = "simd", target_arch = "aarch64", target_feature = "neon"))]
    {
        // SAFETY: NEON is enabled for this compilation target.
        unsafe { neon::apply_i16_rows_from(dst, src, weights, adds, subs) }
        return;
    }
    #[cfg(all(feature = "simd", target_arch = "x86_64", target_feature = "avx2"))]
    {
        // SAFETY: AVX2 is enabled for this compilation target.
        unsafe { avx2::apply_i16_rows_from(dst, src, weights, adds, subs) }
        return;
    }
    #[allow(unreachable_code)]
    scalar::apply_i16_rows_from(dst, src, weights, adds, subs);
}

#[inline]
pub(crate) fn apply_i8_rows(
    accumulator: &mut [i16; HIDDEN_SIZE],
    weights: &[u8],
    adds: &[usize],
    subs: &[usize],
) {
    #[cfg(all(feature = "simd", target_arch = "aarch64", target_feature = "neon"))]
    {
        // SAFETY: NEON is enabled for this compilation target.
        unsafe { neon::apply_i8_rows(accumulator, weights, adds, subs) }
        return;
    }
    #[cfg(all(feature = "simd", target_arch = "x86_64", target_feature = "avx2"))]
    {
        // SAFETY: AVX2 is enabled for this compilation target.
        unsafe { avx2::apply_i8_rows(accumulator, weights, adds, subs) }
        return;
    }
    #[allow(unreachable_code)]
    (kernels().apply_i8)(accumulator, weights, adds, subs);
}

/// Copy `src` into `dst` while applying signed i8 feature rows in one pass.
#[inline]
pub(crate) fn apply_i8_rows_from(
    dst: &mut [i16; HIDDEN_SIZE],
    src: &[i16; HIDDEN_SIZE],
    weights: &[u8],
    adds: &[usize],
    subs: &[usize],
) {
    #[cfg(all(feature = "simd", target_arch = "aarch64", target_feature = "neon"))]
    {
        // SAFETY: NEON is enabled for this compilation target.
        unsafe { neon::apply_i8_rows_from(dst, src, weights, adds, subs) }
        return;
    }
    #[cfg(all(feature = "simd", target_arch = "x86_64", target_feature = "avx2"))]
    {
        // SAFETY: AVX2 is enabled for this compilation target.
        unsafe { avx2::apply_i8_rows_from(dst, src, weights, adds, subs) }
        return;
    }
    #[allow(unreachable_code)]
    scalar::apply_i8_rows_from(dst, src, weights, adds, subs);
}

#[inline]
pub(crate) fn forward(
    piece: [&[i16; HIDDEN_SIZE]; 2],
    threat: [&[i16; HIDDEN_SIZE]; 2],
    stm: usize,
    weights: ForwardWeights<'_>,
) -> f32 {
    #[cfg(all(feature = "simd", target_arch = "aarch64", target_feature = "dotprod"))]
    {
        // SAFETY: DotProd is enabled for this compilation target.
        return unsafe { neon::forward_dotprod(piece, threat, stm, weights) };
    }
    #[cfg(all(
        feature = "simd",
        target_arch = "x86_64",
        target_feature = "avx2",
        target_feature = "fma"
    ))]
    {
        // SAFETY: AVX2 and FMA are enabled for this compilation target.
        return unsafe { avx2::forward(piece, threat, stm, weights) };
    }
    #[allow(unreachable_code)]
    (kernels().forward)(piece, threat, stm, weights)
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn apply_i16_avx2(
    accumulator: &mut [i16; HIDDEN_SIZE],
    weights: &[i16],
    adds: &[usize],
    subs: &[usize],
) {
    // SAFETY: this wrapper is installed only after AVX2 runtime detection.
    unsafe { avx2::apply_i16_rows(accumulator, weights, adds, subs) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn apply_i8_avx2(
    accumulator: &mut [i16; HIDDEN_SIZE],
    weights: &[u8],
    adds: &[usize],
    subs: &[usize],
) {
    // SAFETY: this wrapper is installed only after AVX2 runtime detection.
    unsafe { avx2::apply_i8_rows(accumulator, weights, adds, subs) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn forward_avx2(
    piece: [&[i16; HIDDEN_SIZE]; 2],
    threat: [&[i16; HIDDEN_SIZE]; 2],
    stm: usize,
    weights: ForwardWeights<'_>,
) -> f32 {
    // SAFETY: this wrapper is installed only after AVX2 and FMA detection.
    unsafe { avx2::forward(piece, threat, stm, weights) }
}

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
fn apply_i16_neon(
    accumulator: &mut [i16; HIDDEN_SIZE],
    weights: &[i16],
    adds: &[usize],
    subs: &[usize],
) {
    // SAFETY: NEON is part of the AArch64 baseline ISA.
    unsafe { neon::apply_i16_rows(accumulator, weights, adds, subs) }
}

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
fn apply_i8_neon(
    accumulator: &mut [i16; HIDDEN_SIZE],
    weights: &[u8],
    adds: &[usize],
    subs: &[usize],
) {
    // SAFETY: NEON is part of the AArch64 baseline ISA.
    unsafe { neon::apply_i8_rows(accumulator, weights, adds, subs) }
}

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
fn forward_neon(
    piece: [&[i16; HIDDEN_SIZE]; 2],
    threat: [&[i16; HIDDEN_SIZE]; 2],
    stm: usize,
    weights: ForwardWeights<'_>,
) -> f32 {
    // SAFETY: NEON is part of the AArch64 baseline ISA.
    unsafe { neon::forward(piece, threat, stm, weights) }
}

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
fn forward_neon_dotprod(
    piece: [&[i16; HIDDEN_SIZE]; 2],
    threat: [&[i16; HIDDEN_SIZE]; 2],
    stm: usize,
    weights: ForwardWeights<'_>,
) -> f32 {
    // SAFETY: this wrapper is installed only after DotProd runtime detection.
    unsafe { neon::forward_dotprod(piece, threat, stm, weights) }
}

mod scalar {
    use super::*;

    pub(super) fn apply_i16_rows(
        accumulator: &mut [i16; HIDDEN_SIZE],
        weights: &[i16],
        adds: &[usize],
        subs: &[usize],
    ) {
        const CHUNK: usize = 16;
        for start in (0..HIDDEN_SIZE).step_by(CHUNK) {
            let mut values = [0i16; CHUNK];
            values.copy_from_slice(&accumulator[start..start + CHUNK]);
            for &index in adds {
                let row = index * HIDDEN_SIZE + start;
                for lane in 0..CHUNK {
                    values[lane] = values[lane].wrapping_add(weights[row + lane]);
                }
            }
            for &index in subs {
                let row = index * HIDDEN_SIZE + start;
                for lane in 0..CHUNK {
                    values[lane] = values[lane].wrapping_sub(weights[row + lane]);
                }
            }
            accumulator[start..start + CHUNK].copy_from_slice(&values);
        }
    }

    pub(super) fn apply_i16_rows_from(
        dst: &mut [i16; HIDDEN_SIZE],
        src: &[i16; HIDDEN_SIZE],
        weights: &[i16],
        adds: &[usize],
        subs: &[usize],
    ) {
        const CHUNK: usize = 16;
        for start in (0..HIDDEN_SIZE).step_by(CHUNK) {
            let mut values = [0i16; CHUNK];
            values.copy_from_slice(&src[start..start + CHUNK]);
            for &index in adds {
                let row = index * HIDDEN_SIZE + start;
                for lane in 0..CHUNK {
                    values[lane] = values[lane].wrapping_add(weights[row + lane]);
                }
            }
            for &index in subs {
                let row = index * HIDDEN_SIZE + start;
                for lane in 0..CHUNK {
                    values[lane] = values[lane].wrapping_sub(weights[row + lane]);
                }
            }
            dst[start..start + CHUNK].copy_from_slice(&values);
        }
    }

    pub(super) fn apply_i8_rows(
        accumulator: &mut [i16; HIDDEN_SIZE],
        weights: &[u8],
        adds: &[usize],
        subs: &[usize],
    ) {
        const CHUNK: usize = 16;
        for start in (0..HIDDEN_SIZE).step_by(CHUNK) {
            let mut values = [0i16; CHUNK];
            values.copy_from_slice(&accumulator[start..start + CHUNK]);
            for &index in adds {
                let row = index * HIDDEN_SIZE + start;
                for lane in 0..CHUNK {
                    values[lane] = values[lane].wrapping_add(weights[row + lane] as i8 as i16);
                }
            }
            for &index in subs {
                let row = index * HIDDEN_SIZE + start;
                for lane in 0..CHUNK {
                    values[lane] = values[lane].wrapping_sub(weights[row + lane] as i8 as i16);
                }
            }
            accumulator[start..start + CHUNK].copy_from_slice(&values);
        }
    }

    pub(super) fn apply_i8_rows_from(
        dst: &mut [i16; HIDDEN_SIZE],
        src: &[i16; HIDDEN_SIZE],
        weights: &[u8],
        adds: &[usize],
        subs: &[usize],
    ) {
        const CHUNK: usize = 16;
        for start in (0..HIDDEN_SIZE).step_by(CHUNK) {
            let mut values = [0i16; CHUNK];
            values.copy_from_slice(&src[start..start + CHUNK]);
            for &index in adds {
                let row = index * HIDDEN_SIZE + start;
                for lane in 0..CHUNK {
                    values[lane] = values[lane].wrapping_add(weights[row + lane] as i8 as i16);
                }
            }
            for &index in subs {
                let row = index * HIDDEN_SIZE + start;
                for lane in 0..CHUNK {
                    values[lane] = values[lane].wrapping_sub(weights[row + lane] as i8 as i16);
                }
            }
            dst[start..start + CHUNK].copy_from_slice(&values);
        }
    }

    pub(super) fn forward(
        piece: [&[i16; HIDDEN_SIZE]; 2],
        threat: [&[i16; HIDDEN_SIZE]; 2],
        stm: usize,
        weights: ForwardWeights<'_>,
    ) -> f32 {
        let mut transformed = [0u8; HIDDEN_SIZE];
        for flip in 0..2 {
            let perspective = stm ^ flip;
            for index in 0..HIDDEN_SIZE / 2 {
                let left = piece[perspective][index]
                    .wrapping_add(threat[perspective][index])
                    .clamp(0, FT_QUANT) as i32;
                let right = piece[perspective][index + HIDDEN_SIZE / 2]
                    .wrapping_add(threat[perspective][index + HIDDEN_SIZE / 2])
                    .clamp(0, FT_QUANT) as i32;
                transformed[index + flip * HIDDEN_SIZE / 2] = ((left * right) >> FT_SHIFT) as u8;
            }
        }

        let mut l1 = [0f32; L2_SIZE];
        for group in 0..HIDDEN_SIZE / 4 {
            let input = &transformed[group * 4..group * 4 + 4];
            if input == [0, 0, 0, 0] {
                continue;
            }
            let base = group * L2_SIZE * 4;
            for (output, output_value) in l1.iter_mut().enumerate() {
                let row = base + output * 4;
                for (lane, &input_value) in input.iter().enumerate() {
                    *output_value +=
                        f32::from(input_value) * f32::from(weights.l1[row + lane] as i8);
                }
            }
        }
        const DEQUANT: f32 = (1u32 << FT_SHIFT) as f32 / (255 * 255 * 64) as f32;
        for (index, value) in l1.iter_mut().enumerate() {
            *value = (*value * DEQUANT + weights.l1_biases[index]).clamp(0.0, 1.0);
        }

        let mut l2 = [0f32; L3_SIZE];
        for (input, &input_value) in l1.iter().enumerate() {
            for (output, output_value) in l2.iter_mut().enumerate() {
                *output_value += weights.l2[input * L3_SIZE + output] * input_value;
            }
        }
        for (index, value) in l2.iter_mut().enumerate() {
            *value = (*value + weights.l2_biases[index]).clamp(0.0, 1.0);
        }

        let mut output = 0.0;
        for (index, value) in l2.iter().enumerate() {
            output = weights.l3[index].mul_add(*value, output);
        }
        output + weights.l3_bias
    }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
mod avx2 {
    #![allow(clippy::undocumented_unsafe_blocks)]

    use std::arch::x86_64::*;

    use super::*;

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn apply_i16_rows(
        accumulator: &mut [i16; HIDDEN_SIZE],
        weights: &[i16],
        adds: &[usize],
        subs: &[usize],
    ) {
        unsafe {
            const REGISTERS: usize = 8;
            const BLOCK: usize = REGISTERS * 16;
            for index in (0..HIDDEN_SIZE).step_by(BLOCK) {
                let mut values = [_mm256_setzero_si256(); REGISTERS];
                for (register, value) in values.iter_mut().enumerate() {
                    *value =
                        _mm256_loadu_si256(accumulator.as_ptr().add(index + register * 16).cast());
                }

                let paired = adds.len().min(subs.len());
                for pair in 0..paired {
                    let add = weights.as_ptr().add(adds[pair] * HIDDEN_SIZE + index);
                    let sub = weights.as_ptr().add(subs[pair] * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        *value = _mm256_add_epi16(
                            *value,
                            _mm256_sub_epi16(
                                _mm256_loadu_si256(add.add(register * 16).cast()),
                                _mm256_loadu_si256(sub.add(register * 16).cast()),
                            ),
                        );
                    }
                }
                for &row in &adds[paired..] {
                    let add = weights.as_ptr().add(row * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        *value = _mm256_add_epi16(
                            *value,
                            _mm256_loadu_si256(add.add(register * 16).cast()),
                        );
                    }
                }
                for &row in &subs[paired..] {
                    let sub = weights.as_ptr().add(row * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        *value = _mm256_sub_epi16(
                            *value,
                            _mm256_loadu_si256(sub.add(register * 16).cast()),
                        );
                    }
                }
                for (register, value) in values.iter().enumerate() {
                    _mm256_storeu_si256(
                        accumulator.as_mut_ptr().add(index + register * 16).cast(),
                        *value,
                    );
                }
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn apply_i16_rows_from(
        dst: &mut [i16; HIDDEN_SIZE],
        src: &[i16; HIDDEN_SIZE],
        weights: &[i16],
        adds: &[usize],
        subs: &[usize],
    ) {
        unsafe {
            const REGISTERS: usize = 8;
            const BLOCK: usize = REGISTERS * 16;
            for index in (0..HIDDEN_SIZE).step_by(BLOCK) {
                let mut values = [_mm256_setzero_si256(); REGISTERS];
                for (register, value) in values.iter_mut().enumerate() {
                    *value = _mm256_loadu_si256(src.as_ptr().add(index + register * 16).cast());
                }

                let paired = adds.len().min(subs.len());
                for pair in 0..paired {
                    let add = weights.as_ptr().add(adds[pair] * HIDDEN_SIZE + index);
                    let sub = weights.as_ptr().add(subs[pair] * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        *value = _mm256_add_epi16(
                            *value,
                            _mm256_sub_epi16(
                                _mm256_loadu_si256(add.add(register * 16).cast()),
                                _mm256_loadu_si256(sub.add(register * 16).cast()),
                            ),
                        );
                    }
                }
                for &row in &adds[paired..] {
                    let add = weights.as_ptr().add(row * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        *value = _mm256_add_epi16(
                            *value,
                            _mm256_loadu_si256(add.add(register * 16).cast()),
                        );
                    }
                }
                for &row in &subs[paired..] {
                    let sub = weights.as_ptr().add(row * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        *value = _mm256_sub_epi16(
                            *value,
                            _mm256_loadu_si256(sub.add(register * 16).cast()),
                        );
                    }
                }
                for (register, value) in values.iter().enumerate() {
                    _mm256_storeu_si256(dst.as_mut_ptr().add(index + register * 16).cast(), *value);
                }
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn apply_i8_rows(
        accumulator: &mut [i16; HIDDEN_SIZE],
        weights: &[u8],
        adds: &[usize],
        subs: &[usize],
    ) {
        unsafe {
            const REGISTERS: usize = 8;
            const BLOCK: usize = REGISTERS * 16;
            for index in (0..HIDDEN_SIZE).step_by(BLOCK) {
                let mut values = [_mm256_setzero_si256(); REGISTERS];
                for (register, value) in values.iter_mut().enumerate() {
                    *value =
                        _mm256_loadu_si256(accumulator.as_ptr().add(index + register * 16).cast());
                }

                let paired = adds.len().min(subs.len());
                for pair in 0..paired {
                    let add = weights.as_ptr().add(adds[pair] * HIDDEN_SIZE + index);
                    let sub = weights.as_ptr().add(subs[pair] * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        let offset = register * 16;
                        let add = _mm256_cvtepi8_epi16(_mm_loadu_si128(add.add(offset).cast()));
                        let sub = _mm256_cvtepi8_epi16(_mm_loadu_si128(sub.add(offset).cast()));
                        *value = _mm256_add_epi16(*value, _mm256_sub_epi16(add, sub));
                    }
                }
                for &row in &adds[paired..] {
                    let add = weights.as_ptr().add(row * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        let bytes = _mm_loadu_si128(add.add(register * 16).cast());
                        *value = _mm256_add_epi16(*value, _mm256_cvtepi8_epi16(bytes));
                    }
                }
                for &row in &subs[paired..] {
                    let sub = weights.as_ptr().add(row * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        let bytes = _mm_loadu_si128(sub.add(register * 16).cast());
                        *value = _mm256_sub_epi16(*value, _mm256_cvtepi8_epi16(bytes));
                    }
                }
                for (register, value) in values.iter().enumerate() {
                    _mm256_storeu_si256(
                        accumulator.as_mut_ptr().add(index + register * 16).cast(),
                        *value,
                    );
                }
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn apply_i8_rows_from(
        dst: &mut [i16; HIDDEN_SIZE],
        src: &[i16; HIDDEN_SIZE],
        weights: &[u8],
        adds: &[usize],
        subs: &[usize],
    ) {
        unsafe {
            const REGISTERS: usize = 8;
            const BLOCK: usize = REGISTERS * 16;
            for index in (0..HIDDEN_SIZE).step_by(BLOCK) {
                let mut values = [_mm256_setzero_si256(); REGISTERS];
                for (register, value) in values.iter_mut().enumerate() {
                    *value = _mm256_loadu_si256(src.as_ptr().add(index + register * 16).cast());
                }

                let paired = adds.len().min(subs.len());
                for pair in 0..paired {
                    let add = weights.as_ptr().add(adds[pair] * HIDDEN_SIZE + index);
                    let sub = weights.as_ptr().add(subs[pair] * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        let offset = register * 16;
                        let add = _mm256_cvtepi8_epi16(_mm_loadu_si128(add.add(offset).cast()));
                        let sub = _mm256_cvtepi8_epi16(_mm_loadu_si128(sub.add(offset).cast()));
                        *value = _mm256_add_epi16(*value, _mm256_sub_epi16(add, sub));
                    }
                }
                for &row in &adds[paired..] {
                    let add = weights.as_ptr().add(row * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        let bytes = _mm_loadu_si128(add.add(register * 16).cast());
                        *value = _mm256_add_epi16(*value, _mm256_cvtepi8_epi16(bytes));
                    }
                }
                for &row in &subs[paired..] {
                    let sub = weights.as_ptr().add(row * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        let bytes = _mm_loadu_si128(sub.add(register * 16).cast());
                        *value = _mm256_sub_epi16(*value, _mm256_cvtepi8_epi16(bytes));
                    }
                }
                for (register, value) in values.iter().enumerate() {
                    _mm256_storeu_si256(dst.as_mut_ptr().add(index + register * 16).cast(), *value);
                }
            }
        }
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn activate(
        piece: [&[i16; HIDDEN_SIZE]; 2],
        threat: [&[i16; HIDDEN_SIZE]; 2],
        stm: usize,
    ) -> [u8; HIDDEN_SIZE] {
        unsafe {
            let mut output = [0u8; HIDDEN_SIZE];
            let zero = _mm256_setzero_si256();
            let max = _mm256_set1_epi16(FT_QUANT);

            for flip in 0..2 {
                let perspective = stm ^ flip;
                let dst = flip * HIDDEN_SIZE / 2;
                for index in (0..HIDDEN_SIZE / 2).step_by(32) {
                    let left0 = _mm256_add_epi16(
                        _mm256_loadu_si256(piece[perspective].as_ptr().add(index).cast()),
                        _mm256_loadu_si256(threat[perspective].as_ptr().add(index).cast()),
                    );
                    let left1 = _mm256_add_epi16(
                        _mm256_loadu_si256(piece[perspective].as_ptr().add(index + 16).cast()),
                        _mm256_loadu_si256(threat[perspective].as_ptr().add(index + 16).cast()),
                    );
                    let right0 = _mm256_add_epi16(
                        _mm256_loadu_si256(
                            piece[perspective]
                                .as_ptr()
                                .add(index + HIDDEN_SIZE / 2)
                                .cast(),
                        ),
                        _mm256_loadu_si256(
                            threat[perspective]
                                .as_ptr()
                                .add(index + HIDDEN_SIZE / 2)
                                .cast(),
                        ),
                    );
                    let right1 = _mm256_add_epi16(
                        _mm256_loadu_si256(
                            piece[perspective]
                                .as_ptr()
                                .add(index + HIDDEN_SIZE / 2 + 16)
                                .cast(),
                        ),
                        _mm256_loadu_si256(
                            threat[perspective]
                                .as_ptr()
                                .add(index + HIDDEN_SIZE / 2 + 16)
                                .cast(),
                        ),
                    );

                    let left0 = _mm256_max_epi16(_mm256_min_epi16(left0, max), zero);
                    let left1 = _mm256_max_epi16(_mm256_min_epi16(left1, max), zero);
                    let right0 = _mm256_min_epi16(right0, max);
                    let right1 = _mm256_min_epi16(right1, max);
                    let product0 = _mm256_mulhi_epi16(_mm256_slli_epi16::<7>(left0), right0);
                    let product1 = _mm256_mulhi_epi16(_mm256_slli_epi16::<7>(left1), right1);
                    let packed = _mm256_permute4x64_epi64::<0b11_01_10_00>(_mm256_packus_epi16(
                        product0, product1,
                    ));
                    _mm256_storeu_si256(output.as_mut_ptr().add(dst + index).cast(), packed);
                }
            }
            output
        }
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn dot_bytes(input: __m256i, weights: __m256i) -> __m256i {
        let pairwise = _mm256_maddubs_epi16(input, weights);
        _mm256_madd_epi16(pairwise, _mm256_set1_epi16(1))
    }

    #[repr(C, align(16))]
    #[derive(Clone, Copy)]
    struct SparseEntry {
        indexes: [u16; 8],
        count: usize,
    }

    const fn build_sparse_table() -> [SparseEntry; 256] {
        let mut table = [SparseEntry {
            indexes: [0; 8],
            count: 0,
        }; 256];
        let mut mask = 0;
        while mask < table.len() {
            let mut bit = 0;
            let mut count = 0;
            while bit < 8 {
                if mask & (1 << bit) != 0 {
                    table[mask].indexes[count] = bit as u16;
                    count += 1;
                }
                bit += 1;
            }
            table[mask].count = count;
            mask += 1;
        }
        table
    }

    static SPARSE_TABLE: [SparseEntry; 256] = build_sparse_table();

    #[inline(always)]
    unsafe fn nonzero_mask(values: __m256i) -> usize {
        unsafe {
            let zero = _mm256_setzero_si256();
            let eq = _mm256_cmpeq_epi32(values, zero);
            // Movemask sets a bit per lane that is all-ones (== zero). Invert for nonzero.
            let zero_bits = _mm256_movemask_ps(_mm256_castsi256_ps(eq)) as u32;
            (!zero_bits & 0xFF) as usize
        }
    }

    #[inline(always)]
    unsafe fn collect_nonzero_groups(
        transformed: &[u8; HIDDEN_SIZE],
        indexes: &mut [u16; HIDDEN_SIZE / 4],
    ) -> usize {
        unsafe {
            let mut count = 0;
            let mut base: u16 = 0;
            for offset in (0..HIDDEN_SIZE).step_by(32) {
                let values = _mm256_loadu_si256(transformed.as_ptr().add(offset).cast());
                let mask = nonzero_mask(values);
                let entry = &SPARSE_TABLE[mask];
                for index in 0..entry.count {
                    *indexes.get_unchecked_mut(count + index) =
                        base + *entry.indexes.get_unchecked(index);
                }
                count += entry.count;
                base = base.wrapping_add(8);
            }
            count
        }
    }

    #[target_feature(enable = "avx2")]
    unsafe fn horizontal_sum(value: __m256) -> f32 {
        let high = _mm256_extractf128_ps::<1>(value);
        let low = _mm256_castps256_ps128(value);
        let sum = _mm_add_ps(low, high);
        let sum = _mm_hadd_ps(sum, sum);
        let sum = _mm_hadd_ps(sum, sum);
        _mm_cvtss_f32(sum)
    }

    #[target_feature(enable = "avx2,fma")]
    pub(super) unsafe fn forward(
        piece: [&[i16; HIDDEN_SIZE]; 2],
        threat: [&[i16; HIDDEN_SIZE]; 2],
        stm: usize,
        weights: ForwardWeights<'_>,
    ) -> f32 {
        unsafe {
            let transformed = activate(piece, threat, stm);
            let packed =
                std::slice::from_raw_parts(transformed.as_ptr().cast::<i32>(), HIDDEN_SIZE / 4);
            let mut indexes = [0u16; HIDDEN_SIZE / 4];
            let count = collect_nonzero_groups(&transformed, &mut indexes);

            let mut sums = [_mm256_setzero_si256(); L2_SIZE / 8];
            let mut pairs = indexes[..count].chunks_exact(2);
            for pair in &mut pairs {
                let first = pair[0] as usize;
                let second = pair[1] as usize;
                let first_input = _mm256_set1_epi32(packed[first]);
                let second_input = _mm256_set1_epi32(packed[second]);
                let first_base = weights.l1.as_ptr().add(first * L2_SIZE * 4);
                let second_base = weights.l1.as_ptr().add(second * L2_SIZE * 4);
                for output in (0..L2_SIZE).step_by(8) {
                    let first_row = _mm256_loadu_si256(first_base.add(output * 4).cast());
                    let second_row = _mm256_loadu_si256(second_base.add(output * 4).cast());
                    let lane = &mut sums[output / 8];
                    *lane = _mm256_add_epi32(*lane, dot_bytes(first_input, first_row));
                    *lane = _mm256_add_epi32(*lane, dot_bytes(second_input, second_row));
                }
            }
            if let Some(&group) = pairs.remainder().first() {
                let group = group as usize;
                let input = _mm256_set1_epi32(packed[group]);
                let base = weights.l1.as_ptr().add(group * L2_SIZE * 4);
                for output in (0..L2_SIZE).step_by(8) {
                    let row = _mm256_loadu_si256(base.add(output * 4).cast());
                    sums[output / 8] = _mm256_add_epi32(sums[output / 8], dot_bytes(input, row));
                }
            }

            const DEQUANT: f32 = (1u32 << FT_SHIFT) as f32 / (255 * 255 * 64) as f32;
            let mut l1 = [0.0f32; L2_SIZE];
            let zero = _mm256_setzero_ps();
            let one = _mm256_set1_ps(1.0);
            let dequant = _mm256_set1_ps(DEQUANT);
            for index in (0..L2_SIZE).step_by(8) {
                let bias = _mm256_loadu_ps(weights.l1_biases.as_ptr().add(index));
                let value = _mm256_fmadd_ps(_mm256_cvtepi32_ps(sums[index / 8]), dequant, bias);
                _mm256_storeu_ps(
                    l1.as_mut_ptr().add(index),
                    _mm256_max_ps(_mm256_min_ps(value, one), zero),
                );
            }

            let mut l2 = [_mm256_setzero_ps(); L3_SIZE / 8];
            for (input, &activation) in l1.iter().enumerate() {
                let value = _mm256_set1_ps(activation);
                let row = weights.l2.as_ptr().add(input * L3_SIZE);
                for output in (0..L3_SIZE).step_by(8) {
                    let weight = _mm256_loadu_ps(row.add(output));
                    l2[output / 8] = _mm256_fmadd_ps(weight, value, l2[output / 8]);
                }
            }
            for index in (0..L3_SIZE).step_by(8) {
                let value = _mm256_add_ps(
                    l2[index / 8],
                    _mm256_loadu_ps(weights.l2_biases.as_ptr().add(index)),
                );
                l2[index / 8] = _mm256_max_ps(_mm256_min_ps(value, one), zero);
            }

            let mut total = _mm256_setzero_ps();
            for index in (0..L3_SIZE).step_by(8) {
                total = _mm256_fmadd_ps(
                    _mm256_loadu_ps(weights.l3.as_ptr().add(index)),
                    l2[index / 8],
                    total,
                );
            }
            horizontal_sum(total) + weights.l3_bias
        }
    }
}

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
mod neon {
    #![allow(clippy::undocumented_unsafe_blocks)]

    use std::arch::aarch64::*;

    use super::*;

    #[repr(C, align(16))]
    #[derive(Clone, Copy)]
    struct SparseEntry {
        indexes: [u16; 8],
        count: usize,
    }

    const fn build_sparse_table() -> [SparseEntry; 256] {
        let mut table = [SparseEntry {
            indexes: [0; 8],
            count: 0,
        }; 256];
        let mut mask = 0;
        while mask < table.len() {
            let mut bit = 0;
            let mut count = 0;
            while bit < 8 {
                if mask & (1 << bit) != 0 {
                    table[mask].indexes[count] = bit as u16;
                    count += 1;
                }
                bit += 1;
            }
            table[mask].count = count;
            mask += 1;
        }
        table
    }

    static SPARSE_TABLE: [SparseEntry; 256] = build_sparse_table();

    pub(super) unsafe fn apply_i16_rows(
        accumulator: &mut [i16; HIDDEN_SIZE],
        weights: &[i16],
        adds: &[usize],
        subs: &[usize],
    ) {
        unsafe {
            const REGISTERS: usize = 16;
            const BLOCK: usize = REGISTERS * 8;
            for index in (0..HIDDEN_SIZE).step_by(BLOCK) {
                let mut values = [vdupq_n_s16(0); REGISTERS];
                for (register, value) in values.iter_mut().enumerate() {
                    *value = vld1q_s16(accumulator.as_ptr().add(index + register * 8));
                }

                let paired = adds.len().min(subs.len());
                for pair in 0..paired {
                    let add = weights.as_ptr().add(adds[pair] * HIDDEN_SIZE + index);
                    let sub = weights.as_ptr().add(subs[pair] * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        *value = vaddq_s16(
                            *value,
                            vsubq_s16(
                                vld1q_s16(add.add(register * 8)),
                                vld1q_s16(sub.add(register * 8)),
                            ),
                        );
                    }
                }
                for &row in &adds[paired..] {
                    let add = weights.as_ptr().add(row * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        *value = vaddq_s16(*value, vld1q_s16(add.add(register * 8)));
                    }
                }
                for &row in &subs[paired..] {
                    let sub = weights.as_ptr().add(row * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        *value = vsubq_s16(*value, vld1q_s16(sub.add(register * 8)));
                    }
                }
                for (register, value) in values.iter().enumerate() {
                    vst1q_s16(accumulator.as_mut_ptr().add(index + register * 8), *value);
                }
            }
        }
    }

    pub(super) unsafe fn apply_i16_rows_from(
        dst: &mut [i16; HIDDEN_SIZE],
        src: &[i16; HIDDEN_SIZE],
        weights: &[i16],
        adds: &[usize],
        subs: &[usize],
    ) {
        unsafe {
            const REGISTERS: usize = 16;
            const BLOCK: usize = REGISTERS * 8;
            for index in (0..HIDDEN_SIZE).step_by(BLOCK) {
                let mut values = [vdupq_n_s16(0); REGISTERS];
                for (register, value) in values.iter_mut().enumerate() {
                    *value = vld1q_s16(src.as_ptr().add(index + register * 8));
                }

                let paired = adds.len().min(subs.len());
                for pair in 0..paired {
                    let add = weights.as_ptr().add(adds[pair] * HIDDEN_SIZE + index);
                    let sub = weights.as_ptr().add(subs[pair] * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        *value = vaddq_s16(
                            *value,
                            vsubq_s16(
                                vld1q_s16(add.add(register * 8)),
                                vld1q_s16(sub.add(register * 8)),
                            ),
                        );
                    }
                }
                for &row in &adds[paired..] {
                    let add = weights.as_ptr().add(row * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        *value = vaddq_s16(*value, vld1q_s16(add.add(register * 8)));
                    }
                }
                for &row in &subs[paired..] {
                    let sub = weights.as_ptr().add(row * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        *value = vsubq_s16(*value, vld1q_s16(sub.add(register * 8)));
                    }
                }
                for (register, value) in values.iter().enumerate() {
                    vst1q_s16(dst.as_mut_ptr().add(index + register * 8), *value);
                }
            }
        }
    }

    pub(super) unsafe fn apply_i8_rows(
        accumulator: &mut [i16; HIDDEN_SIZE],
        weights: &[u8],
        adds: &[usize],
        subs: &[usize],
    ) {
        unsafe {
            const REGISTERS: usize = 16;
            const BLOCK: usize = REGISTERS * 8;
            for index in (0..HIDDEN_SIZE).step_by(BLOCK) {
                let mut values = [vdupq_n_s16(0); REGISTERS];
                for (register, value) in values.iter_mut().enumerate() {
                    *value = vld1q_s16(accumulator.as_ptr().add(index + register * 8));
                }

                let paired = adds.len().min(subs.len());
                for pair in 0..paired {
                    let add = weights.as_ptr().add(adds[pair] * HIDDEN_SIZE + index);
                    let sub = weights.as_ptr().add(subs[pair] * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        let offset = register * 8;
                        let add = vmovl_s8(vld1_s8(add.add(offset).cast()));
                        let sub = vmovl_s8(vld1_s8(sub.add(offset).cast()));
                        *value = vaddq_s16(*value, vsubq_s16(add, sub));
                    }
                }
                for &row in &adds[paired..] {
                    let add = weights.as_ptr().add(row * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        *value = vaddq_s16(*value, vmovl_s8(vld1_s8(add.add(register * 8).cast())));
                    }
                }
                for &row in &subs[paired..] {
                    let sub = weights.as_ptr().add(row * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        *value = vsubq_s16(*value, vmovl_s8(vld1_s8(sub.add(register * 8).cast())));
                    }
                }
                for (register, value) in values.iter().enumerate() {
                    vst1q_s16(accumulator.as_mut_ptr().add(index + register * 8), *value);
                }
            }
        }
    }

    pub(super) unsafe fn apply_i8_rows_from(
        dst: &mut [i16; HIDDEN_SIZE],
        src: &[i16; HIDDEN_SIZE],
        weights: &[u8],
        adds: &[usize],
        subs: &[usize],
    ) {
        unsafe {
            const REGISTERS: usize = 16;
            const BLOCK: usize = REGISTERS * 8;
            for index in (0..HIDDEN_SIZE).step_by(BLOCK) {
                let mut values = [vdupq_n_s16(0); REGISTERS];
                for (register, value) in values.iter_mut().enumerate() {
                    *value = vld1q_s16(src.as_ptr().add(index + register * 8));
                }

                let paired = adds.len().min(subs.len());
                for pair in 0..paired {
                    let add = weights.as_ptr().add(adds[pair] * HIDDEN_SIZE + index);
                    let sub = weights.as_ptr().add(subs[pair] * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        let offset = register * 8;
                        let add = vmovl_s8(vld1_s8(add.add(offset).cast()));
                        let sub = vmovl_s8(vld1_s8(sub.add(offset).cast()));
                        *value = vaddq_s16(*value, vsubq_s16(add, sub));
                    }
                }
                for &row in &adds[paired..] {
                    let add = weights.as_ptr().add(row * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        *value = vaddq_s16(*value, vmovl_s8(vld1_s8(add.add(register * 8).cast())));
                    }
                }
                for &row in &subs[paired..] {
                    let sub = weights.as_ptr().add(row * HIDDEN_SIZE + index);
                    for (register, value) in values.iter_mut().enumerate() {
                        *value = vsubq_s16(*value, vmovl_s8(vld1_s8(sub.add(register * 8).cast())));
                    }
                }
                for (register, value) in values.iter().enumerate() {
                    vst1q_s16(dst.as_mut_ptr().add(index + register * 8), *value);
                }
            }
        }
    }

    #[inline(always)]
    unsafe fn activate(
        piece: [&[i16; HIDDEN_SIZE]; 2],
        threat: [&[i16; HIDDEN_SIZE]; 2],
        stm: usize,
    ) -> [u8; HIDDEN_SIZE] {
        unsafe {
            let mut output = [0u8; HIDDEN_SIZE];
            let zero = vdupq_n_s16(0);
            let max = vdupq_n_s16(FT_QUANT);

            for flip in 0..2 {
                let perspective = stm ^ flip;
                let dst = flip * HIDDEN_SIZE / 2;
                for index in (0..HIDDEN_SIZE / 2).step_by(16) {
                    let left0 = vaddq_s16(
                        vld1q_s16(piece[perspective].as_ptr().add(index)),
                        vld1q_s16(threat[perspective].as_ptr().add(index)),
                    );
                    let left1 = vaddq_s16(
                        vld1q_s16(piece[perspective].as_ptr().add(index + 8)),
                        vld1q_s16(threat[perspective].as_ptr().add(index + 8)),
                    );
                    let right0 = vaddq_s16(
                        vld1q_s16(piece[perspective].as_ptr().add(index + HIDDEN_SIZE / 2)),
                        vld1q_s16(threat[perspective].as_ptr().add(index + HIDDEN_SIZE / 2)),
                    );
                    let right1 = vaddq_s16(
                        vld1q_s16(piece[perspective].as_ptr().add(index + HIDDEN_SIZE / 2 + 8)),
                        vld1q_s16(
                            threat[perspective]
                                .as_ptr()
                                .add(index + HIDDEN_SIZE / 2 + 8),
                        ),
                    );
                    let left0 = vmaxq_s16(vminq_s16(left0, max), zero);
                    let left1 = vmaxq_s16(vminq_s16(left1, max), zero);
                    let right0 = vminq_s16(right0, max);
                    let right1 = vminq_s16(right1, max);
                    let product0 = vqdmulhq_s16(vshlq_n_s16::<6>(left0), right0);
                    let product1 = vqdmulhq_s16(vshlq_n_s16::<6>(left1), right1);
                    let packed = vcombine_u8(vqmovun_s16(product0), vqmovun_s16(product1));
                    vst1q_u8(output.as_mut_ptr().add(dst + index), packed);
                }
            }
            output
        }
    }

    #[inline(always)]
    unsafe fn dot_bytes<const DOTPROD: bool>(
        mut accumulator: int32x4_t,
        input: int32x4_t,
        weights: int8x16_t,
    ) -> int32x4_t {
        unsafe {
            if DOTPROD {
                std::arch::asm!(
                    "sdot {acc:v}.4s, {src1:v}.16b, {src2:v}.16b",
                    acc = inout(vreg) accumulator,
                    src1 = in(vreg) input,
                    src2 = in(vreg) weights,
                    options(pure, nomem, nostack)
                );
                return accumulator;
            }

            let input = vreinterpretq_u8_s32(input);
            let low = vmulq_s16(
                vreinterpretq_s16_u16(vmovl_u8(vget_low_u8(input))),
                vmovl_s8(vget_low_s8(weights)),
            );
            let high = vmulq_s16(
                vreinterpretq_s16_u16(vmovl_u8(vget_high_u8(input))),
                vmovl_s8(vget_high_s8(weights)),
            );
            vaddq_s32(accumulator, vpaddq_s32(vpaddlq_s16(low), vpaddlq_s16(high)))
        }
    }

    #[inline(always)]
    unsafe fn nonzero_mask(values: int32x4_t) -> usize {
        unsafe {
            let nonzero = vmvnq_u32(vceqq_s32(values, vdupq_n_s32(0)));
            let bits = [1u32, 2, 4, 8];
            vaddvq_u32(vandq_u32(nonzero, vld1q_u32(bits.as_ptr()))) as usize
        }
    }

    #[inline(always)]
    unsafe fn collect_nonzero_groups(
        transformed: &[u8; HIDDEN_SIZE],
        indexes: &mut [u16; HIDDEN_SIZE / 4],
    ) -> usize {
        unsafe {
            let mut count = 0;
            let mut base = vdupq_n_u16(0);
            let increment = vdupq_n_u16(8);
            for offset in (0..HIDDEN_SIZE).step_by(32) {
                let first = vld1q_s32(transformed.as_ptr().add(offset).cast());
                let second = vld1q_s32(transformed.as_ptr().add(offset + 16).cast());
                let mask = nonzero_mask(first) | (nonzero_mask(second) << 4);
                let entry = &SPARSE_TABLE[mask];
                let compacted = vaddq_u16(base, vld1q_u16(entry.indexes.as_ptr()));
                vst1q_u16(indexes.as_mut_ptr().add(count), compacted);
                count += entry.count;
                base = vaddq_u16(base, increment);
            }
            count
        }
    }

    #[inline(always)]
    unsafe fn forward_impl<const DOTPROD: bool>(
        piece: [&[i16; HIDDEN_SIZE]; 2],
        threat: [&[i16; HIDDEN_SIZE]; 2],
        stm: usize,
        weights: ForwardWeights<'_>,
    ) -> f32 {
        unsafe {
            let transformed = activate(piece, threat, stm);
            let packed =
                std::slice::from_raw_parts(transformed.as_ptr().cast::<i32>(), HIDDEN_SIZE / 4);
            let mut indexes = [0u16; HIDDEN_SIZE / 4];
            let count = collect_nonzero_groups(&transformed, &mut indexes);

            let mut sums = [vdupq_n_s32(0); L2_SIZE / 4];
            let mut pairs = indexes[..count].chunks_exact(2);
            for pair in &mut pairs {
                let first = pair[0] as usize;
                let second = pair[1] as usize;
                let first_input = vdupq_n_s32(packed[first]);
                let second_input = vdupq_n_s32(packed[second]);
                let first_weights = weights.l1.as_ptr().add(first * L2_SIZE * 4);
                let second_weights = weights.l1.as_ptr().add(second * L2_SIZE * 4);
                for output in (0..L2_SIZE).step_by(4) {
                    let sum = dot_bytes::<DOTPROD>(
                        sums[output / 4],
                        first_input,
                        vld1q_s8(first_weights.add(output * 4).cast()),
                    );
                    sums[output / 4] = dot_bytes::<DOTPROD>(
                        sum,
                        second_input,
                        vld1q_s8(second_weights.add(output * 4).cast()),
                    );
                }
            }
            if let Some(&group) = pairs.remainder().first() {
                let group = group as usize;
                let input = vdupq_n_s32(packed[group]);
                let group_weights = weights.l1.as_ptr().add(group * L2_SIZE * 4);
                for output in (0..L2_SIZE).step_by(4) {
                    sums[output / 4] = dot_bytes::<DOTPROD>(
                        sums[output / 4],
                        input,
                        vld1q_s8(group_weights.add(output * 4).cast()),
                    );
                }
            }

            const DEQUANT: f32 = (1u32 << FT_SHIFT) as f32 / (255 * 255 * 64) as f32;
            let mut l1 = [0.0f32; L2_SIZE];
            let zero = vdupq_n_f32(0.0);
            let one = vdupq_n_f32(1.0);
            let dequant = vdupq_n_f32(DEQUANT);
            for index in (0..L2_SIZE).step_by(4) {
                let bias = vld1q_f32(weights.l1_biases.as_ptr().add(index));
                let value = vfmaq_f32(bias, vcvtq_f32_s32(sums[index / 4]), dequant);
                vst1q_f32(
                    l1.as_mut_ptr().add(index),
                    vmaxq_f32(vminq_f32(value, one), zero),
                );
            }

            let mut l2 = [vdupq_n_f32(0.0); L3_SIZE / 4];
            for index in (0..L3_SIZE).step_by(4) {
                l2[index / 4] = vld1q_f32(weights.l2_biases.as_ptr().add(index));
            }
            for (input, &activation) in l1.iter().enumerate() {
                let value = vdupq_n_f32(activation);
                let row = weights.l2.as_ptr().add(input * L3_SIZE);
                for output in (0..L3_SIZE).step_by(4) {
                    let weight = vld1q_f32(row.add(output));
                    l2[output / 4] = vfmaq_f32(l2[output / 4], weight, value);
                }
            }
            for index in (0..L3_SIZE).step_by(4) {
                l2[index / 4] = vmaxq_f32(vminq_f32(l2[index / 4], one), zero);
            }

            let mut totals = [vdupq_n_f32(0.0); 4];
            for (lane, total) in totals.iter_mut().enumerate() {
                for index in (lane * 4..L3_SIZE).step_by(16) {
                    *total = vfmaq_f32(
                        *total,
                        vld1q_f32(weights.l3.as_ptr().add(index)),
                        l2[index / 4],
                    );
                }
            }
            let sum02 = vaddq_f32(totals[0], totals[2]);
            let sum13 = vaddq_f32(totals[1], totals[3]);
            let sum = vaddq_f32(sum02, sum13);
            let pair = vadd_f32(vget_low_f32(sum), vget_high_f32(sum));
            vget_lane_f32::<0>(pair) + vget_lane_f32::<1>(pair) + weights.l3_bias
        }
    }

    pub(super) unsafe fn forward(
        piece: [&[i16; HIDDEN_SIZE]; 2],
        threat: [&[i16; HIDDEN_SIZE]; 2],
        stm: usize,
        weights: ForwardWeights<'_>,
    ) -> f32 {
        unsafe { forward_impl::<false>(piece, threat, stm, weights) }
    }

    #[target_feature(enable = "dotprod")]
    pub(super) unsafe fn forward_dotprod(
        piece: [&[i16; HIDDEN_SIZE]; 2],
        threat: [&[i16; HIDDEN_SIZE]; 2],
        stm: usize,
        weights: ForwardWeights<'_>,
    ) -> f32 {
        unsafe { forward_impl::<true>(piece, threat, stm, weights) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_is_initialized_once_and_matches_the_host() {
        let first = std::ptr::from_ref(kernels());
        let second = std::ptr::from_ref(kernels());
        assert_eq!(first, second);

        #[cfg(all(feature = "simd", target_arch = "aarch64"))]
        if std::arch::is_aarch64_feature_detected!("dotprod") {
            assert_eq!(selected_backend(), RecklessSimdBackend::ArmNeonDotprod);
        } else {
            assert_eq!(selected_backend(), RecklessSimdBackend::ArmNeon);
        }

        #[cfg(all(feature = "simd", target_arch = "x86_64"))]
        if std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma")
        {
            assert_eq!(selected_backend(), RecklessSimdBackend::X86Avx2Fma);
        } else {
            assert_eq!(selected_backend(), RecklessSimdBackend::Scalar);
        }

        #[cfg(not(all(feature = "simd", any(target_arch = "aarch64", target_arch = "x86_64"))))]
        assert_eq!(selected_backend(), RecklessSimdBackend::Scalar);
    }

    #[test]
    fn i8_rows_match_scalar() {
        let weights = (0..HIDDEN_SIZE * 8)
            .map(|index| (index as u8).wrapping_mul(37))
            .collect::<Vec<_>>();
        let mut expected = [0i16; HIDDEN_SIZE];
        let mut actual = expected;
        scalar::apply_i8_rows(&mut expected, &weights, &[1, 6], &[3]);
        apply_i8_rows(&mut actual, &weights, &[1, 6], &[3]);
        assert_eq!(actual, expected);
    }

    #[test]
    fn i16_rows_match_scalar() {
        let weights = (0..HIDDEN_SIZE * 8)
            .map(|index| (index as i16).wrapping_mul(83))
            .collect::<Vec<_>>();
        let mut expected = [0i16; HIDDEN_SIZE];
        let mut actual = expected;
        scalar::apply_i16_rows(&mut expected, &weights, &[2, 7], &[4]);
        apply_i16_rows(&mut actual, &weights, &[2, 7], &[4]);
        assert_eq!(actual, expected);
    }

    #[test]
    fn i16_rows_from_matches_copy_then_apply() {
        let weights = (0..HIDDEN_SIZE * 8)
            .map(|index| (index as i16).wrapping_mul(41))
            .collect::<Vec<_>>();
        let mut src = [0i16; HIDDEN_SIZE];
        for (index, value) in src.iter_mut().enumerate() {
            *value = (index as i16).wrapping_mul(3);
        }
        let mut expected = src;
        apply_i16_rows(&mut expected, &weights, &[1, 5], &[3]);
        let mut actual = [0i16; HIDDEN_SIZE];
        apply_i16_rows_from(&mut actual, &src, &weights, &[1, 5], &[3]);
        assert_eq!(actual, expected);
    }

    #[test]
    fn i8_rows_from_matches_copy_then_apply() {
        let weights = (0..HIDDEN_SIZE * 8)
            .map(|index| ((index * 17) % 200) as u8)
            .collect::<Vec<_>>();
        let mut src = [0i16; HIDDEN_SIZE];
        for (index, value) in src.iter_mut().enumerate() {
            *value = (index as i16).wrapping_mul(5) - 40;
        }
        let mut expected = src;
        apply_i8_rows(&mut expected, &weights, &[0, 4], &[2]);
        let mut actual = [0i16; HIDDEN_SIZE];
        apply_i8_rows_from(&mut actual, &src, &weights, &[0, 4], &[2]);
        assert_eq!(actual, expected);
    }

    #[test]
    fn forward_dispatch_matches_scalar() {
        let mut piece = [[0i16; HIDDEN_SIZE]; 2];
        let mut threat = [[0i16; HIDDEN_SIZE]; 2];
        for perspective in 0..2 {
            for index in 0..HIDDEN_SIZE {
                piece[perspective][index] = ((index * 17 + perspective * 31) % 401) as i16 - 100;
                threat[perspective][index] = ((index * 7 + perspective * 13) % 101) as i16 - 50;
            }
        }

        let l1 = (0..HIDDEN_SIZE * L2_SIZE)
            .map(|index| (((index * 11) % 9) as i8 - 4) as u8)
            .collect::<Vec<_>>();
        let l1_biases = (0..L2_SIZE)
            .map(|index| index as f32 * 0.003 - 0.02)
            .collect::<Vec<_>>();
        let l2 = (0..L2_SIZE * L3_SIZE)
            .map(|index| ((index * 5 % 17) as f32 - 8.0) * 0.002)
            .collect::<Vec<_>>();
        let l2_biases = (0..L3_SIZE)
            .map(|index| index as f32 * 0.001 - 0.01)
            .collect::<Vec<_>>();
        let l3 = (0..L3_SIZE)
            .map(|index| ((index * 3 % 11) as f32 - 5.0) * 0.01)
            .collect::<Vec<_>>();

        let inputs = [&piece[0], &piece[1]];
        let threats = [&threat[0], &threat[1]];
        let expected = scalar::forward(
            inputs,
            threats,
            1,
            ForwardWeights {
                l1: &l1,
                l1_biases: &l1_biases,
                l2: &l2,
                l2_biases: &l2_biases,
                l3: &l3,
                l3_bias: 0.125,
            },
        );
        let actual = forward(
            inputs,
            threats,
            1,
            ForwardWeights {
                l1: &l1,
                l1_biases: &l1_biases,
                l2: &l2,
                l2_biases: &l2_biases,
                l3: &l3,
                l3_bias: 0.125,
            },
        );
        assert!(
            (actual - expected).abs() <= 1.0e-5,
            "SIMD={actual}, scalar={expected}"
        );
    }
}
