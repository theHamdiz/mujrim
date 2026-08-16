//! Runtime-dispatched i16/i8 feature kernels.
//!
//! Stockfish uses the fixed-width helpers; Obsidian (and other i16 FTs) call
//! the width-generic `apply_*_feature_width` entry points without enabling
//! the Stockfish network format.

#![cfg_attr(not(feature = "stockfish-nnue"), allow(dead_code))]

#[cfg(feature = "stockfish-nnue")]
use super::stockfish_format::L1;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StockfishSimdBackend {
    Scalar,
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    X86Avx2,
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    X86Avx512,
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    X86Avx512Vnni,
    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    ArmNeon,
    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    ArmNeonDotprod,
}

impl StockfishSimdBackend {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Scalar => "scalar",
            #[cfg(all(feature = "simd", target_arch = "x86_64"))]
            Self::X86Avx2 => "AVX2",
            #[cfg(all(feature = "simd", target_arch = "x86_64"))]
            Self::X86Avx512 => "AVX-512",
            #[cfg(all(feature = "simd", target_arch = "x86_64"))]
            Self::X86Avx512Vnni => "AVX-512+VNNI",
            #[cfg(all(feature = "simd", target_arch = "aarch64"))]
            Self::ArmNeon => "NEON",
            #[cfg(all(feature = "simd", target_arch = "aarch64"))]
            Self::ArmNeonDotprod => "NEON+DotProd",
        }
    }
}

type AffineKernel = fn(&[u8], &[i8], &mut [i32]);
type ApplyI16Kernel = fn(&mut [i16], &[i16], i16);
type ApplyI8Kernel = fn(&mut [i16], &[i8], i16);
type TransformKernel = fn(&[i16], &[i16], &mut [u8]);

struct KernelDispatch {
    affine: AffineKernel,
    apply_i16: ApplyI16Kernel,
    apply_i8: ApplyI8Kernel,
    transform_pair: TransformKernel,
    backend: StockfishSimdBackend,
}

static KERNEL_DISPATCH: OnceLock<KernelDispatch> = OnceLock::new();

#[inline(always)]
fn kernels() -> &'static KernelDispatch {
    KERNEL_DISPATCH.get_or_init(detect_kernels)
}

fn detect_kernels() -> KernelDispatch {
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    {
        if std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
        {
            let vnni = std::arch::is_x86_feature_detected!("avx512vnni");
            return KernelDispatch {
                affine: if vnni {
                    avx512_affine_vnni
                } else {
                    avx512_affine
                },
                apply_i16: avx512_apply_i16,
                apply_i8: avx512_apply_i8,
                transform_pair: avx512_transform_pair,
                backend: if vnni {
                    StockfishSimdBackend::X86Avx512Vnni
                } else {
                    StockfishSimdBackend::X86Avx512
                },
            };
        }
        if std::arch::is_x86_feature_detected!("avx2") {
            return KernelDispatch {
                affine: avx2_affine,
                apply_i16: avx2_apply_i16,
                apply_i8: avx2_apply_i8,
                transform_pair: avx2_transform_pair,
                backend: StockfishSimdBackend::X86Avx2,
            };
        }
    }

    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    {
        if std::arch::is_aarch64_feature_detected!("dotprod") {
            return KernelDispatch {
                affine: neon_affine_dotprod,
                apply_i16: neon_apply_i16,
                apply_i8: neon_apply_i8,
                transform_pair: neon_transform_pair,
                backend: StockfishSimdBackend::ArmNeonDotprod,
            };
        }
        return KernelDispatch {
            affine: scalar::affine,
            apply_i16: neon_apply_i16,
            apply_i8: neon_apply_i8,
            transform_pair: neon_transform_pair,
            backend: StockfishSimdBackend::ArmNeon,
        };
    }

    #[allow(unreachable_code)]
    KernelDispatch {
        affine: scalar::affine,
        apply_i16: scalar::apply_i16,
        apply_i8: scalar::apply_i8,
        transform_pair: scalar::transform_pair,
        backend: StockfishSimdBackend::Scalar,
    }
}

pub(crate) fn affine(input: &[u8], weights: &[i8], output: &mut [i32]) {
    debug_assert_eq!(weights.len(), input.len() * output.len());
    (kernels().affine)(input, weights, output);
}

#[cfg(feature = "stockfish-nnue")]
pub(crate) fn apply_i16_feature(
    accumulator: &mut [i16; L1],
    weights: &[i16],
    feature: usize,
    sign: i16,
) {
    apply_i16_feature_width(accumulator, weights, feature, sign);
}

pub(crate) fn apply_i16_feature_width(
    accumulator: &mut [i16],
    weights: &[i16],
    feature: usize,
    sign: i16,
) {
    let width = accumulator.len();
    let row = &weights[feature * width..(feature + 1) * width];
    (kernels().apply_i16)(accumulator, row, sign);
}

/// Copy `src` into `dst` while adding/subtracting feature rows in one pass.
pub(crate) fn apply_i16_from_width(
    dst: &mut [i16],
    src: &[i16],
    weights: &[i16],
    adds: &[usize],
    subs: &[usize],
) {
    debug_assert_eq!(dst.len(), src.len());
    let width = dst.len();
    if dst.len().is_multiple_of(32) {
        #[cfg(all(feature = "simd", target_arch = "x86_64"))]
        if apply_from_uses_avx512() {
            unsafe {
                avx512::apply_i16_from(dst, src, weights, adds, subs);
            }
            return;
        }
    }
    if dst.len().is_multiple_of(16) {
        #[cfg(all(feature = "simd", target_arch = "x86_64"))]
        if apply_from_uses_avx2() {
            unsafe {
                avx2::apply_i16_from(dst, src, weights, adds, subs);
            }
            return;
        }
    }
    dst.copy_from_slice(src);
    for &feature in adds {
        (kernels().apply_i16)(dst, &weights[feature * width..(feature + 1) * width], 1);
    }
    for &feature in subs {
        (kernels().apply_i16)(dst, &weights[feature * width..(feature + 1) * width], -1);
    }
}

/// Copy `src` into `dst` while adding/subtracting i8 feature rows in one pass.
pub(crate) fn apply_i8_from_width(
    dst: &mut [i16],
    src: &[i16],
    weights: &[i8],
    adds: &[usize],
    subs: &[usize],
) {
    debug_assert_eq!(dst.len(), src.len());
    let width = dst.len();
    if dst.len().is_multiple_of(32) {
        #[cfg(all(feature = "simd", target_arch = "x86_64"))]
        if apply_from_uses_avx512() {
            unsafe {
                avx512::apply_i8_from(dst, src, weights, adds, subs);
            }
            return;
        }
    }
    if dst.len().is_multiple_of(16) {
        #[cfg(all(feature = "simd", target_arch = "x86_64"))]
        if apply_from_uses_avx2() {
            unsafe {
                avx2::apply_i8_from(dst, src, weights, adds, subs);
            }
            return;
        }
    }
    dst.copy_from_slice(src);
    for &feature in adds {
        (kernels().apply_i8)(dst, &weights[feature * width..(feature + 1) * width], 1);
    }
    for &feature in subs {
        (kernels().apply_i8)(dst, &weights[feature * width..(feature + 1) * width], -1);
    }
}

/// Pairwise FT on `(first + first_add, second + second_add)` without a full acc copy.
pub(crate) fn activate_shifted_pair_sum(
    first: &[i16],
    first_add: &[i16],
    second: &[i16],
    second_add: &[i16],
    output: &mut [u8],
) {
    debug_assert_eq!(first.len(), first_add.len());
    debug_assert_eq!(second.len(), second_add.len());
    debug_assert_eq!(first.len(), second.len());
    debug_assert_eq!(first.len(), output.len());
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    if first.len().is_multiple_of(16) && shifted_pair_uses_avx2() {
        unsafe {
            avx2::activate_shifted_pair_sum(first, first_add, second, second_add, output);
        }
        return;
    }
    scalar::activate_shifted_pair_sum(first, first_add, second, second_add, output);
}

/// Stockfish-style `/512` pairwise FT on summed accumulators.
pub(crate) fn transform_pair_sum(
    first: &[i16],
    first_add: &[i16],
    second: &[i16],
    second_add: &[i16],
    output: &mut [u8],
) {
    debug_assert_eq!(first.len(), first_add.len());
    debug_assert_eq!(second.len(), second_add.len());
    debug_assert_eq!(first.len(), second.len());
    debug_assert_eq!(first.len(), output.len());
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    if first.len().is_multiple_of(32) && apply_from_uses_avx512() {
        unsafe {
            avx512::transform_pair_sum(first, first_add, second, second_add, output);
        }
        return;
    }
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    if first.len().is_multiple_of(16) && shifted_pair_uses_avx2() {
        unsafe {
            avx2::transform_pair_sum(first, first_add, second, second_add, output);
        }
        return;
    }
    scalar::transform_pair_sum(first, first_add, second, second_add, output);
}

#[cfg(feature = "stockfish-nnue")]
pub(crate) fn apply_i8_feature(
    accumulator: &mut [i16; L1],
    weights: &[i8],
    feature: usize,
    sign: i16,
) {
    apply_i8_feature_width(accumulator, weights, feature, sign);
}

pub(crate) fn apply_i8_feature_width(
    accumulator: &mut [i16],
    weights: &[i8],
    feature: usize,
    sign: i16,
) {
    let width = accumulator.len();
    let row = &weights[feature * width..(feature + 1) * width];
    (kernels().apply_i8)(accumulator, row, sign);
}

pub(crate) fn transform_pair(first: &[i16], second: &[i16], output: &mut [u8]) {
    debug_assert_eq!(first.len(), second.len());
    debug_assert_eq!(first.len(), output.len());
    (kernels().transform_pair)(first, second, output);
}

/// Obsidian/PlentyChess pairwise FT: `(clamp(c0,0,255)<<7)*min(c1,255) >> 16`.
///
/// This is not Stockfish `transform_pair` (`/512` with both sides clamped to 0).
pub(crate) fn activate_shifted_pair(first: &[i16], second: &[i16], output: &mut [u8]) {
    debug_assert_eq!(first.len(), second.len());
    debug_assert_eq!(first.len(), output.len());
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    if first.len().is_multiple_of(16) && shifted_pair_uses_avx2() {
        // SAFETY: runtime AVX2 check above.
        unsafe {
            avx2::activate_shifted_pair(first, second, output);
        }
        return;
    }
    scalar::activate_shifted_pair(first, second, output);
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn shifted_pair_uses_avx2() -> bool {
    static AVX2: OnceLock<bool> = OnceLock::new();
    *AVX2.get_or_init(|| std::arch::is_x86_feature_detected!("avx2"))
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn apply_from_uses_avx2() -> bool {
    shifted_pair_uses_avx2()
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn apply_from_uses_avx512() -> bool {
    static AVX512: OnceLock<bool> = OnceLock::new();
    *AVX512.get_or_init(|| {
        std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
    })
}

pub(crate) fn selected_backend() -> &'static str {
    kernels().backend.name()
}

#[cfg(test)]
fn selected_backend_kind() -> StockfishSimdBackend {
    kernels().backend
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

    pub(super) fn activate_shifted_pair(first: &[i16], second: &[i16], output: &mut [u8]) {
        for ((target, &lhs), &rhs) in output.iter_mut().zip(first).zip(second) {
            let c0 = i32::from(lhs.clamp(0, 255));
            let c1 = i32::from(rhs.min(255));
            let prod = ((c0 << 7) * c1) >> 16;
            *target = prod.clamp(0, 255) as u8;
        }
    }

    pub(super) fn activate_shifted_pair_sum(
        first: &[i16],
        first_add: &[i16],
        second: &[i16],
        second_add: &[i16],
        output: &mut [u8],
    ) {
        for (index, target) in output.iter_mut().enumerate() {
            let lhs = first[index].wrapping_add(first_add[index]);
            let rhs = second[index].wrapping_add(second_add[index]);
            let c0 = i32::from(lhs.clamp(0, 255));
            let c1 = i32::from(rhs.min(255));
            let prod = ((c0 << 7) * c1) >> 16;
            *target = prod.clamp(0, 255) as u8;
        }
    }

    pub(super) fn transform_pair_sum(
        first: &[i16],
        first_add: &[i16],
        second: &[i16],
        second_add: &[i16],
        output: &mut [u8],
    ) {
        for (index, target) in output.iter_mut().enumerate() {
            let lhs = first[index].wrapping_add(first_add[index]);
            let rhs = second[index].wrapping_add(second_add[index]);
            let lhs = i32::from(lhs.clamp(0, 255));
            let rhs = i32::from(rhs.clamp(0, 255));
            *target = ((lhs * rhs) / 512) as u8;
        }
    }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx2_affine(input: &[u8], weights: &[i8], output: &mut [i32]) {
    // SAFETY: installed only after runtime AVX2 detection.
    unsafe { avx2::affine(input, weights, output) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx2_apply_i16(accumulator: &mut [i16], row: &[i16], sign: i16) {
    // SAFETY: installed only after runtime AVX2 detection.
    unsafe { avx2::apply_i16(accumulator, row, sign) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx2_apply_i8(accumulator: &mut [i16], row: &[i8], sign: i16) {
    // SAFETY: installed only after runtime AVX2 detection.
    unsafe { avx2::apply_i8(accumulator, row, sign) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx2_transform_pair(first: &[i16], second: &[i16], output: &mut [u8]) {
    // SAFETY: installed only after runtime AVX2 detection.
    unsafe { avx2::transform_pair(first, second, output) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx512_affine(input: &[u8], weights: &[i8], output: &mut [i32]) {
    // SAFETY: installed only after runtime AVX-512F+BW detection.
    unsafe { avx512::affine(input, weights, output) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx512_affine_vnni(input: &[u8], weights: &[i8], output: &mut [i32]) {
    // SAFETY: installed only after runtime AVX-512F+BW+VNNI detection.
    unsafe { avx512::affine_vnni(input, weights, output) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx512_apply_i16(accumulator: &mut [i16], row: &[i16], sign: i16) {
    // SAFETY: installed only after runtime AVX-512F+BW detection.
    unsafe { avx512::apply_i16(accumulator, row, sign) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx512_apply_i8(accumulator: &mut [i16], row: &[i8], sign: i16) {
    // SAFETY: installed only after runtime AVX-512F+BW detection.
    unsafe { avx512::apply_i8(accumulator, row, sign) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx512_transform_pair(first: &[i16], second: &[i16], output: &mut [u8]) {
    // SAFETY: AVX-512 implies AVX2; reuse the bit-exact 16-lane pack.
    unsafe { avx2::transform_pair(first, second, output) }
}

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
fn neon_affine_dotprod(input: &[u8], weights: &[i8], output: &mut [i32]) {
    // SAFETY: installed only after runtime DotProd detection.
    unsafe { neon::affine_dotprod(input, weights, output) }
}

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
fn neon_apply_i16(accumulator: &mut [i16], row: &[i16], sign: i16) {
    // SAFETY: NEON is part of the AArch64 baseline.
    unsafe { neon::apply_i16(accumulator, row, sign) }
}

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
fn neon_apply_i8(accumulator: &mut [i16], row: &[i8], sign: i16) {
    // SAFETY: NEON is part of the AArch64 baseline.
    unsafe { neon::apply_i8(accumulator, row, sign) }
}

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
fn neon_transform_pair(first: &[i16], second: &[i16], output: &mut [u8]) {
    // SAFETY: NEON is part of the AArch64 baseline.
    unsafe { neon::transform_pair(first, second, output) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
mod avx2 {
    #![allow(clippy::undocumented_unsafe_blocks)]

    use std::arch::x86_64::*;

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn apply_i16_from(
        dst: &mut [i16],
        src: &[i16],
        weights: &[i16],
        adds: &[usize],
        subs: &[usize],
    ) {
        unsafe {
            let width = dst.len();
            debug_assert_eq!(src.len(), width);
            debug_assert_eq!(width % 16, 0);
            for index in (0..width).step_by(16) {
                let mut value = _mm256_loadu_si256(src.as_ptr().add(index).cast());
                for &feature in adds {
                    let row = weights.as_ptr().add(feature * width + index);
                    value = _mm256_add_epi16(value, _mm256_loadu_si256(row.cast()));
                }
                for &feature in subs {
                    let row = weights.as_ptr().add(feature * width + index);
                    value = _mm256_sub_epi16(value, _mm256_loadu_si256(row.cast()));
                }
                _mm256_storeu_si256(dst.as_mut_ptr().add(index).cast(), value);
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn apply_i8_from(
        dst: &mut [i16],
        src: &[i16],
        weights: &[i8],
        adds: &[usize],
        subs: &[usize],
    ) {
        unsafe {
            let width = dst.len();
            debug_assert_eq!(src.len(), width);
            debug_assert_eq!(width % 16, 0);
            for index in (0..width).step_by(16) {
                let mut value = _mm256_loadu_si256(src.as_ptr().add(index).cast());
                for &feature in adds {
                    let row = weights.as_ptr().add(feature * width + index);
                    let delta = _mm256_cvtepi8_epi16(_mm_loadu_si128(row.cast()));
                    value = _mm256_add_epi16(value, delta);
                }
                for &feature in subs {
                    let row = weights.as_ptr().add(feature * width + index);
                    let delta = _mm256_cvtepi8_epi16(_mm_loadu_si128(row.cast()));
                    value = _mm256_sub_epi16(value, delta);
                }
                _mm256_storeu_si256(dst.as_mut_ptr().add(index).cast(), value);
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn activate_shifted_pair_sum(
        first: &[i16],
        first_add: &[i16],
        second: &[i16],
        second_add: &[i16],
        output: &mut [u8],
    ) {
        unsafe {
            let zero = _mm256_setzero_si256();
            let maximum = _mm256_set1_epi16(255);
            for index in (0..first.len()).step_by(16) {
                let lhs = _mm256_max_epi16(
                    _mm256_min_epi16(
                        _mm256_add_epi16(
                            _mm256_loadu_si256(first.as_ptr().add(index).cast()),
                            _mm256_loadu_si256(first_add.as_ptr().add(index).cast()),
                        ),
                        maximum,
                    ),
                    zero,
                );
                let rhs = _mm256_min_epi16(
                    _mm256_add_epi16(
                        _mm256_loadu_si256(second.as_ptr().add(index).cast()),
                        _mm256_loadu_si256(second_add.as_ptr().add(index).cast()),
                    ),
                    maximum,
                );
                let lhs_lo =
                    _mm256_slli_epi32::<7>(_mm256_cvtepi16_epi32(_mm256_castsi256_si128(lhs)));
                let lhs_hi = _mm256_slli_epi32::<7>(_mm256_cvtepi16_epi32(
                    _mm256_extracti128_si256::<1>(lhs),
                ));
                let rhs_lo = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(rhs));
                let rhs_hi = _mm256_cvtepi16_epi32(_mm256_extracti128_si256::<1>(rhs));
                let prod_lo = _mm256_srai_epi32::<16>(_mm256_mullo_epi32(lhs_lo, rhs_lo));
                let prod_hi = _mm256_srai_epi32::<16>(_mm256_mullo_epi32(lhs_hi, rhs_hi));
                let packed16 = _mm256_packus_epi32(prod_lo, prod_hi);
                let ordered = _mm256_permute4x64_epi64::<0b11_01_10_00>(packed16);
                let packed8 = _mm_packus_epi16(
                    _mm256_castsi256_si128(ordered),
                    _mm256_extracti128_si256::<1>(ordered),
                );
                _mm_storeu_si128(output.as_mut_ptr().add(index).cast(), packed8);
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn transform_pair_sum(
        first: &[i16],
        first_add: &[i16],
        second: &[i16],
        second_add: &[i16],
        output: &mut [u8],
    ) {
        unsafe {
            let zero = _mm256_setzero_si256();
            let maximum = _mm256_set1_epi16(255);
            for index in (0..first.len()).step_by(16) {
                let lhs = _mm256_max_epi16(
                    _mm256_min_epi16(
                        _mm256_add_epi16(
                            _mm256_loadu_si256(first.as_ptr().add(index).cast()),
                            _mm256_loadu_si256(first_add.as_ptr().add(index).cast()),
                        ),
                        maximum,
                    ),
                    zero,
                );
                let rhs = _mm256_max_epi16(
                    _mm256_min_epi16(
                        _mm256_add_epi16(
                            _mm256_loadu_si256(second.as_ptr().add(index).cast()),
                            _mm256_loadu_si256(second_add.as_ptr().add(index).cast()),
                        ),
                        maximum,
                    ),
                    zero,
                );
                let lhs_lo = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(lhs));
                let lhs_hi = _mm256_cvtepi16_epi32(_mm256_extracti128_si256::<1>(lhs));
                let rhs_lo = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(rhs));
                let rhs_hi = _mm256_cvtepi16_epi32(_mm256_extracti128_si256::<1>(rhs));
                let prod_lo = _mm256_srli_epi32::<9>(_mm256_mullo_epi32(lhs_lo, rhs_lo));
                let prod_hi = _mm256_srli_epi32::<9>(_mm256_mullo_epi32(lhs_hi, rhs_hi));
                let packed16 = _mm256_packus_epi32(prod_lo, prod_hi);
                let ordered = _mm256_permute4x64_epi64::<0b11_01_10_00>(packed16);
                let packed8 = _mm_packus_epi16(
                    _mm256_castsi256_si128(ordered),
                    _mm256_extracti128_si256::<1>(ordered),
                );
                _mm_storeu_si128(output.as_mut_ptr().add(index).cast(), packed8);
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn apply_i16(accumulator: &mut [i16], row: &[i16], sign: i16) {
        unsafe {
            debug_assert_eq!(accumulator.len() % 16, 0);
            debug_assert_eq!(row.len(), accumulator.len());
            for index in (0..accumulator.len()).step_by(16) {
                let current = _mm256_loadu_si256(accumulator.as_ptr().add(index).cast());
                let delta = _mm256_loadu_si256(row.as_ptr().add(index).cast());
                let updated = if sign > 0 {
                    _mm256_add_epi16(current, delta)
                } else {
                    _mm256_sub_epi16(current, delta)
                };
                _mm256_storeu_si256(accumulator.as_mut_ptr().add(index).cast(), updated);
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn apply_i8(accumulator: &mut [i16], row: &[i8], sign: i16) {
        unsafe {
            debug_assert_eq!(accumulator.len() % 16, 0);
            debug_assert_eq!(row.len(), accumulator.len());
            for index in (0..accumulator.len()).step_by(16) {
                let current = _mm256_loadu_si256(accumulator.as_ptr().add(index).cast());
                let bytes = _mm_loadu_si128(row.as_ptr().add(index).cast());
                let delta = _mm256_cvtepi8_epi16(bytes);
                let updated = if sign > 0 {
                    _mm256_add_epi16(current, delta)
                } else {
                    _mm256_sub_epi16(current, delta)
                };
                _mm256_storeu_si256(accumulator.as_mut_ptr().add(index).cast(), updated);
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn transform_pair(first: &[i16], second: &[i16], output: &mut [u8]) {
        unsafe {
            debug_assert_eq!(first.len() % 16, 0);
            debug_assert_eq!(second.len(), first.len());
            debug_assert_eq!(output.len(), first.len());
            let zero = _mm256_setzero_si256();
            let maximum = _mm256_set1_epi16(255);
            for index in (0..first.len()).step_by(16) {
                let lhs = _mm256_max_epi16(
                    _mm256_min_epi16(
                        _mm256_loadu_si256(first.as_ptr().add(index).cast()),
                        maximum,
                    ),
                    zero,
                );
                let rhs = _mm256_max_epi16(
                    _mm256_min_epi16(
                        _mm256_loadu_si256(second.as_ptr().add(index).cast()),
                        maximum,
                    ),
                    zero,
                );
                // (lhs * rhs) >> 9 using 32-bit products in two halves.
                let lhs_lo = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(lhs));
                let lhs_hi = _mm256_cvtepi16_epi32(_mm256_extracti128_si256::<1>(lhs));
                let rhs_lo = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(rhs));
                let rhs_hi = _mm256_cvtepi16_epi32(_mm256_extracti128_si256::<1>(rhs));
                let prod_lo = _mm256_srli_epi32::<9>(_mm256_mullo_epi32(lhs_lo, rhs_lo));
                let prod_hi = _mm256_srli_epi32::<9>(_mm256_mullo_epi32(lhs_hi, rhs_hi));
                let packed16 = _mm256_packus_epi32(prod_lo, prod_hi);
                let ordered = _mm256_permute4x64_epi64::<0b11_01_10_00>(packed16);
                let packed8 = _mm_packus_epi16(
                    _mm256_castsi256_si128(ordered),
                    _mm256_extracti128_si256::<1>(ordered),
                );
                _mm_storeu_si128(output.as_mut_ptr().add(index).cast(), packed8);
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn activate_shifted_pair(first: &[i16], second: &[i16], output: &mut [u8]) {
        unsafe {
            debug_assert_eq!(first.len() % 16, 0);
            debug_assert_eq!(second.len(), first.len());
            debug_assert_eq!(output.len(), first.len());
            let zero = _mm256_setzero_si256();
            let maximum = _mm256_set1_epi16(255);
            for index in (0..first.len()).step_by(16) {
                let lhs = _mm256_max_epi16(
                    _mm256_min_epi16(
                        _mm256_loadu_si256(first.as_ptr().add(index).cast()),
                        maximum,
                    ),
                    zero,
                );
                let rhs = _mm256_min_epi16(
                    _mm256_loadu_si256(second.as_ptr().add(index).cast()),
                    maximum,
                );
                let lhs_lo =
                    _mm256_slli_epi32::<7>(_mm256_cvtepi16_epi32(_mm256_castsi256_si128(lhs)));
                let lhs_hi = _mm256_slli_epi32::<7>(_mm256_cvtepi16_epi32(
                    _mm256_extracti128_si256::<1>(lhs),
                ));
                let rhs_lo = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(rhs));
                let rhs_hi = _mm256_cvtepi16_epi32(_mm256_extracti128_si256::<1>(rhs));
                let prod_lo = _mm256_srai_epi32::<16>(_mm256_mullo_epi32(lhs_lo, rhs_lo));
                let prod_hi = _mm256_srai_epi32::<16>(_mm256_mullo_epi32(lhs_hi, rhs_hi));
                let packed16 = _mm256_packus_epi32(prod_lo, prod_hi);
                let ordered = _mm256_permute4x64_epi64::<0b11_01_10_00>(packed16);
                let packed8 = _mm_packus_epi16(
                    _mm256_castsi256_si128(ordered),
                    _mm256_extracti128_si256::<1>(ordered),
                );
                _mm_storeu_si128(output.as_mut_ptr().add(index).cast(), packed8);
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn affine(input: &[u8], weights: &[i8], output: &mut [i32]) {
        unsafe {
            debug_assert_eq!(input.len() % 32, 0);
            let input_len = input.len();
            let ones = _mm256_set1_epi16(1);
            for (row_index, value) in output.iter_mut().enumerate() {
                let row = weights.as_ptr().add(row_index * input_len);
                let mut sum = _mm256_setzero_si256();
                for index in (0..input_len).step_by(32) {
                    let activations = _mm256_loadu_si256(input.as_ptr().add(index).cast());
                    let row_weights = _mm256_loadu_si256(row.add(index).cast());
                    // maddubs treats the first operand as unsigned bytes (activations 0..255).
                    let pairwise = _mm256_maddubs_epi16(activations, row_weights);
                    sum = _mm256_add_epi32(sum, _mm256_madd_epi16(pairwise, ones));
                }
                let high = _mm256_extracti128_si256::<1>(sum);
                let low = _mm256_castsi256_si128(sum);
                let reduced = _mm_add_epi32(low, high);
                let shuffled = _mm_shuffle_epi32::<0b01_00_11_10>(reduced);
                let reduced = _mm_add_epi32(reduced, shuffled);
                let shuffled = _mm_shuffle_epi32::<0b10_11_00_01>(reduced);
                let reduced = _mm_add_epi32(reduced, shuffled);
                *value = value.wrapping_add(_mm_cvtsi128_si32(reduced));
            }
        }
    }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
mod avx512 {
    #![allow(clippy::undocumented_unsafe_blocks)]

    use std::arch::x86_64::*;

    #[inline(always)]
    unsafe fn load_i16(ptr: *const i16) -> __m512i {
        unsafe { _mm512_loadu_si512(ptr.cast()) }
    }

    #[inline(always)]
    unsafe fn store_i16(ptr: *mut i16, value: __m512i) {
        unsafe { _mm512_storeu_si512(ptr.cast(), value) }
    }

    #[target_feature(enable = "avx512f,avx512bw")]
    pub(super) unsafe fn apply_i16(accumulator: &mut [i16], row: &[i16], sign: i16) {
        unsafe {
            debug_assert_eq!(accumulator.len() % 32, 0);
            debug_assert_eq!(row.len(), accumulator.len());
            for index in (0..accumulator.len()).step_by(32) {
                let current = load_i16(accumulator.as_ptr().add(index));
                let delta = load_i16(row.as_ptr().add(index));
                let updated = if sign > 0 {
                    _mm512_add_epi16(current, delta)
                } else {
                    _mm512_sub_epi16(current, delta)
                };
                store_i16(accumulator.as_mut_ptr().add(index), updated);
            }
        }
    }

    #[target_feature(enable = "avx512f,avx512bw")]
    pub(super) unsafe fn apply_i8(accumulator: &mut [i16], row: &[i8], sign: i16) {
        unsafe {
            debug_assert_eq!(accumulator.len() % 32, 0);
            debug_assert_eq!(row.len(), accumulator.len());
            for index in (0..accumulator.len()).step_by(32) {
                let current = load_i16(accumulator.as_ptr().add(index));
                let delta =
                    _mm512_cvtepi8_epi16(_mm256_loadu_si256(row.as_ptr().add(index).cast()));
                let updated = if sign > 0 {
                    _mm512_add_epi16(current, delta)
                } else {
                    _mm512_sub_epi16(current, delta)
                };
                store_i16(accumulator.as_mut_ptr().add(index), updated);
            }
        }
    }

    #[inline(always)]
    unsafe fn dpbusd_maddubs(sum: __m512i, activations: __m512i, weights: __m512i) -> __m512i {
        unsafe {
            let pairwise = _mm512_maddubs_epi16(activations, weights);
            _mm512_add_epi32(sum, _mm512_madd_epi16(pairwise, _mm512_set1_epi16(1)))
        }
    }

    #[inline(always)]
    unsafe fn horizontal_sum_i32(sum: __m512i) -> i32 {
        unsafe {
            let high = _mm512_extracti64x4_epi64::<1>(sum);
            let low = _mm512_castsi512_si256(sum);
            let reduced = _mm256_add_epi32(low, high);
            let high = _mm256_extracti128_si256::<1>(reduced);
            let low = _mm256_castsi256_si128(reduced);
            let reduced = _mm_add_epi32(low, high);
            let shuffled = _mm_shuffle_epi32::<0b01_00_11_10>(reduced);
            let reduced = _mm_add_epi32(reduced, shuffled);
            let shuffled = _mm_shuffle_epi32::<0b10_11_00_01>(reduced);
            let reduced = _mm_add_epi32(reduced, shuffled);
            _mm_cvtsi128_si32(reduced)
        }
    }

    #[target_feature(enable = "avx512f,avx512bw")]
    pub(super) unsafe fn affine(input: &[u8], weights: &[i8], output: &mut [i32]) {
        unsafe {
            debug_assert_eq!(input.len() % 64, 0);
            let input_len = input.len();
            for (row_index, value) in output.iter_mut().enumerate() {
                let row = weights.as_ptr().add(row_index * input_len);
                let mut sum = _mm512_setzero_si512();
                for index in (0..input_len).step_by(64) {
                    let activations = _mm512_loadu_si512(input.as_ptr().add(index).cast());
                    let row_weights = _mm512_loadu_si512(row.add(index).cast());
                    sum = dpbusd_maddubs(sum, activations, row_weights);
                }
                *value = value.wrapping_add(horizontal_sum_i32(sum));
            }
        }
    }

    #[target_feature(enable = "avx512f,avx512bw")]
    pub(super) unsafe fn apply_i16_from(
        dst: &mut [i16],
        src: &[i16],
        weights: &[i16],
        adds: &[usize],
        subs: &[usize],
    ) {
        unsafe {
            let width = dst.len();
            debug_assert_eq!(src.len(), width);
            debug_assert_eq!(width % 32, 0);
            const REGS: usize = 16;
            const CHUNK: usize = 32;
            const UNROLL: usize = CHUNK * REGS;
            if adds.len() == 1 && subs.len() == 1 && width.is_multiple_of(UNROLL) {
                let add_row = weights.as_ptr().add(adds[0] * width);
                let sub_row = weights.as_ptr().add(subs[0] * width);
                _mm_prefetch::<_MM_HINT_T0>(add_row.cast::<i8>());
                _mm_prefetch::<_MM_HINT_T0>(sub_row.cast::<i8>());
                for base in (0..width).step_by(UNROLL) {
                    let add_base = add_row.add(base);
                    let sub_base = sub_row.add(base);
                    for r_idx in 0..REGS {
                        let off = r_idx * CHUNK;
                        let value = _mm512_sub_epi16(
                            load_i16(src.as_ptr().add(base + off)),
                            load_i16(sub_base.add(off)),
                        );
                        store_i16(
                            dst.as_mut_ptr().add(base + off),
                            _mm512_add_epi16(value, load_i16(add_base.add(off))),
                        );
                    }
                }
                return;
            }
            if width.is_multiple_of(UNROLL) {
                for base in (0..width).step_by(UNROLL) {
                    let mut regs = [_mm512_setzero_si512(); REGS];
                    for (r_idx, reg) in regs.iter_mut().enumerate() {
                        *reg = load_i16(src.as_ptr().add(base + r_idx * CHUNK));
                    }
                    for &feature in subs {
                        let row = weights.as_ptr().add(feature * width + base);
                        for (r_idx, reg) in regs.iter_mut().enumerate() {
                            *reg = _mm512_sub_epi16(*reg, load_i16(row.add(r_idx * CHUNK)));
                        }
                    }
                    for &feature in adds {
                        let row = weights.as_ptr().add(feature * width + base);
                        for (r_idx, reg) in regs.iter_mut().enumerate() {
                            *reg = _mm512_add_epi16(*reg, load_i16(row.add(r_idx * CHUNK)));
                        }
                    }
                    for (r_idx, reg) in regs.iter().enumerate() {
                        store_i16(dst.as_mut_ptr().add(base + r_idx * CHUNK), *reg);
                    }
                }
                return;
            }
            for index in (0..width).step_by(32) {
                let mut value = load_i16(src.as_ptr().add(index));
                for &feature in adds {
                    let row = weights.as_ptr().add(feature * width + index);
                    value = _mm512_add_epi16(value, load_i16(row));
                }
                for &feature in subs {
                    let row = weights.as_ptr().add(feature * width + index);
                    value = _mm512_sub_epi16(value, load_i16(row));
                }
                store_i16(dst.as_mut_ptr().add(index), value);
            }
        }
    }

    #[target_feature(enable = "avx512f,avx512bw")]
    pub(super) unsafe fn apply_i8_from(
        dst: &mut [i16],
        src: &[i16],
        weights: &[i8],
        adds: &[usize],
        subs: &[usize],
    ) {
        unsafe {
            let width = dst.len();
            debug_assert_eq!(src.len(), width);
            debug_assert_eq!(width % 32, 0);
            const REGS: usize = 16;
            const CHUNK: usize = 32;
            const UNROLL: usize = CHUNK * REGS;
            if width.is_multiple_of(UNROLL) {
                // Each i8 row is `width` bytes (1024 for sandhi). Prefetch every
                // cache line, not just the first — official only touches the
                // start, then pays for the second 512-byte unroll.
                for &feature in adds.iter().chain(subs.iter()) {
                    let row = weights.as_ptr().add(feature * width);
                    let mut line = 0;
                    while line < width {
                        _mm_prefetch::<_MM_HINT_T0>(row.add(line).cast::<i8>());
                        line += 64;
                    }
                }
                for base in (0..width).step_by(UNROLL) {
                    let mut regs = [_mm512_setzero_si512(); REGS];
                    for (r_idx, reg) in regs.iter_mut().enumerate() {
                        *reg = load_i16(src.as_ptr().add(base + r_idx * CHUNK));
                    }
                    for &feature in subs {
                        let row = weights.as_ptr().add(feature * width + base);
                        for (r_idx, reg) in regs.iter_mut().enumerate() {
                            let delta = _mm512_cvtepi8_epi16(_mm256_loadu_si256(
                                row.add(r_idx * CHUNK).cast(),
                            ));
                            *reg = _mm512_sub_epi16(*reg, delta);
                        }
                    }
                    for &feature in adds {
                        let row = weights.as_ptr().add(feature * width + base);
                        for (r_idx, reg) in regs.iter_mut().enumerate() {
                            let delta = _mm512_cvtepi8_epi16(_mm256_loadu_si256(
                                row.add(r_idx * CHUNK).cast(),
                            ));
                            *reg = _mm512_add_epi16(*reg, delta);
                        }
                    }
                    for (r_idx, reg) in regs.iter().enumerate() {
                        store_i16(dst.as_mut_ptr().add(base + r_idx * CHUNK), *reg);
                    }
                }
                return;
            }
            for index in (0..width).step_by(32) {
                let mut value = load_i16(src.as_ptr().add(index));
                for &feature in adds {
                    let row = weights.as_ptr().add(feature * width + index);
                    let delta = _mm512_cvtepi8_epi16(_mm256_loadu_si256(row.cast()));
                    value = _mm512_add_epi16(value, delta);
                }
                for &feature in subs {
                    let row = weights.as_ptr().add(feature * width + index);
                    let delta = _mm512_cvtepi8_epi16(_mm256_loadu_si256(row.cast()));
                    value = _mm512_sub_epi16(value, delta);
                }
                store_i16(dst.as_mut_ptr().add(index), value);
            }
        }
    }

    #[target_feature(enable = "avx512f,avx512bw")]
    pub(super) unsafe fn transform_pair_sum(
        first: &[i16],
        first_add: &[i16],
        second: &[i16],
        second_add: &[i16],
        output: &mut [u8],
    ) {
        unsafe {
            let zero = _mm512_setzero_si512();
            let maximum = _mm512_set1_epi16(255);
            for index in (0..first.len()).step_by(32) {
                let lhs = _mm512_max_epi16(
                    _mm512_min_epi16(
                        _mm512_add_epi16(
                            load_i16(first.as_ptr().add(index)),
                            load_i16(first_add.as_ptr().add(index)),
                        ),
                        maximum,
                    ),
                    zero,
                );
                let rhs = _mm512_max_epi16(
                    _mm512_min_epi16(
                        _mm512_add_epi16(
                            load_i16(second.as_ptr().add(index)),
                            load_i16(second_add.as_ptr().add(index)),
                        ),
                        maximum,
                    ),
                    zero,
                );
                let lhs_lo = _mm512_cvtepi16_epi32(_mm512_castsi512_si256(lhs));
                let lhs_hi = _mm512_cvtepi16_epi32(_mm512_extracti64x4_epi64::<1>(lhs));
                let rhs_lo = _mm512_cvtepi16_epi32(_mm512_castsi512_si256(rhs));
                let rhs_hi = _mm512_cvtepi16_epi32(_mm512_extracti64x4_epi64::<1>(rhs));
                let prod_lo = _mm512_srli_epi32::<9>(_mm512_mullo_epi32(lhs_lo, rhs_lo));
                let prod_hi = _mm512_srli_epi32::<9>(_mm512_mullo_epi32(lhs_hi, rhs_hi));
                _mm_storeu_si128(
                    output.as_mut_ptr().add(index).cast(),
                    _mm512_cvtepi32_epi8(prod_lo),
                );
                _mm_storeu_si128(
                    output.as_mut_ptr().add(index + 16).cast(),
                    _mm512_cvtepi32_epi8(prod_hi),
                );
            }
        }
    }

    #[target_feature(enable = "avx512f,avx512bw,avx512vnni")]
    pub(super) unsafe fn affine_vnni(input: &[u8], weights: &[i8], output: &mut [i32]) {
        unsafe {
            debug_assert_eq!(input.len() % 64, 0);
            let input_len = input.len();
            for (row_index, value) in output.iter_mut().enumerate() {
                let row = weights.as_ptr().add(row_index * input_len);
                let mut sum = _mm512_setzero_si512();
                for index in (0..input_len).step_by(64) {
                    let activations = _mm512_loadu_si512(input.as_ptr().add(index).cast());
                    let row_weights = _mm512_loadu_si512(row.add(index).cast());
                    sum = _mm512_dpbusd_epi32(sum, activations, row_weights);
                }
                *value = value.wrapping_add(horizontal_sum_i32(sum));
            }
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
    fn dispatched_affine_matches_scalar_for_stockfish_l1_width() {
        const WIDTH: usize = 1024;
        let input = (0..WIDTH)
            .map(|index| (index % 127) as u8)
            .collect::<Vec<_>>();
        let weights = (0..WIDTH * 8)
            .map(|index| (index as i8).wrapping_mul(3).wrapping_sub(40))
            .collect::<Vec<_>>();
        let mut expected = [5_i32; 8];
        let mut actual = expected;
        scalar::affine(&input, &weights, &mut expected);
        affine(&input, &weights, &mut actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn dispatched_feature_updates_match_scalar() {
        const WIDTH: usize = 1536;
        let i16_weights = (0..WIDTH * 2)
            .map(|index| (index as i16).wrapping_mul(29))
            .collect::<Vec<_>>();
        let i8_weights = (0..WIDTH * 2)
            .map(|index| (index as i8).wrapping_mul(13))
            .collect::<Vec<_>>();
        let mut expected = [7_i16; WIDTH];
        let mut actual = expected;
        scalar::apply_i16(&mut expected, &i16_weights[WIDTH..], -1);
        apply_i16_feature_width(&mut actual, &i16_weights, 1, -1);
        scalar::apply_i8(&mut expected, &i8_weights[..WIDTH], 1);
        apply_i8_feature_width(&mut actual, &i8_weights, 0, 1);
        assert_eq!(actual, expected);
    }

    #[test]
    fn dispatched_transform_pair_matches_scalar() {
        const WIDTH: usize = 1024;
        let first = (0..WIDTH / 2)
            .map(|index| index as i16 - 127)
            .collect::<Vec<_>>();
        let second = (0..WIDTH / 2)
            .map(|index| 383 - index as i16)
            .collect::<Vec<_>>();
        let mut expected = [0_u8; WIDTH / 2];
        let mut actual = expected;
        scalar::transform_pair(&first, &second, &mut expected);
        transform_pair(&first, &second, &mut actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn apply_i8_from_matches_copy_then_apply() {
        const WIDTH: usize = 1024;
        let weights = (0..WIDTH * 4)
            .map(|index| (index as i16).wrapping_mul(13) as i8)
            .collect::<Vec<_>>();
        let src = (0..WIDTH)
            .map(|index| (index as i16).wrapping_mul(5))
            .collect::<Vec<_>>();
        let mut expected = src.clone();
        apply_i8_feature_width(&mut expected, &weights, 1, 1);
        apply_i8_feature_width(&mut expected, &weights, 3, -1);
        apply_i8_feature_width(&mut expected, &weights, 0, 1);
        let mut actual = vec![0_i16; WIDTH];
        apply_i8_from_width(&mut actual, &src, &weights, &[1, 0], &[3]);
        assert_eq!(actual, expected);
    }

    #[test]
    fn apply_i16_from_matches_copy_then_apply() {
        const WIDTH: usize = 1536;
        let weights = (0..WIDTH * 4)
            .map(|index| (index as i16).wrapping_mul(17))
            .collect::<Vec<_>>();
        let src = (0..WIDTH)
            .map(|index| (index as i16).wrapping_mul(3))
            .collect::<Vec<_>>();
        let mut expected = src.clone();
        apply_i16_feature_width(&mut expected, &weights, 1, 1);
        apply_i16_feature_width(&mut expected, &weights, 3, -1);
        apply_i16_feature_width(&mut expected, &weights, 0, 1);
        let mut actual = vec![0_i16; WIDTH];
        apply_i16_from_width(&mut actual, &src, &weights, &[1, 0], &[3]);
        assert_eq!(actual, expected);
    }

    #[test]
    fn transform_pair_sum_matches_scalar() {
        const WIDTH: usize = 1024;
        let first = (0..WIDTH)
            .map(|index| index as i16 - 40)
            .collect::<Vec<_>>();
        let first_add = (0..WIDTH)
            .map(|index| (index as i16 % 11) - 3)
            .collect::<Vec<_>>();
        let second = (0..WIDTH)
            .map(|index| 180 - index as i16)
            .collect::<Vec<_>>();
        let second_add = (0..WIDTH)
            .map(|index| (index as i16 % 7) - 2)
            .collect::<Vec<_>>();
        let mut expected = vec![0_u8; WIDTH];
        let mut actual = vec![0_u8; WIDTH];
        scalar::transform_pair_sum(&first, &first_add, &second, &second_add, &mut expected);
        transform_pair_sum(&first, &first_add, &second, &second_add, &mut actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn activate_pair_sum_matches_add_then_activate() {
        const WIDTH: usize = 768;
        let first = (0..WIDTH)
            .map(|index| index as i16 - 80)
            .collect::<Vec<_>>();
        let first_add = (0..WIDTH)
            .map(|index| (index as i16 % 40) - 10)
            .collect::<Vec<_>>();
        let second = (0..WIDTH)
            .map(|index| 200 - index as i16)
            .collect::<Vec<_>>();
        let second_add = (0..WIDTH)
            .map(|index| (index as i16 % 17) - 4)
            .collect::<Vec<_>>();
        let mut summed_first = first.clone();
        let mut summed_second = second.clone();
        for (dst, add) in summed_first.iter_mut().zip(&first_add) {
            *dst = dst.wrapping_add(*add);
        }
        for (dst, add) in summed_second.iter_mut().zip(&second_add) {
            *dst = dst.wrapping_add(*add);
        }
        let mut expected = vec![0_u8; WIDTH];
        let mut actual = vec![0_u8; WIDTH];
        activate_shifted_pair(&summed_first, &summed_second, &mut expected);
        activate_shifted_pair_sum(&first, &first_add, &second, &second_add, &mut actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn dispatched_shifted_pair_matches_obsidian_plenty_formula() {
        const WIDTH: usize = 768;
        let first = (0..WIDTH)
            .map(|index| index as i16 - 200)
            .collect::<Vec<_>>();
        let second = (0..WIDTH)
            .map(|index| 400 - index as i16)
            .collect::<Vec<_>>();
        let mut expected = [0_u8; WIDTH];
        let mut actual = expected;
        scalar::activate_shifted_pair(&first, &second, &mut expected);
        activate_shifted_pair(&first, &second, &mut actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn selected_backend_is_vectorized_on_supported_hosts() {
        let name = selected_backend();
        let kind = selected_backend_kind();
        #[cfg(all(feature = "simd", target_arch = "x86_64"))]
        if std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
        {
            if std::arch::is_x86_feature_detected!("avx512vnni") {
                assert_eq!(name, "AVX-512+VNNI");
                assert_eq!(kind, StockfishSimdBackend::X86Avx512Vnni);
            } else {
                assert_eq!(name, "AVX-512");
                assert_eq!(kind, StockfishSimdBackend::X86Avx512);
            }
        } else if std::arch::is_x86_feature_detected!("avx2") {
            assert_eq!(name, "AVX2");
            assert_eq!(kind, StockfishSimdBackend::X86Avx2);
        }
        #[cfg(all(feature = "simd", target_arch = "aarch64"))]
        assert!(name.starts_with("NEON"));
        assert_eq!(name, kind.name());
    }
}
