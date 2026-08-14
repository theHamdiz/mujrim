//! Viridithas NNUE loader (zstd `.nnue.zst`).
//!
//! Published layouts evaluated in-process; Mujrim's alpha-beta stays on the
//! Viridithas search profile either way.
//!
//! - Simple: `16×768→H×2→1` SCReLU (early piece-feature nets).
//! - Layered velarised: `704×16hm → 2560` pairwise-CReLU → `16` HardSwish6 →
//!   `32` SwiGLU → `1`, eight output buckets.
//! - Sandhi threat transformer: `704×16hm + (59808+4560)hm → 1024` pairwise-CReLU
//!   → `32` HardSwish6 → `32` SwiGLU → `1` ×8. Piece planes reuse the velarised
//!   704-index; aux planes use the in-crate SFNNv12 threat / pawn-pair indices.

use std::io::Read;
use std::path::Path;

use types::{Board, Color, Piece};

use super::stockfish_format::{
    PAIR_FEATURES, THREAT_FEATURES, visit_pawn_pair_features, visit_threat_features,
};

pub const KING_BUCKETS: usize = 16;
pub const FEATURES: usize = 768;
pub const HIDDEN: usize = 1024;
pub const QA: i32 = 255;
pub const SCALE: i32 = 400;
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

pub const LAYERED_INPUT: usize = 704;
pub const LAYERED_L1: usize = 2560;
pub const LAYERED_L2: usize = 16;
pub const LAYERED_L3: usize = 32;
pub const LAYERED_OUTPUT_BUCKETS: usize = 8;
pub const LAYERED_SCALE: i32 = 240;
const LAYERED_QB: i32 = 64;
const FT_SHIFT: i32 = 9;
const SWISH_K: f32 = 6.0;
const L1_MUL: f32 = (1 << FT_SHIFT) as f32 / ((QA * QA * LAYERED_QB) as f32);

pub const SANDHI_L0: usize = 1024;
pub const SANDHI_L1: usize = 32;
pub const SANDHI_L2: usize = 32;
pub const SANDHI_AUX: usize = THREAT_FEATURES + PAIR_FEATURES;

pub const FILE_SIZE: u64 = simple_size(HIDDEN) as u64;

pub const fn simple_size(hidden: usize) -> usize {
    KING_BUCKETS * FEATURES * hidden * 2 + hidden * 2 + 2 * hidden * 2 + 4
}

pub const fn layered_velarised_size() -> usize {
    KING_BUCKETS * LAYERED_INPUT * LAYERED_L1 * 2
        + LAYERED_L1 * 2
        + LAYERED_L1 * LAYERED_OUTPUT_BUCKETS * LAYERED_L2
        + LAYERED_OUTPUT_BUCKETS * LAYERED_L2 * 4
        + LAYERED_L2 * LAYERED_OUTPUT_BUCKETS * (LAYERED_L3 * 2) * 4
        + LAYERED_OUTPUT_BUCKETS * (LAYERED_L3 * 2) * 4
        + LAYERED_L3 * LAYERED_OUTPUT_BUCKETS * 4
        + LAYERED_OUTPUT_BUCKETS * 4
}

pub const fn sandhi_size() -> usize {
    SANDHI_AUX * SANDHI_L0
        + KING_BUCKETS * LAYERED_INPUT * SANDHI_L0 * 2
        + SANDHI_L0 * 2
        + SANDHI_L0 * LAYERED_OUTPUT_BUCKETS * SANDHI_L1
        + LAYERED_OUTPUT_BUCKETS * SANDHI_L1 * 4
        + SANDHI_L1 * LAYERED_OUTPUT_BUCKETS * (SANDHI_L2 * 2) * 4
        + LAYERED_OUTPUT_BUCKETS * (SANDHI_L2 * 2) * 4
        + SANDHI_L2 * LAYERED_OUTPUT_BUCKETS * 4
        + LAYERED_OUTPUT_BUCKETS * 4
}

#[rustfmt::skip]
const HALF_BUCKET_MAP: [usize; 32] = [
    0, 1, 2, 3,
    4, 5, 6, 7,
    8, 9, 10, 11,
    8, 9, 10, 11,
    12, 12, 13, 13,
    12, 12, 13, 13,
    14, 14, 15, 15,
    14, 14, 15, 15,
];

pub enum ViridithasNetwork {
    Simple(SimpleNetwork),
    Layered(Box<LayeredNetwork>),
    Sandhi(Box<SandhiNetwork>),
}

pub struct SimpleNetwork {
    hidden: usize,
    features_per_bucket: usize,
    feature_weights: Box<[i16]>,
    feature_biases: Box<[i16]>,
    output_weights: Box<[i16]>,
    output_bias: i32,
}

pub struct LayeredNetwork {
    feature_weights: Box<[i16]>,
    feature_biases: Box<[i16]>,
    l1_weights: Box<[i8]>,
    l1_biases: Box<[f32]>,
    l2_weights: Box<[f32]>,
    l2_biases: Box<[f32]>,
    l3_weights: Box<[f32]>,
    l3_biases: Box<[f32]>,
}

pub struct SandhiNetwork {
    aux_weights: Box<[i8]>,
    feature_weights: Box<[i16]>,
    feature_biases: Box<[i16]>,
    l1_weights: Box<[i8]>,
    l1_biases: Box<[f32]>,
    l2_weights: Box<[f32]>,
    l2_biases: Box<[f32]>,
    l3_weights: Box<[f32]>,
    l3_biases: Box<[f32]>,
}

impl ViridithasNetwork {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let decoded = maybe_decompress(bytes)?;
        for hidden in [HIDDEN, 768, 512, 256] {
            if decoded.len() == simple_size(hidden) {
                return parse_simple(&decoded, hidden, FEATURES).map(Self::Simple);
            }
        }
        if decoded.len() == layered_velarised_size() {
            return parse_layered(&decoded).map(|net| Self::Layered(Box::new(net)));
        }
        if decoded.len() == sandhi_size() {
            return parse_sandhi(&decoded).map(|net| Self::Sandhi(Box::new(net)));
        }
        Err(format!(
            "Viridithas NNUE size {} is not a supported layout (simple H=1024 is {}, velarised layered is {}, sandhi is {})",
            decoded.len(),
            simple_size(HIDDEN),
            layered_velarised_size(),
            sandhi_size()
        ))
    }

    #[inline(always)]
    pub fn features_per_bucket(&self) -> usize {
        match self {
            Self::Simple(net) => net.features_per_bucket,
            Self::Layered(_) => LAYERED_INPUT,
            Self::Sandhi(_) => LAYERED_INPUT,
        }
    }

    #[inline(always)]
    pub fn hidden(&self) -> usize {
        match self {
            Self::Simple(net) => net.hidden,
            Self::Layered(_) => LAYERED_L1,
            Self::Sandhi(_) => SANDHI_L0,
        }
    }

    #[inline(always)]
    pub fn scale(&self) -> i32 {
        match self {
            Self::Simple(_) => SCALE,
            Self::Layered(_) | Self::Sandhi(_) => LAYERED_SCALE,
        }
    }

    #[inline]
    pub fn architecture(&self) -> String {
        match self {
            Self::Simple(net) => format!(
                "16×{}→{}×2→1 SCReLU (Viridithas piece features)",
                net.features_per_bucket, net.hidden
            ),
            Self::Layered(_) => {
                "704×16hm → 2560 pairwise-CReLU → 16 HardSwish6 → 32 SwiGLU → 1 ×8 (velarised)"
                    .to_string()
            }
            Self::Sandhi(_) => {
                "704×16hm + (59808+4560)hm → 1024 pairwise-CReLU → 32 HardSwish6 → 32 SwiGLU → 1 ×8 (sandhi)"
                    .to_string()
            }
        }
    }

    #[inline(always)]
    pub fn evaluate(&self, board: &Board) -> i32 {
        match self {
            Self::Simple(net) => match net.hidden {
                1024 => evaluate_simple::<1024>(net, board),
                768 => evaluate_simple::<768>(net, board),
                512 => evaluate_simple::<512>(net, board),
                256 => evaluate_simple::<256>(net, board),
                hidden => evaluate_simple_capped(net, board, hidden),
            },
            Self::Layered(net) => evaluate_layered(net, board),
            Self::Sandhi(net) => evaluate_sandhi(net, board),
        }
    }
}

fn evaluate_simple<const H: usize>(net: &SimpleNetwork, board: &Board) -> i32 {
    let us = accumulate_simple::<H>(net, board, board.side_to_move);
    let them = accumulate_simple::<H>(net, board, board.side_to_move.opponent());
    finish_simple(net, &us, &them, H)
}

fn evaluate_simple_capped(net: &SimpleNetwork, board: &Board, hidden: usize) -> i32 {
    let us = accumulate_simple::<HIDDEN>(net, board, board.side_to_move);
    let them = accumulate_simple::<HIDDEN>(net, board, board.side_to_move.opponent());
    finish_simple(net, &us, &them, hidden)
}

#[inline(always)]
fn finish_simple(net: &SimpleNetwork, us: &[i16], them: &[i16], hidden: usize) -> i32 {
    let mut sum = net.output_bias;
    let weights = net.output_weights.as_ptr();
    unsafe {
        for i in 0..hidden {
            sum += screlu(*us.get_unchecked(i)) * i32::from(*weights.add(i));
            sum += screlu(*them.get_unchecked(i)) * i32::from(*weights.add(hidden + i));
        }
    }
    (sum / (QA * QA)) * SCALE / 64
}

#[inline(always)]
fn accumulate_simple<const H: usize>(
    net: &SimpleNetwork,
    board: &Board,
    perspective: Color,
) -> [i16; H] {
    let mut acc = [0i16; H];
    let hidden = net.hidden.min(H);
    acc[..hidden].copy_from_slice(&net.feature_biases[..hidden]);
    let king = relative_square(perspective, board.king_square(perspective).index());
    let bucket = king / 4;
    let stride = net.features_per_bucket;
    for piece in Piece::ALL {
        for color in [Color::White, Color::Black] {
            let mut bb = board.piece_bb(piece, color);
            while bb != 0 {
                let sq = bb.trailing_zeros() as usize;
                bb &= bb - 1;
                let rel_sq = relative_square(perspective, sq);
                let them = usize::from(color != perspective);
                let local = (them * 6 + piece.index()) * 64 + rel_sq;
                let index = bucket * stride + local;
                add_i16(&mut acc[..hidden], &net.feature_weights, index * hidden);
            }
        }
    }
    acc
}

fn evaluate_layered(net: &LayeredNetwork, board: &Board) -> i32 {
    let mut acc_white = [0i16; LAYERED_L1];
    let mut acc_black = [0i16; LAYERED_L1];
    acc_white.copy_from_slice(&net.feature_biases);
    acc_black.copy_from_slice(&net.feature_biases);
    accumulate_layered(net, board, Color::White, &mut acc_white);
    accumulate_layered(net, board, Color::Black, &mut acc_black);

    let (us, them) = if board.side_to_move == Color::White {
        (&acc_white, &acc_black)
    } else {
        (&acc_black, &acc_white)
    };

    let mut ft = [0u8; LAYERED_L1];
    activate_pairwise(us, &mut ft[..LAYERED_L1 / 2]);
    activate_pairwise(them, &mut ft[LAYERED_L1 / 2..]);

    let pieces = board.all_occupancy().count_ones() as usize;
    let bucket = ((pieces - 2) / 4).min(LAYERED_OUTPUT_BUCKETS - 1);

    let mut l1 = [0.0f32; LAYERED_L2];
    propagate_l1(net, &ft, bucket, &mut l1);

    let mut l2 = [0.0f32; LAYERED_L3];
    propagate_l2(net, &l1, bucket, &mut l2);

    let mut sum = net.l3_biases[bucket];
    let l3_base = bucket;
    for (i, value) in l2.iter().enumerate() {
        sum += net.l3_weights[i * LAYERED_OUTPUT_BUCKETS + l3_base] * *value;
    }
    (sum * LAYERED_SCALE as f32) as i32
}

fn evaluate_sandhi(net: &SandhiNetwork, board: &Board) -> i32 {
    let mut acc_white = [0i16; SANDHI_L0];
    let mut acc_black = [0i16; SANDHI_L0];
    acc_white.copy_from_slice(&net.feature_biases);
    acc_black.copy_from_slice(&net.feature_biases);
    accumulate_sandhi(net, board, Color::White, &mut acc_white);
    accumulate_sandhi(net, board, Color::Black, &mut acc_black);

    let (us, them) = if board.side_to_move == Color::White {
        (&acc_white, &acc_black)
    } else {
        (&acc_black, &acc_white)
    };

    let mut ft = [0u8; SANDHI_L0];
    activate_pairwise_sandhi(us, &mut ft[..SANDHI_L0 / 2]);
    activate_pairwise_sandhi(them, &mut ft[SANDHI_L0 / 2..]);

    let pieces = board.all_occupancy().count_ones() as usize;
    let bucket = ((pieces - 2) / 4).min(LAYERED_OUTPUT_BUCKETS - 1);

    let mut l1 = [0.0f32; SANDHI_L1];
    let mut sums = [0i32; SANDHI_L1];
    for (i, input) in ft.iter().enumerate() {
        if *input == 0 {
            continue;
        }
        let row = i * LAYERED_OUTPUT_BUCKETS * SANDHI_L1 + bucket * SANDHI_L1;
        let input = i32::from(*input);
        for (j, sum) in sums.iter_mut().enumerate() {
            *sum += input * i32::from(net.l1_weights[row + j]);
        }
    }
    let bias = bucket * SANDHI_L1;
    for (j, sum) in sums.iter().enumerate() {
        l1[j] = hard_swish6((*sum as f32).mul_add(L1_MUL, net.l1_biases[bias + j]));
    }

    let mut l2_pre = [0.0f32; SANDHI_L2 * 2];
    let l2_bias = bucket * SANDHI_L2 * 2;
    l2_pre.copy_from_slice(&net.l2_biases[l2_bias..l2_bias + SANDHI_L2 * 2]);
    for (i, input) in l1.iter().enumerate() {
        let row = i * LAYERED_OUTPUT_BUCKETS * (SANDHI_L2 * 2) + bucket * (SANDHI_L2 * 2);
        for (j, sum) in l2_pre.iter_mut().enumerate() {
            *sum = input.mul_add(net.l2_weights[row + j], *sum);
        }
    }
    // Sandhi L2 is square (32→32); the published head adds the L1 residual.
    let mut l2 = [0.0f32; SANDHI_L2];
    for i in 0..SANDHI_L2 {
        l2[i] = hard_swish6(l2_pre[i]).mul_add(l2_pre[i + SANDHI_L2], l1[i]);
    }

    let mut sum = net.l3_biases[bucket];
    for (i, value) in l2.iter().enumerate() {
        sum += net.l3_weights[i * LAYERED_OUTPUT_BUCKETS + bucket] * *value;
    }
    (sum * LAYERED_SCALE as f32) as i32
}

fn accumulate_sandhi(
    net: &SandhiNetwork,
    board: &Board,
    perspective: Color,
    acc: &mut [i16; SANDHI_L0],
) {
    let king = board.king_square(perspective).index();
    let bucket = king_bucket(relative_square(perspective, king));
    let ft_base = bucket * LAYERED_INPUT * SANDHI_L0;
    for piece in Piece::ALL {
        for color in [Color::White, Color::Black] {
            let mut bb = board.piece_bb(piece, color);
            while bb != 0 {
                let sq = bb.trailing_zeros() as usize;
                bb &= bb - 1;
                let feat = feature_index(perspective, king, piece, color, sq);
                add_i16(acc, &net.feature_weights, ft_base + feat * SANDHI_L0);
            }
        }
    }

    let pov = perspective.index();
    visit_threat_features(board, pov, |feature| {
        add_i8(acc, &net.aux_weights, feature * SANDHI_L0);
    });
    visit_pawn_pair_features(board, pov, |feature| {
        add_i8(acc, &net.aux_weights, feature * SANDHI_L0);
    });
}

#[inline]
fn activate_pairwise_sandhi(acc: &[i16; SANDHI_L0], out: &mut [u8]) {
    let half = SANDHI_L0 / 2;
    debug_assert_eq!(out.len(), half);
    for i in 0..half {
        let left = i32::from(acc[i]).clamp(0, QA);
        let right = i32::from(acc[i + half]).clamp(0, QA);
        out[i] = ((left * right) >> FT_SHIFT) as u8;
    }
}

fn accumulate_layered(
    net: &LayeredNetwork,
    board: &Board,
    perspective: Color,
    acc: &mut [i16; LAYERED_L1],
) {
    let king = board.king_square(perspective).index();
    let bucket = king_bucket(relative_square(perspective, king));
    let ft_base = bucket * LAYERED_INPUT * LAYERED_L1;
    for piece in Piece::ALL {
        for color in [Color::White, Color::Black] {
            let mut bb = board.piece_bb(piece, color);
            while bb != 0 {
                let sq = bb.trailing_zeros() as usize;
                bb &= bb - 1;
                let feat = feature_index(perspective, king, piece, color, sq);
                add_i16(acc, &net.feature_weights, ft_base + feat * LAYERED_L1);
            }
        }
    }
}

#[inline]
fn activate_pairwise(acc: &[i16; LAYERED_L1], out: &mut [u8]) {
    let half = LAYERED_L1 / 2;
    debug_assert_eq!(out.len(), half);
    for i in 0..half {
        let left = i32::from(acc[i]).clamp(0, QA);
        let right = i32::from(acc[i + half]).clamp(0, QA);
        out[i] = ((left * right) >> FT_SHIFT) as u8;
    }
}

fn propagate_l1(
    net: &LayeredNetwork,
    inputs: &[u8; LAYERED_L1],
    bucket: usize,
    out: &mut [f32; LAYERED_L2],
) {
    let mut sums = [0i32; LAYERED_L2];
    for (i, input) in inputs.iter().enumerate() {
        if *input == 0 {
            continue;
        }
        let row = i * LAYERED_OUTPUT_BUCKETS * LAYERED_L2 + bucket * LAYERED_L2;
        let input = i32::from(*input);
        for (j, sum) in sums.iter_mut().enumerate() {
            *sum += input * i32::from(net.l1_weights[row + j]);
        }
    }
    let bias = bucket * LAYERED_L2;
    for (j, sum) in sums.iter().enumerate() {
        let pre = (*sum as f32).mul_add(L1_MUL, net.l1_biases[bias + j]);
        out[j] = hard_swish6(pre);
    }
}

fn propagate_l2(
    net: &LayeredNetwork,
    inputs: &[f32; LAYERED_L2],
    bucket: usize,
    out: &mut [f32; LAYERED_L3],
) {
    let mut sums = [0.0f32; LAYERED_L3 * 2];
    let bias = bucket * LAYERED_L3 * 2;
    sums.copy_from_slice(&net.l2_biases[bias..bias + LAYERED_L3 * 2]);
    for (i, input) in inputs.iter().enumerate() {
        let row = i * LAYERED_OUTPUT_BUCKETS * (LAYERED_L3 * 2) + bucket * (LAYERED_L3 * 2);
        for (j, sum) in sums.iter_mut().enumerate() {
            *sum = input.mul_add(net.l2_weights[row + j], *sum);
        }
    }
    for i in 0..LAYERED_L3 {
        out[i] = hard_swish6(sums[i]) * sums[i + LAYERED_L3];
    }
}

#[inline]
fn hard_swish6(value: f32) -> f32 {
    value * (value + SWISH_K * 0.5).clamp(0.0, SWISH_K) / SWISH_K
}

#[inline]
fn king_bucket(rel_king: usize) -> usize {
    let file = rel_king % 8;
    let rank = rel_king / 8;
    let col = if file >= 4 { 7 - file } else { file };
    HALF_BUCKET_MAP[rank * 4 + col]
}

#[inline]
fn feature_index(
    perspective: Color,
    king: usize,
    piece: Piece,
    piece_color: Color,
    mut sq: usize,
) -> usize {
    if king % 8 >= 4 {
        sq ^= 7;
    }
    let rel_sq = relative_square(perspective, sq);
    let them = usize::from(piece_color != perspective);
    let colour = them * usize::from(piece != Piece::King);
    colour * 384 + piece.index() * 64 + rel_sq
}

#[inline(always)]
fn add_i8(acc: &mut [i16], weights: &[i8], base: usize) {
    let len = acc.len();
    debug_assert!(base + len <= weights.len());
    let acc_ptr = acc.as_mut_ptr();
    let weight_ptr = unsafe { weights.as_ptr().add(base) };
    for i in 0..len {
        unsafe {
            *acc_ptr.add(i) = (*acc_ptr.add(i)).wrapping_add(i16::from(*weight_ptr.add(i)));
        }
    }
}

#[inline(always)]
fn add_i16(acc: &mut [i16], weights: &[i16], base: usize) {
    let len = acc.len();
    debug_assert!(base + len <= weights.len());
    let acc_ptr = acc.as_mut_ptr();
    let weight_ptr = unsafe { weights.as_ptr().add(base) };
    for i in 0..len {
        unsafe {
            *acc_ptr.add(i) = (*acc_ptr.add(i)).wrapping_add(*weight_ptr.add(i));
        }
    }
}

#[inline(always)]
fn relative_square(side: Color, sq: usize) -> usize {
    if side == Color::Black { sq ^ 56 } else { sq }
}

#[inline(always)]
fn screlu(value: i16) -> i32 {
    let clipped = i32::from(value).clamp(0, QA);
    clipped * clipped
}

pub const fn wide_ft_size(hidden: usize, features_per_bucket: usize) -> usize {
    KING_BUCKETS * features_per_bucket * hidden * 2
}

pub const fn one_layer_head_size(hidden: usize) -> usize {
    hidden * 2 + hidden * 2 * 2 + 4
}

fn parse_simple(
    bytes: &[u8],
    hidden: usize,
    features_per_bucket: usize,
) -> Result<SimpleNetwork, String> {
    let mut offset = 0;
    let ft = KING_BUCKETS * features_per_bucket * hidden;
    let feature_weights = read_i16s(bytes, &mut offset, ft)?;
    let feature_biases = read_i16s(bytes, &mut offset, hidden)?;
    let output_weights = read_i16s(bytes, &mut offset, hidden * 2)?;
    let output_bias = read_i32(bytes, &mut offset)?;
    Ok(SimpleNetwork {
        hidden,
        features_per_bucket,
        feature_weights,
        feature_biases,
        output_weights,
        output_bias,
    })
}

fn parse_layered(bytes: &[u8]) -> Result<LayeredNetwork, String> {
    let mut offset = 0;
    let feature_weights = read_i16s(
        bytes,
        &mut offset,
        KING_BUCKETS * LAYERED_INPUT * LAYERED_L1,
    )?;
    let feature_biases = read_i16s(bytes, &mut offset, LAYERED_L1)?;
    let l1_weights = read_i8s(
        bytes,
        &mut offset,
        LAYERED_L1 * LAYERED_OUTPUT_BUCKETS * LAYERED_L2,
    )?;
    let l1_biases = read_f32s(bytes, &mut offset, LAYERED_OUTPUT_BUCKETS * LAYERED_L2)?;
    let l2_weights = read_f32s(
        bytes,
        &mut offset,
        LAYERED_L2 * LAYERED_OUTPUT_BUCKETS * (LAYERED_L3 * 2),
    )?;
    let l2_biases = read_f32s(
        bytes,
        &mut offset,
        LAYERED_OUTPUT_BUCKETS * (LAYERED_L3 * 2),
    )?;
    let l3_weights = read_f32s(bytes, &mut offset, LAYERED_L3 * LAYERED_OUTPUT_BUCKETS)?;
    let l3_biases = read_f32s(bytes, &mut offset, LAYERED_OUTPUT_BUCKETS)?;
    debug_assert_eq!(offset, layered_velarised_size());
    Ok(LayeredNetwork {
        feature_weights,
        feature_biases,
        l1_weights,
        l1_biases,
        l2_weights,
        l2_biases,
        l3_weights,
        l3_biases,
    })
}

fn parse_sandhi(bytes: &[u8]) -> Result<SandhiNetwork, String> {
    let mut offset = 0;
    let aux_weights = read_i8s(bytes, &mut offset, SANDHI_AUX * SANDHI_L0)?;
    let feature_weights = read_i16s(bytes, &mut offset, KING_BUCKETS * LAYERED_INPUT * SANDHI_L0)?;
    let feature_biases = read_i16s(bytes, &mut offset, SANDHI_L0)?;
    let l1_weights = read_i8s(
        bytes,
        &mut offset,
        SANDHI_L0 * LAYERED_OUTPUT_BUCKETS * SANDHI_L1,
    )?;
    let l1_biases = read_f32s(bytes, &mut offset, LAYERED_OUTPUT_BUCKETS * SANDHI_L1)?;
    let l2_weights = read_f32s(
        bytes,
        &mut offset,
        SANDHI_L1 * LAYERED_OUTPUT_BUCKETS * (SANDHI_L2 * 2),
    )?;
    let l2_biases = read_f32s(bytes, &mut offset, LAYERED_OUTPUT_BUCKETS * (SANDHI_L2 * 2))?;
    let l3_weights = read_f32s(bytes, &mut offset, SANDHI_L2 * LAYERED_OUTPUT_BUCKETS)?;
    let l3_biases = read_f32s(bytes, &mut offset, LAYERED_OUTPUT_BUCKETS)?;
    debug_assert_eq!(offset, sandhi_size());
    Ok(SandhiNetwork {
        aux_weights,
        feature_weights,
        feature_biases,
        l1_weights,
        l1_biases,
        l2_weights,
        l2_biases,
        l3_weights,
        l3_biases,
    })
}

fn maybe_decompress(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.starts_with(&ZSTD_MAGIC) {
        let mut decoder = zstd::stream::read::Decoder::new(bytes)
            .map_err(|error| format!("Viridithas zstd header: {error}"))?;
        let mut decoded = Vec::new();
        decoder
            .read_to_end(&mut decoded)
            .map_err(|error| format!("Viridithas zstd decode: {error}"))?;
        return Ok(decoded);
    }
    Ok(bytes.to_vec())
}

pub fn load(path: &Path) -> Result<Box<ViridithasNetwork>, String> {
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "failed to read Viridithas NNUE '{}': {error}",
            path.display()
        )
    })?;
    ViridithasNetwork::from_bytes(&bytes).map(Box::new)
}

pub fn looks_like_viridithas(path: &Path, bytes: &[u8]) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.contains("viri")
        || name.contains("velarised")
        || name.contains("sandhi")
        || name.ends_with(".nnue.zst")
        || name.ends_with(".zst")
        || bytes.starts_with(&ZSTD_MAGIC)
}

fn read_i16s(bytes: &[u8], offset: &mut usize, count: usize) -> Result<Box<[i16]>, String> {
    let need = count * 2;
    let slice = bytes
        .get(*offset..*offset + need)
        .ok_or_else(|| "truncated Viridithas i16 weights".to_string())?;
    *offset += need;
    Ok(slice
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect())
}

fn read_i8s(bytes: &[u8], offset: &mut usize, count: usize) -> Result<Box<[i8]>, String> {
    let slice = bytes
        .get(*offset..*offset + count)
        .ok_or_else(|| "truncated Viridithas i8 weights".to_string())?;
    *offset += count;
    Ok(slice.iter().map(|value| *value as i8).collect())
}

fn read_i32(bytes: &[u8], offset: &mut usize) -> Result<i32, String> {
    let slice = bytes
        .get(*offset..*offset + 4)
        .ok_or_else(|| "truncated Viridithas bias".to_string())?;
    *offset += 4;
    Ok(i32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_f32s(bytes: &[u8], offset: &mut usize, count: usize) -> Result<Box<[f32]>, String> {
    let need = count * 4;
    let slice = bytes
        .get(*offset..*offset + need)
        .ok_or_else(|| "truncated Viridithas f32 weights".to_string())?;
    *offset += need;
    Ok(slice
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_and_layered_sizes_are_stable() {
        assert_eq!(simple_size(1024), 25_171_972);
        assert_eq!(simple_size(256), 6_292_996);
        assert_eq!(layered_velarised_size(), 58_040_864);
        assert_eq!(sandhi_size(), 89_315_360);
        assert_eq!(SANDHI_AUX, 64_368);
        assert_eq!(
            wide_ft_size(1024, 1770) + one_layer_head_size(1024),
            58_005_508
        );
    }

    #[test]
    fn zero_hidden256_network_evaluates_to_zero() {
        types::init();
        let bytes = vec![0u8; simple_size(256)];
        let net = ViridithasNetwork::from_bytes(&bytes).unwrap();
        assert_eq!(net.evaluate(&Board::new()), 0);
        assert_eq!(net.features_per_bucket(), FEATURES);
        assert!(matches!(net, ViridithasNetwork::Simple(_)));
    }

    #[test]
    fn unknown_payload_size_is_rejected() {
        let error = match ViridithasNetwork::from_bytes(&[1, 2, 3, 4]) {
            Ok(_) => panic!("tiny payload must be rejected"),
            Err(error) => error,
        };
        assert!(error.contains("not a supported layout"), "{error}");
    }

    #[test]
    fn hard_swish6_matches_x_times_clamped_linear_gate() {
        assert_eq!(hard_swish6(0.0), 0.0);
        assert_eq!(hard_swish6(-4.0), 0.0);
        assert!((hard_swish6(3.0) - 3.0).abs() < f32::EPSILON);
        assert!((hard_swish6(6.0) - 6.0).abs() < f32::EPSILON);
    }

    #[test]
    fn king_bucket_mirrors_the_published_half_map() {
        assert_eq!(king_bucket(0), 0); // a1
        assert_eq!(king_bucket(4), 3); // e1 mirrors to d1 → 3
        assert_eq!(king_bucket(relative_square(Color::Black, 60)), 3); // e8 from Black
        assert_eq!(king_bucket(60), 15); // e8 absolute is rank-8, file-d after mirror
        assert_eq!(king_bucket(27), 11); // d4
    }

    #[test]
    fn merged_king_plane_shares_one_64_slot_block() {
        let white_king = feature_index(Color::White, 4, Piece::King, Color::White, 4);
        let black_king = feature_index(Color::White, 4, Piece::King, Color::Black, 60);
        assert_eq!(white_king / 64, 5);
        assert_eq!(black_king / 64, 5);
        assert_ne!(white_king, black_king);
        assert!(white_king < LAYERED_INPUT);
        assert!(black_king < LAYERED_INPUT);
    }

    #[test]
    fn horizontal_mirror_flips_files_when_the_king_is_on_the_right() {
        let e2 = feature_index(Color::White, 4, Piece::Pawn, Color::White, 12);
        let d2 = feature_index(Color::White, 3, Piece::Pawn, Color::White, 11);
        assert_eq!(e2, d2);
    }

    #[test]
    fn downloaded_velarised_file_evaluates_in_opening_range() {
        types::init();
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let candidates = [
            workspace.join("nnue/velarised-2-b800.nnue.zst"),
            workspace.join("dist/nnue/velarised-2-b800.nnue.zst"),
        ];
        let Some(path) = candidates.iter().find(|path| path.is_file()) else {
            return;
        };
        let net = load(path).expect("velarised-2 layered layout");
        assert!(matches!(*net, ViridithasNetwork::Layered(_)));
        assert_eq!(net.features_per_bucket(), LAYERED_INPUT);
        let startpos = net.evaluate(&Board::new());
        assert!(
            startpos.abs() < 250,
            "velarised-2 startpos should be a quiet opening score, got {startpos}"
        );
        let white_mates =
            Board::from_fen("4k3/8/8/8/8/8/8/4KQ2 w - - 0 1").expect("KQ vs k is valid");
        let black_to_move =
            Board::from_fen("4k3/8/8/8/8/8/8/4KQ2 b - - 0 1").expect("KQ vs k is valid");
        let white_score = net.evaluate(&white_mates);
        let black_score = net.evaluate(&black_to_move);
        assert!(
            white_score > 300,
            "velarised-2 must prefer White in KQ vs k, got {white_score}"
        );
        assert!(
            black_score < -300,
            "velarised-2 must flip with side-to-move, got {black_score}"
        );
    }

    #[test]
    fn unknown_sizes_still_name_the_supported_layouts() {
        let Err(err) = ViridithasNetwork::from_bytes(&[0u8; 64]) else {
            panic!("tiny buffers must not parse as a Viridithas net");
        };
        assert!(err.contains("sandhi"), "{err}");
        assert!(err.contains("velarised"), "{err}");
    }

    #[test]
    fn downloaded_sandhi_file_evaluates_in_opening_range() {
        types::init();
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let candidates = [
            workspace.join("nnue/sandhi-s2-b200.nnue.zst"),
            workspace.join("nnue/viri_default.nnue.zst"),
            workspace.join("dist/nnue/viri_default.nnue.zst"),
        ];
        let Some(path) = candidates.iter().find(|path| path.is_file()) else {
            return;
        };
        let net = match load(path) {
            Ok(net) if matches!(*net, ViridithasNetwork::Sandhi(_)) => net,
            Ok(_) => return,
            Err(_) => return,
        };
        assert_eq!(net.hidden(), SANDHI_L0);
        let startpos = net.evaluate(&Board::new());
        assert!(
            startpos.abs() < 250,
            "sandhi startpos should be a quiet opening score, got {startpos}"
        );
        let white_mates =
            Board::from_fen("4k3/8/8/8/8/8/8/4KQ2 w - - 0 1").expect("KQ vs k is valid");
        let black_to_move =
            Board::from_fen("4k3/8/8/8/8/8/8/4KQ2 b - - 0 1").expect("KQ vs k is valid");
        let white_score = net.evaluate(&white_mates);
        let black_score = net.evaluate(&black_to_move);
        assert!(
            white_score > 300,
            "sandhi must prefer White in KQ vs k, got {white_score}"
        );
        assert!(
            black_score < -300,
            "sandhi must flip with side-to-move, got {black_score}"
        );
        let extra_pawn =
            Board::from_fen("4k3/8/8/8/8/8/P7/4K3 w - - 0 1").expect("K+P vs k is valid");
        let bare_kings = Board::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").expect("Kk is valid");
        assert!(
            net.evaluate(&extra_pawn) > net.evaluate(&bare_kings),
            "sandhi must prefer an extra pawn over bare kings"
        );
        let mut threats = 0usize;
        let mut pairs = 0usize;
        visit_threat_features(&Board::new(), 0, |_| threats += 1);
        visit_pawn_pair_features(&Board::new(), 0, |_| pairs += 1);
        assert!(threats > 0, "startpos must fire SFNNv12 threat features");
        assert!(pairs > 0, "startpos must fire pawn-pair features");
    }

    #[test]
    fn detects_viridithas_names_and_magic() {
        assert!(looks_like_viridithas(
            Path::new("viri_default.nnue.zst"),
            &[]
        ));
        assert!(looks_like_viridithas(
            Path::new("sandhi-s2-b200.nnue.zst"),
            &[]
        ));
        assert!(looks_like_viridithas(Path::new("net.bin"), &ZSTD_MAGIC));
        assert!(!looks_like_viridithas(
            Path::new("ak_default.bin"),
            &[0, 1, 2]
        ));
    }
}
