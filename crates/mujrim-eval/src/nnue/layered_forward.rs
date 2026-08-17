//! Shared NNZ-sparse L1 and float L2/L3 kernels for layered adapter nets.
//!
//! Weight layout for L1 stays `[output][input]` so the existing dense affine
//! path remains the bit-exact reference. NNZ skips all-zero 4-byte activation
//! blocks the same way Obsidian / Viridithas / Reckless do.

use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::sync::OnceLock;

const NNZ_BLOCK: usize = 4;
const MAX_NNZ_BLOCKS: usize = 1024;

#[repr(C, align(64))]
pub(crate) struct Align64<T>(pub T);

impl<T> Align64<T> {
    #[inline]
    pub(crate) fn new(value: T) -> Self {
        Self(value)
    }
}

/// Heap slice forced to 64-byte alignment so AVX-512 loads do not split lines.
pub(crate) struct Align64Box<T> {
    ptr: NonNull<T>,
    len: usize,
}

unsafe impl<T: Send> Send for Align64Box<T> {}
unsafe impl<T: Sync> Sync for Align64Box<T> {}

impl<T> Align64Box<T> {
    fn layout(len: usize) -> Layout {
        let bytes = size_of::<T>() * len;
        let align = if bytes >= 2 * 1024 * 1024 {
            2 * 1024 * 1024
        } else {
            64.max(align_of::<T>())
        };
        Layout::from_size_align(bytes, align).expect("Align64Box layout")
    }
}

impl<T: Copy> Align64Box<T> {
    pub(crate) fn from_slice(src: &[T]) -> Self {
        let len = src.len();
        if len == 0 {
            return Self {
                ptr: NonNull::dangling(),
                len: 0,
            };
        }
        let layout = Self::layout(len);
        unsafe {
            let raw = alloc(layout);
            if raw.is_null() {
                handle_alloc_error(layout);
            }
            raw.cast::<T>().copy_from_nonoverlapping(src.as_ptr(), len);
            advise_huge_pages(raw, layout.size());
            Self {
                ptr: NonNull::new_unchecked(raw.cast()),
                len,
            }
        }
    }
}

impl<T> Deref for Align64Box<T> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &[T] {
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl<T> DerefMut for Align64Box<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl<T> Drop for Align64Box<T> {
    fn drop(&mut self) {
        if self.len == 0 {
            return;
        }
        let layout = Self::layout(self.len);
        unsafe {
            dealloc(self.ptr.as_ptr().cast(), layout);
        }
    }
}

#[inline]
fn advise_huge_pages(ptr: *mut u8, len: usize) {
    #[cfg(all(unix, target_os = "linux"))]
    {
        const MADV_HUGEPAGE: i32 = 14;
        const MADV_WILLNEED: i32 = 3;
        unsafe extern "C" {
            fn madvise(addr: *mut core::ffi::c_void, len: usize, advice: i32) -> i32;
        }
        if len == 0 || ptr.is_null() {
            return;
        }
        unsafe {
            let addr = ptr.cast::<core::ffi::c_void>();
            let _ = madvise(addr, len, MADV_HUGEPAGE);
            let _ = madvise(addr, len, MADV_WILLNEED);
        }
    }
    #[cfg(not(all(unix, target_os = "linux")))]
    {
        let _ = (ptr, len);
    }
}

#[inline]
pub(crate) fn find_nnz(ft_out: &[u8], indexes: &mut [u16]) -> usize {
    debug_assert_eq!(ft_out.len() % NNZ_BLOCK, 0);
    debug_assert!(indexes.len() >= ft_out.len() / NNZ_BLOCK);
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    if uses_avx512_vnni() {
        return unsafe { avx512::find_nnz(ft_out, indexes) };
    }
    (kernels().find_nnz)(ft_out, indexes)
}

fn find_nnz_scalar(ft_out: &[u8], indexes: &mut [u16]) -> usize {
    let blocks = ft_out.len() / NNZ_BLOCK;
    let packed = unsafe { std::slice::from_raw_parts(ft_out.as_ptr().cast::<u32>(), blocks) };
    let mut count = 0;
    for (block, &value) in packed.iter().enumerate() {
        if value != 0 {
            indexes[count] = block as u16;
            count += 1;
        }
    }
    count
}

/// Pack `[output][input]` L1 weights into the official `[block][output][4]` dpbusd layout.
#[cfg(test)]
pub(crate) fn pack_nnz_layout(src: &[i8], inputs: usize, outputs: usize) -> Box<[i8]> {
    debug_assert_eq!(src.len(), inputs * outputs);
    debug_assert_eq!(inputs % NNZ_BLOCK, 0);
    let mut dst = vec![0i8; src.len()].into_boxed_slice();
    pack_nnz_into(src, &mut dst, inputs, outputs);
    dst
}

/// Pack each output-bucket of `[bucket][output][input]` into `[bucket][block][output][4]`.
pub(crate) fn pack_nnz_buckets(
    src: &[i8],
    buckets: usize,
    inputs: usize,
    outputs: usize,
) -> Box<[i8]> {
    debug_assert_eq!(src.len(), buckets * inputs * outputs);
    debug_assert_eq!(inputs % NNZ_BLOCK, 0);
    let mut dst = vec![0i8; src.len()].into_boxed_slice();
    let bucket_size = inputs * outputs;
    for bucket in 0..buckets {
        let start = bucket * bucket_size;
        pack_nnz_into(
            &src[start..start + bucket_size],
            &mut dst[start..start + bucket_size],
            inputs,
            outputs,
        );
    }
    dst
}

/// Pack `[input][bucket][output]` float rows into `[bucket][input][output]`.
pub(crate) fn pack_f32_buckets(
    src: &[f32],
    buckets: usize,
    inputs: usize,
    outputs: usize,
) -> Box<[f32]> {
    debug_assert_eq!(src.len(), buckets * inputs * outputs);
    let mut dst = vec![0f32; src.len()].into_boxed_slice();
    for input in 0..inputs {
        for bucket in 0..buckets {
            let src_base = (input * buckets + bucket) * outputs;
            let dst_base = (bucket * inputs + input) * outputs;
            dst[dst_base..dst_base + outputs].copy_from_slice(&src[src_base..src_base + outputs]);
        }
    }
    dst
}

fn pack_nnz_into(src: &[i8], dst: &mut [i8], inputs: usize, outputs: usize) {
    let blocks = inputs / NNZ_BLOCK;
    for block in 0..blocks {
        for output in 0..outputs {
            let src_base = output * inputs + block * NNZ_BLOCK;
            let dst_base = block * outputs * NNZ_BLOCK + output * NNZ_BLOCK;
            dst[dst_base..dst_base + NNZ_BLOCK]
                .copy_from_slice(&src[src_base..src_base + NNZ_BLOCK]);
        }
    }
}

/// Row-major `[output][input]` path used as the bit-exact test reference.
///
/// Production adapters store packed weights and call [`affine_sparse_packed`]
/// so the hot path never gathers into a stack buffer.
#[cfg(test)]
#[inline]
pub(crate) fn affine_sparse(input: &[u8], weights: &[i8], output: &mut [i32]) {
    debug_assert_eq!(weights.len(), input.len() * output.len());
    debug_assert_eq!(input.len() % NNZ_BLOCK, 0);
    let blocks = input.len() / NNZ_BLOCK;
    if blocks > MAX_NNZ_BLOCKS {
        super::stockfish_simd::affine(input, weights, output);
        return;
    }
    let mut nnz = [0u16; MAX_NNZ_BLOCKS];
    let count = find_nnz(input, &mut nnz[..blocks]);
    if count == 0 {
        return;
    }
    if count == blocks {
        super::stockfish_simd::affine(input, weights, output);
        return;
    }
    let packed = pack_nnz_layout(weights, input.len(), output.len());
    (kernels().blocked)(input, &nnz[..count], &packed, output);
}

/// Sparse L1 over weights already packed as `[block][output][4]`.
#[inline]
pub(crate) fn affine_sparse_packed(input: &[u8], weights: &[i8], output: &mut [i32]) {
    debug_assert_eq!(weights.len(), input.len() * output.len());
    debug_assert_eq!(input.len() % NNZ_BLOCK, 0);
    let blocks = input.len() / NNZ_BLOCK;
    debug_assert!(blocks <= MAX_NNZ_BLOCKS);
    let mut nnz = [0u16; MAX_NNZ_BLOCKS];
    let count = find_nnz(input, &mut nnz[..blocks]);
    if count == 0 {
        return;
    }
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    if uses_avx512_vnni() {
        unsafe {
            avx512::blocked(input, &nnz[..count], weights, output);
        }
        return;
    }
    (kernels().blocked)(input, &nnz[..count], weights, output);
}

#[inline]
pub(crate) fn affine_f32(inputs: &[f32], weights: &[f32], biases: &[f32], outputs: &mut [f32]) {
    debug_assert_eq!(outputs.len(), biases.len());
    debug_assert_eq!(weights.len(), inputs.len() * outputs.len());
    outputs.copy_from_slice(biases);
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    if uses_avx512_vnni() {
        unsafe {
            avx512::affine_f32(inputs, weights, outputs);
        }
        return;
    }
    (kernels().affine_f32)(inputs, weights, outputs);
}

#[inline]
pub(crate) fn dot_f32(inputs: &[f32], weights: &[f32], bias: f32) -> f32 {
    debug_assert_eq!(inputs.len(), weights.len());
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    if uses_avx512_vnni() {
        return unsafe { avx512::dot_f32(inputs, weights, bias) };
    }
    (kernels().dot_f32)(inputs, weights, bias)
}

const SWISH_K: f32 = 6.0;

#[inline]
pub(crate) fn hard_swish6(value: f32) -> f32 {
    value * (value + SWISH_K * 0.5).clamp(0.0, SWISH_K) / SWISH_K
}

/// Official sandhi L1 finish: `swish(sum * scale + bias)`.
#[inline]
pub(crate) fn hard_swish6_bias(sums: &[i32], biases: &[f32], scale: f32, out: &mut [f32]) {
    debug_assert_eq!(sums.len(), biases.len());
    debug_assert_eq!(sums.len(), out.len());
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    if uses_avx512_fma() {
        unsafe {
            avx512::hard_swish6_bias(sums, biases, scale, out);
        }
        return;
    }
    for (j, sum) in sums.iter().enumerate() {
        out[j] = hard_swish6((*sum as f32).mul_add(scale, biases[j]));
    }
}

/// Official sandhi L2: `swish(gate) * id + residual`.
#[inline]
pub(crate) fn swiglu_residual(pre: &[f32], residual: &[f32], out: &mut [f32]) {
    debug_assert_eq!(pre.len(), residual.len() * 2);
    debug_assert_eq!(out.len(), residual.len());
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    if uses_avx512_fma() {
        unsafe {
            avx512::swiglu_residual(pre, residual, out);
        }
        return;
    }
    let n = residual.len();
    for i in 0..n {
        out[i] = hard_swish6(pre[i]).mul_add(pre[i + n], residual[i]);
    }
}

#[inline]
pub(crate) fn clamp01(values: &mut [f32]) {
    for value in values {
        *value = value.clamp(0.0, 1.0);
    }
}

#[inline]
pub(crate) fn square_clamp01(values: &mut [f32]) {
    for value in values {
        let activated = value.clamp(0.0, 1.0);
        *value = activated * activated;
    }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
#[inline(always)]
fn uses_avx512_fma() -> bool {
    #[cfg(all(target_feature = "avx512f", target_feature = "fma"))]
    {
        true
    }
    #[cfg(not(all(target_feature = "avx512f", target_feature = "fma")))]
    {
        static READY: OnceLock<bool> = OnceLock::new();
        *READY.get_or_init(|| {
            std::arch::is_x86_feature_detected!("avx512f")
                && std::arch::is_x86_feature_detected!("fma")
        })
    }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
#[inline(always)]
fn uses_avx512_vnni() -> bool {
    #[cfg(all(
        target_feature = "avx512f",
        target_feature = "avx512bw",
        target_feature = "avx512vnni",
        target_feature = "fma"
    ))]
    {
        true
    }
    #[cfg(not(all(
        target_feature = "avx512f",
        target_feature = "avx512bw",
        target_feature = "avx512vnni",
        target_feature = "fma"
    )))]
    {
        static READY: OnceLock<bool> = OnceLock::new();
        *READY.get_or_init(|| {
            std::arch::is_x86_feature_detected!("avx512f")
                && std::arch::is_x86_feature_detected!("avx512bw")
                && std::arch::is_x86_feature_detected!("avx512vnni")
                && std::arch::is_x86_feature_detected!("fma")
        })
    }
}

type BlockedKernel = fn(&[u8], &[u16], &[i8], &mut [i32]);
type AffineF32Kernel = fn(&[f32], &[f32], &mut [f32]);
type DotF32Kernel = fn(&[f32], &[f32], f32) -> f32;
type FindNnzKernel = fn(&[u8], &mut [u16]) -> usize;

struct KernelDispatch {
    blocked: BlockedKernel,
    affine_f32: AffineF32Kernel,
    dot_f32: DotF32Kernel,
    find_nnz: FindNnzKernel,
}

static KERNEL_DISPATCH: OnceLock<KernelDispatch> = OnceLock::new();

#[inline]
fn kernels() -> &'static KernelDispatch {
    KERNEL_DISPATCH.get_or_init(detect_kernels)
}

fn detect_kernels() -> KernelDispatch {
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    {
        if std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
            && std::arch::is_x86_feature_detected!("avx512vnni")
            && std::arch::is_x86_feature_detected!("fma")
        {
            return KernelDispatch {
                blocked: avx512_blocked,
                affine_f32: avx512_affine_f32,
                dot_f32: avx512_dot_f32,
                find_nnz: avx512_find_nnz,
            };
        }
        if std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma")
        {
            return KernelDispatch {
                blocked: avx2_blocked,
                affine_f32: avx2_affine_f32,
                dot_f32: avx2_dot_f32,
                find_nnz: avx2_find_nnz,
            };
        }
    }
    KernelDispatch {
        blocked: scalar::blocked,
        affine_f32: scalar::affine_f32,
        dot_f32: scalar::dot_f32,
        find_nnz: find_nnz_scalar,
    }
}

mod scalar {
    pub(super) fn blocked(input: &[u8], nnz: &[u16], weights: &[i8], output: &mut [i32]) {
        let outputs = output.len();
        for &block in nnz {
            let input_base = usize::from(block) * 4;
            let weight_base = usize::from(block) * outputs * 4;
            for (output_index, value) in output.iter_mut().enumerate() {
                let row = weight_base + output_index * 4;
                *value += i32::from(input[input_base]) * i32::from(weights[row])
                    + i32::from(input[input_base + 1]) * i32::from(weights[row + 1])
                    + i32::from(input[input_base + 2]) * i32::from(weights[row + 2])
                    + i32::from(input[input_base + 3]) * i32::from(weights[row + 3]);
            }
        }
    }

    pub(super) fn affine_f32(inputs: &[f32], weights: &[f32], outputs: &mut [f32]) {
        let width = outputs.len();
        for (index, &input) in inputs.iter().enumerate() {
            let row = &weights[index * width..index * width + width];
            for (output, &weight) in outputs.iter_mut().zip(row) {
                *output = input.mul_add(weight, *output);
            }
        }
    }

    pub(super) fn dot_f32(inputs: &[f32], weights: &[f32], bias: f32) -> f32 {
        inputs
            .iter()
            .zip(weights)
            .fold(bias, |acc, (&input, &weight)| input.mul_add(weight, acc))
    }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx512_find_nnz(ft_out: &[u8], indexes: &mut [u16]) -> usize {
    unsafe { avx512::find_nnz(ft_out, indexes) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx2_find_nnz(ft_out: &[u8], indexes: &mut [u16]) -> usize {
    unsafe { avx2::find_nnz(ft_out, indexes) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx512_blocked(input: &[u8], nnz: &[u16], weights: &[i8], output: &mut [i32]) {
    if output.len().is_multiple_of(16) && output.len() <= 64 {
        unsafe { avx512::blocked(input, nnz, weights, output) }
    } else if output.len().is_multiple_of(8) && output.len() <= 64 {
        unsafe { avx2::blocked(input, nnz, weights, output) }
    } else {
        scalar::blocked(input, nnz, weights, output);
    }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx512_affine_f32(inputs: &[f32], weights: &[f32], outputs: &mut [f32]) {
    unsafe { avx512::affine_f32(inputs, weights, outputs) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx512_dot_f32(inputs: &[f32], weights: &[f32], bias: f32) -> f32 {
    unsafe { avx512::dot_f32(inputs, weights, bias) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx2_blocked(input: &[u8], nnz: &[u16], weights: &[i8], output: &mut [i32]) {
    if output.len().is_multiple_of(8) && output.len() <= 64 {
        unsafe { avx2::blocked(input, nnz, weights, output) }
    } else {
        scalar::blocked(input, nnz, weights, output);
    }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx2_affine_f32(inputs: &[f32], weights: &[f32], outputs: &mut [f32]) {
    unsafe { avx2::affine_f32(inputs, weights, outputs) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn avx2_dot_f32(inputs: &[f32], weights: &[f32], bias: f32) -> f32 {
    unsafe { avx2::dot_f32(inputs, weights, bias) }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
mod avx2 {
    #![allow(clippy::undocumented_unsafe_blocks)]

    use std::arch::x86_64::*;

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn find_nnz(ft_out: &[u8], indexes: &mut [u16]) -> usize {
        unsafe {
            let blocks = ft_out.len() / super::NNZ_BLOCK;
            let packed = ft_out.as_ptr().cast::<u32>();
            let zero = _mm256_setzero_si256();
            let mut count = 0;
            let mut block = 0;
            while block + 8 <= blocks {
                let values = _mm256_loadu_si256(packed.add(block).cast());
                let eq_zero = _mm256_cmpeq_epi32(values, zero);
                let mut bits = (!_mm256_movemask_ps(_mm256_castsi256_ps(eq_zero))) as u32 & 0xFF;
                while bits != 0 {
                    let lane = bits.trailing_zeros();
                    indexes[count] = (block as u32 + lane) as u16;
                    count += 1;
                    bits &= bits - 1;
                }
                block += 8;
            }
            while block < blocks {
                if *packed.add(block) != 0 {
                    indexes[count] = block as u16;
                    count += 1;
                }
                block += 1;
            }
            count
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn blocked(input: &[u8], nnz: &[u16], weights: &[i8], output: &mut [i32]) {
        unsafe {
            debug_assert!(output.len().is_multiple_of(8));
            debug_assert!(output.len() <= 64);
            let outputs = output.len();
            let groups = outputs / 8;
            let ones = _mm256_set1_epi16(1);
            let packed = input.as_ptr().cast::<i32>();
            let mut acc = [_mm256_setzero_si256(); 8];
            for &block in nnz {
                let splat = _mm256_set1_epi32(*packed.add(usize::from(block)));
                let weight_base = usize::from(block) * outputs * 4;
                for (group, sum) in acc.iter_mut().enumerate().take(groups) {
                    let row =
                        _mm256_loadu_si256(weights.as_ptr().add(weight_base + group * 32).cast());
                    let pairwise = _mm256_maddubs_epi16(splat, row);
                    *sum = _mm256_add_epi32(*sum, _mm256_madd_epi16(pairwise, ones));
                }
            }
            for (group, sum) in acc.iter().enumerate().take(groups) {
                let dest = output.as_mut_ptr().add(group * 8).cast();
                let current = _mm256_loadu_si256(dest);
                _mm256_storeu_si256(dest, _mm256_add_epi32(current, *sum));
            }
        }
    }

    #[target_feature(enable = "avx2,fma")]
    pub(super) unsafe fn affine_f32(inputs: &[f32], weights: &[f32], outputs: &mut [f32]) {
        unsafe {
            let width = outputs.len();
            if width.is_multiple_of(8) {
                for (index, &input) in inputs.iter().enumerate() {
                    let x = _mm256_set1_ps(input);
                    let row = weights.as_ptr().add(index * width);
                    for out_index in (0..width).step_by(8) {
                        let acc = _mm256_loadu_ps(outputs.as_ptr().add(out_index));
                        let w = _mm256_loadu_ps(row.add(out_index));
                        _mm256_storeu_ps(
                            outputs.as_mut_ptr().add(out_index),
                            _mm256_fmadd_ps(x, w, acc),
                        );
                    }
                }
                return;
            }
            super::scalar::affine_f32(inputs, weights, outputs);
        }
    }

    #[target_feature(enable = "avx2,fma")]
    pub(super) unsafe fn dot_f32(inputs: &[f32], weights: &[f32], bias: f32) -> f32 {
        unsafe {
            if inputs.len().is_multiple_of(8) {
                let mut acc = _mm256_setzero_ps();
                for index in (0..inputs.len()).step_by(8) {
                    let a = _mm256_loadu_ps(inputs.as_ptr().add(index));
                    let b = _mm256_loadu_ps(weights.as_ptr().add(index));
                    acc = _mm256_fmadd_ps(a, b, acc);
                }
                let high = _mm256_extractf128_ps::<1>(acc);
                let low = _mm256_castps256_ps128(acc);
                let sum = _mm_add_ps(low, high);
                let shuffled = _mm_movehdup_ps(sum);
                let sum = _mm_add_ps(sum, shuffled);
                let shuffled = _mm_movehl_ps(shuffled, sum);
                let sum = _mm_add_ss(sum, shuffled);
                return bias + _mm_cvtss_f32(sum);
            }
            super::scalar::dot_f32(inputs, weights, bias)
        }
    }
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
mod avx512 {
    #![allow(clippy::undocumented_unsafe_blocks)]

    use std::arch::x86_64::*;

    #[target_feature(enable = "avx512f")]
    pub(super) unsafe fn find_nnz(ft_out: &[u8], indexes: &mut [u16]) -> usize {
        unsafe {
            let blocks = ft_out.len() / super::NNZ_BLOCK;
            let packed = ft_out.as_ptr().cast::<u32>();
            let zero = _mm512_setzero_si512();
            let mut count = 0;
            let mut block = 0;
            while block + 16 <= blocks {
                let values = _mm512_loadu_si512(packed.add(block).cast());
                let mut bits = u32::from(_mm512_cmpneq_epi32_mask(values, zero));
                while bits != 0 {
                    let lane = bits.trailing_zeros();
                    indexes[count] = (block as u32 + lane) as u16;
                    count += 1;
                    bits &= bits - 1;
                }
                block += 16;
            }
            while block < blocks {
                if *packed.add(block) != 0 {
                    indexes[count] = block as u16;
                    count += 1;
                }
                block += 1;
            }
            count
        }
    }

    #[target_feature(enable = "avx512f,avx512bw,avx512vnni")]
    pub(super) unsafe fn blocked(input: &[u8], nnz: &[u16], weights: &[i8], output: &mut [i32]) {
        unsafe {
            debug_assert!(output.len().is_multiple_of(16));
            debug_assert!(output.len() <= 64);
            let outputs = output.len();
            let groups = outputs / 16;
            let packed = input.as_ptr().cast::<i32>();
            let mut acc = [_mm512_setzero_si512(); 4];
            let mut aux = [_mm512_setzero_si512(); 4];
            let tail = nnz.len() - (nnz.len() % 4);
            for chunk in nnz[..tail].chunks_exact(4) {
                let a = _mm512_set1_epi32(*packed.add(usize::from(chunk[0])));
                let b = _mm512_set1_epi32(*packed.add(usize::from(chunk[1])));
                let c = _mm512_set1_epi32(*packed.add(usize::from(chunk[2])));
                let d = _mm512_set1_epi32(*packed.add(usize::from(chunk[3])));
                let wa = usize::from(chunk[0]) * outputs * 4;
                let wb = usize::from(chunk[1]) * outputs * 4;
                let wc = usize::from(chunk[2]) * outputs * 4;
                let wd = usize::from(chunk[3]) * outputs * 4;
                for group in 0..groups {
                    let off = group * 64;
                    acc[group] = _mm512_dpbusd_epi32(
                        acc[group],
                        a,
                        _mm512_loadu_si512(weights.as_ptr().add(wa + off).cast()),
                    );
                    acc[group] = _mm512_dpbusd_epi32(
                        acc[group],
                        b,
                        _mm512_loadu_si512(weights.as_ptr().add(wb + off).cast()),
                    );
                    aux[group] = _mm512_dpbusd_epi32(
                        aux[group],
                        c,
                        _mm512_loadu_si512(weights.as_ptr().add(wc + off).cast()),
                    );
                    aux[group] = _mm512_dpbusd_epi32(
                        aux[group],
                        d,
                        _mm512_loadu_si512(weights.as_ptr().add(wd + off).cast()),
                    );
                }
            }
            for &block in &nnz[tail..] {
                let splat = _mm512_set1_epi32(*packed.add(usize::from(block)));
                let weight_base = usize::from(block) * outputs * 4;
                for (group, sum) in acc.iter_mut().enumerate().take(groups) {
                    let row =
                        _mm512_loadu_si512(weights.as_ptr().add(weight_base + group * 64).cast());
                    *sum = _mm512_dpbusd_epi32(*sum, splat, row);
                }
            }
            for (group, sum) in acc.iter_mut().enumerate().take(groups) {
                *sum = _mm512_add_epi32(*sum, aux[group]);
            }
            for (group, sum) in acc.iter().enumerate().take(groups) {
                let dest = output.as_mut_ptr().add(group * 16).cast();
                let current = _mm512_loadu_si512(dest);
                _mm512_storeu_si512(dest, _mm512_add_epi32(current, *sum));
            }
        }
    }

    #[target_feature(enable = "avx512f,fma")]
    pub(super) unsafe fn hard_swish6_bias(
        sums: &[i32],
        biases: &[f32],
        scale: f32,
        out: &mut [f32],
    ) {
        unsafe {
            let k = _mm512_set1_ps(super::SWISH_K);
            let inv_k = _mm512_set1_ps(1.0 / super::SWISH_K);
            let half_k = _mm512_set1_ps(super::SWISH_K * 0.5);
            let zero = _mm512_setzero_ps();
            let mul = _mm512_set1_ps(scale);
            let mut i = 0;
            while i + 16 <= sums.len() {
                let unscaled = _mm512_cvtepi32_ps(_mm512_loadu_si512(sums.as_ptr().add(i).cast()));
                let bias = _mm512_loadu_ps(biases.as_ptr().add(i));
                let preact = _mm512_fmadd_ps(unscaled, mul, bias);
                let gate = _mm512_min_ps(_mm512_max_ps(_mm512_add_ps(preact, half_k), zero), k);
                _mm512_storeu_ps(
                    out.as_mut_ptr().add(i),
                    _mm512_mul_ps(_mm512_mul_ps(preact, gate), inv_k),
                );
                i += 16;
            }
            while i < sums.len() {
                out[i] = super::hard_swish6((sums[i] as f32).mul_add(scale, biases[i]));
                i += 1;
            }
        }
    }

    #[target_feature(enable = "avx512f,fma")]
    pub(super) unsafe fn swiglu_residual(pre: &[f32], residual: &[f32], out: &mut [f32]) {
        unsafe {
            let n = residual.len();
            let k = _mm512_set1_ps(super::SWISH_K);
            let inv_k = _mm512_set1_ps(1.0 / super::SWISH_K);
            let half_k = _mm512_set1_ps(super::SWISH_K * 0.5);
            let zero = _mm512_setzero_ps();
            let mut i = 0;
            while i + 16 <= n {
                let gate_pre = _mm512_loadu_ps(pre.as_ptr().add(i));
                let id_pre = _mm512_loadu_ps(pre.as_ptr().add(i + n));
                let skip = _mm512_loadu_ps(residual.as_ptr().add(i));
                let clamped =
                    _mm512_min_ps(_mm512_max_ps(_mm512_add_ps(gate_pre, half_k), zero), k);
                let swish = _mm512_mul_ps(_mm512_mul_ps(gate_pre, clamped), inv_k);
                _mm512_storeu_ps(
                    out.as_mut_ptr().add(i),
                    _mm512_fmadd_ps(swish, id_pre, skip),
                );
                i += 16;
            }
            while i < n {
                out[i] = super::hard_swish6(pre[i]).mul_add(pre[i + n], residual[i]);
                i += 1;
            }
        }
    }

    #[target_feature(enable = "avx512f,avx2,fma")]
    pub(super) unsafe fn affine_f32(inputs: &[f32], weights: &[f32], outputs: &mut [f32]) {
        unsafe {
            let width = outputs.len();
            if width.is_multiple_of(16) {
                for (index, &input) in inputs.iter().enumerate() {
                    let x = _mm512_set1_ps(input);
                    let row = weights.as_ptr().add(index * width);
                    for out_index in (0..width).step_by(16) {
                        let acc = _mm512_loadu_ps(outputs.as_ptr().add(out_index));
                        let w = _mm512_loadu_ps(row.add(out_index));
                        _mm512_storeu_ps(
                            outputs.as_mut_ptr().add(out_index),
                            _mm512_fmadd_ps(x, w, acc),
                        );
                    }
                }
                return;
            }
            if width.is_multiple_of(8) {
                super::avx2::affine_f32(inputs, weights, outputs);
                return;
            }
            super::scalar::affine_f32(inputs, weights, outputs);
        }
    }

    #[target_feature(enable = "avx512f,avx2,fma")]
    pub(super) unsafe fn dot_f32(inputs: &[f32], weights: &[f32], bias: f32) -> f32 {
        unsafe {
            if inputs.len().is_multiple_of(16) {
                let mut acc = _mm512_setzero_ps();
                for index in (0..inputs.len()).step_by(16) {
                    let a = _mm512_loadu_ps(inputs.as_ptr().add(index));
                    let b = _mm512_loadu_ps(weights.as_ptr().add(index));
                    acc = _mm512_fmadd_ps(a, b, acc);
                }
                return bias + _mm512_reduce_add_ps(acc);
            }
            if inputs.len().is_multiple_of(8) {
                return super::avx2::dot_f32(inputs, weights, bias);
            }
            super::scalar::dot_f32(inputs, weights, bias)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align64_box_keeps_payload_and_alignment() {
        let src = (0..1024u16).map(|v| v as i16).collect::<Vec<_>>();
        let boxed = Align64Box::from_slice(&src);
        assert_eq!(&*boxed, src.as_slice());
        assert_eq!(boxed.as_ptr() as usize % 64, 0);
    }

    #[test]
    fn large_align64_box_uses_two_megabyte_alignment() {
        let src = vec![7u8; 2 * 1024 * 1024];
        let boxed = Align64Box::from_slice(&src);
        assert_eq!(&*boxed, src.as_slice());
        assert_eq!(boxed.as_ptr() as usize % (2 * 1024 * 1024), 0);
    }

    #[test]
    fn find_nnz_skips_zero_blocks() {
        let mut input = [0u8; 32];
        input[4] = 3;
        input[7] = 9;
        input[20] = 1;
        let mut indexes = [0u16; 8];
        let count = find_nnz(&input, &mut indexes);
        assert_eq!(&indexes[..count], &[1, 5]);
    }

    #[test]
    fn sparse_affine_matches_dense_on_patterned_input() {
        const INPUT: usize = 64;
        const OUTPUT: usize = 8;
        let input = (0..INPUT)
            .map(|index| {
                if index % 5 == 0 {
                    (index % 90) as u8
                } else {
                    0
                }
            })
            .collect::<Vec<_>>();
        let weights = (0..INPUT * OUTPUT)
            .map(|index| (index as i8).wrapping_mul(7).wrapping_sub(11))
            .collect::<Vec<_>>();
        let mut dense = [4_i32; OUTPUT];
        let mut sparse = dense;
        super::super::stockfish_simd::affine(&input, &weights, &mut dense);
        affine_sparse(&input, &weights, &mut sparse);
        assert_eq!(sparse, dense);
    }

    #[test]
    fn packed_blocked_matches_dense_for_sandhi_width() {
        const INPUT: usize = 1024;
        const OUTPUT: usize = 32;
        let input = (0..INPUT)
            .map(|index| {
                if index % 6 == 0 {
                    ((index * 5) % 180) as u8
                } else {
                    0
                }
            })
            .collect::<Vec<_>>();
        let weights = (0..INPUT * OUTPUT)
            .map(|index| (index as i8).wrapping_mul(9).wrapping_sub(17))
            .collect::<Vec<_>>();
        let packed = pack_nnz_layout(&weights, INPUT, OUTPUT);
        let mut dense = [2_i32; OUTPUT];
        let mut blocked = dense;
        super::super::stockfish_simd::affine(&input, &weights, &mut dense);
        affine_sparse_packed(&input, &packed, &mut blocked);
        assert_eq!(blocked, dense);
    }

    #[test]
    fn sparse_affine_matches_dense_for_obsidian_width() {
        const INPUT: usize = 1536;
        const OUTPUT: usize = 16;
        let input = (0..INPUT)
            .map(|index| {
                if index % 7 == 0 {
                    ((index * 3) % 200) as u8
                } else {
                    0
                }
            })
            .collect::<Vec<_>>();
        let weights = (0..INPUT * OUTPUT)
            .map(|index| (index as i8).wrapping_mul(13).wrapping_sub(40))
            .collect::<Vec<_>>();
        let mut dense = [0_i32; OUTPUT];
        let mut sparse = [0_i32; OUTPUT];
        super::super::stockfish_simd::affine(&input, &weights, &mut dense);
        affine_sparse(&input, &weights, &mut sparse);
        assert_eq!(sparse, dense);
    }

    #[test]
    fn affine_f32_matches_scalar_for_l2_width() {
        let inputs = [0.25f32, -0.5, 0.75, 1.0, 0.0, 0.125, -0.25, 0.5];
        let weights = (0..8 * 16)
            .map(|index| (index as f32) * 0.01 - 0.2)
            .collect::<Vec<_>>();
        let biases = [0.1f32; 16];
        let mut expected = [0.0f32; 16];
        expected.copy_from_slice(&biases);
        scalar::affine_f32(&inputs, &weights, &mut expected);
        let mut actual = [0.0f32; 16];
        affine_f32(&inputs, &weights, &biases, &mut actual);
        for (got, want) in actual.iter().zip(expected) {
            assert!((got - want).abs() < 1e-5, "{got} vs {want}");
        }
    }

    #[test]
    fn pack_f32_buckets_groups_one_bucket_contiguously() {
        let src = [
            1.0f32, 2.0, // input 0, bucket 0..1, output 0
            3.0, 4.0, // input 1
        ];
        // inputs=2, buckets=2, outputs=1 stored as [input][bucket][output]
        let packed = pack_f32_buckets(&src, 2, 2, 1);
        assert_eq!(&*packed, &[1.0, 3.0, 2.0, 4.0]);
    }

    #[test]
    fn clamp01_and_square_clamp_keep_unit_interval() {
        let mut values = [-0.5f32, 0.25, 1.5];
        clamp01(&mut values);
        assert_eq!(values, [0.0, 0.25, 1.0]);
        let mut squares = [-0.5f32, 0.5, 2.0];
        square_clamp01(&mut squares);
        assert_eq!(squares, [0.0, 0.25, 1.0]);
    }

    #[test]
    fn hard_swish6_bias_matches_scalar_for_sandhi_l1() {
        let sums: [i32; 32] = core::array::from_fn(|i| (i as i32) * 17 - 80);
        let biases: [f32; 32] = core::array::from_fn(|i| (i as f32) * 0.03 - 0.4);
        let scale = 1.0 / 4096.0;
        let mut expected = [0.0f32; 32];
        for (j, sum) in sums.iter().enumerate() {
            expected[j] = hard_swish6((*sum as f32).mul_add(scale, biases[j]));
        }
        let mut actual = [0.0f32; 32];
        hard_swish6_bias(&sums, &biases, scale, &mut actual);
        for (got, want) in actual.iter().zip(expected) {
            assert!((got - want).abs() < 1e-5, "{got} vs {want}");
        }
    }

    #[test]
    fn compile_time_avx512_gates_match_runtime_detection() {
        #[cfg(all(feature = "simd", target_arch = "x86_64"))]
        {
            let fma = std::arch::is_x86_feature_detected!("avx512f")
                && std::arch::is_x86_feature_detected!("fma");
            let vnni = fma
                && std::arch::is_x86_feature_detected!("avx512bw")
                && std::arch::is_x86_feature_detected!("avx512vnni");
            assert_eq!(uses_avx512_fma(), fma);
            assert_eq!(uses_avx512_vnni(), vnni);
        }
    }

    #[test]
    fn swiglu_residual_matches_scalar_for_sandhi_l2() {
        let pre: [f32; 64] = core::array::from_fn(|i| (i as f32) * 0.08 - 2.4);
        let residual: [f32; 32] = core::array::from_fn(|i| (i as f32) * 0.05 - 0.7);
        let mut expected = [0.0f32; 32];
        for i in 0..32 {
            expected[i] = hard_swish6(pre[i]).mul_add(pre[i + 32], residual[i]);
        }
        let mut actual = [0.0f32; 32];
        swiglu_residual(&pre, &residual, &mut actual);
        for (got, want) in actual.iter().zip(expected) {
            assert!((got - want).abs() < 1e-5, "{got} vs {want}");
        }
    }
}
