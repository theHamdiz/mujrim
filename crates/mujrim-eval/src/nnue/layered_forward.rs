//! Shared NNZ-sparse L1 and float L2/L3 kernels for layered adapter nets.
//!
//! Weight layout for L1 stays `[output][input]` so the existing dense affine
//! path remains the bit-exact reference. NNZ skips all-zero 4-byte activation
//! blocks the same way Obsidian / Viridithas / Reckless do.

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

#[inline]
pub(crate) fn find_nnz(ft_out: &[u8], indexes: &mut [u16]) -> usize {
    debug_assert_eq!(ft_out.len() % NNZ_BLOCK, 0);
    debug_assert!(indexes.len() >= ft_out.len() / NNZ_BLOCK);
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
    (kernels().blocked)(input, &nnz[..count], weights, output);
}

#[inline]
pub(crate) fn affine_f32(inputs: &[f32], weights: &[f32], biases: &[f32], outputs: &mut [f32]) {
    debug_assert_eq!(outputs.len(), biases.len());
    debug_assert_eq!(weights.len(), inputs.len() * outputs.len());
    outputs.copy_from_slice(biases);
    (kernels().affine_f32)(inputs, weights, outputs);
}

#[inline]
pub(crate) fn dot_f32(inputs: &[f32], weights: &[f32], bias: f32) -> f32 {
    debug_assert_eq!(inputs.len(), weights.len());
    (kernels().dot_f32)(inputs, weights, bias)
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

type BlockedKernel = fn(&[u8], &[u16], &[i8], &mut [i32]);
type AffineF32Kernel = fn(&[f32], &[f32], &mut [f32]);
type DotF32Kernel = fn(&[f32], &[f32], f32) -> f32;

struct KernelDispatch {
    blocked: BlockedKernel,
    affine_f32: AffineF32Kernel,
    dot_f32: DotF32Kernel,
}

static KERNEL_DISPATCH: OnceLock<KernelDispatch> = OnceLock::new();

#[inline]
fn kernels() -> &'static KernelDispatch {
    KERNEL_DISPATCH.get_or_init(detect_kernels)
}

fn detect_kernels() -> KernelDispatch {
    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    {
        if std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma")
        {
            return KernelDispatch {
                blocked: avx2_blocked,
                affine_f32: avx2_affine_f32,
                dot_f32: avx2_dot_f32,
            };
        }
    }
    KernelDispatch {
        blocked: scalar::blocked,
        affine_f32: scalar::affine_f32,
        dot_f32: scalar::dot_f32,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
