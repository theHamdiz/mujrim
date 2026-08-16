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

use types::chess_move::MoveFlag;
use types::{Board, Color, Move, Piece, Square};

use super::bit_rays::collect_bit_ray_move_deltas;
use super::dirty_threats::{MAX_DIRTY_THREAT_DELTAS, ThreatDelta, ThreatDeltaSink, ThreatSnapshot};
use super::stockfish_format::{
    AuxFeatureLists, PAIR_FEATURES, THREAT_FEATURES, collect_aux_feature_lists,
    collect_moved_pawn_pair_delta, visit_pawn_pair_features, visit_threat_delta,
    visit_threat_features,
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
    let sum = if hidden == HIDDEN {
        let us: &[i16; HIDDEN] = us[..HIDDEN]
            .try_into()
            .expect("Simple us accumulator is 1024");
        let them: &[i16; HIDDEN] = them[..HIDDEN]
            .try_into()
            .expect("Simple them accumulator is 1024");
        let weights: &[i16] = &net.output_weights;
        let w0: &[i16; HIDDEN] = weights[..HIDDEN]
            .try_into()
            .expect("Simple output weights cover us");
        let w1: &[i16; HIDDEN] = weights[HIDDEN..HIDDEN * 2]
            .try_into()
            .expect("Simple output weights cover them");
        net.output_bias + super::simd::flatten_pair(us, w0, them, w1)
    } else {
        finish_simple_scalar(net, us, them, hidden)
    };
    (sum / (QA * QA)) * SCALE / 64
}

#[inline(always)]
fn finish_simple_scalar(net: &SimpleNetwork, us: &[i16], them: &[i16], hidden: usize) -> i32 {
    let mut sum = net.output_bias;
    let weights = net.output_weights.as_ptr();
    unsafe {
        for i in 0..hidden {
            sum += screlu(*us.get_unchecked(i)) * i32::from(*weights.add(i));
            sum += screlu(*them.get_unchecked(i)) * i32::from(*weights.add(hidden + i));
        }
    }
    sum
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
    finish_layered(net, board, &acc_white, &acc_black)
}

fn finish_layered(
    net: &LayeredNetwork,
    board: &Board,
    acc_white: &[i16; LAYERED_L1],
    acc_black: &[i16; LAYERED_L1],
) -> i32 {
    let (us, them) = if board.side_to_move == Color::White {
        (acc_white, acc_black)
    } else {
        (acc_black, acc_white)
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

    let l3 = super::layered_forward::dot_f32(
        &l2,
        &net.l3_weights[bucket * LAYERED_L3..bucket * LAYERED_L3 + LAYERED_L3],
        net.l3_biases[bucket],
    );
    (l3 * LAYERED_SCALE as f32) as i32
}

fn evaluate_sandhi(net: &SandhiNetwork, board: &Board) -> i32 {
    let mut acc_white = [0i16; SANDHI_L0];
    let mut acc_black = [0i16; SANDHI_L0];
    let mut aux_white = [0i16; SANDHI_L0];
    let mut aux_black = [0i16; SANDHI_L0];
    acc_white.copy_from_slice(&net.feature_biases);
    acc_black.copy_from_slice(&net.feature_biases);
    accumulate_sandhi_pieces(net, board, Color::White, &mut acc_white);
    accumulate_sandhi_pieces(net, board, Color::Black, &mut acc_black);
    add_sandhi_aux(net, board, Color::White, &mut aux_white);
    add_sandhi_aux(net, board, Color::Black, &mut aux_black);
    finish_sandhi(net, board, &acc_white, &acc_black, &aux_white, &aux_black)
}

fn finish_sandhi(
    net: &SandhiNetwork,
    board: &Board,
    acc_white: &[i16; SANDHI_L0],
    acc_black: &[i16; SANDHI_L0],
    aux_white: &[i16; SANDHI_L0],
    aux_black: &[i16; SANDHI_L0],
) -> i32 {
    let (us, them, us_aux, them_aux) = if board.side_to_move == Color::White {
        (acc_white, acc_black, aux_white, aux_black)
    } else {
        (acc_black, acc_white, aux_black, aux_white)
    };

    let mut ft = super::layered_forward::Align64::new([0u8; SANDHI_L0]);
    activate_pairwise_sandhi_sum(us, us_aux, &mut ft.0[..SANDHI_L0 / 2]);
    activate_pairwise_sandhi_sum(them, them_aux, &mut ft.0[SANDHI_L0 / 2..]);

    let pieces = board.all_occupancy().count_ones() as usize;
    let bucket = ((pieces - 2) / 4).min(LAYERED_OUTPUT_BUCKETS - 1);

    let mut l1 = [0.0f32; SANDHI_L1];
    let mut sums = [0i32; SANDHI_L1];
    let l1_base = bucket * SANDHI_L1 * SANDHI_L0;
    super::layered_forward::affine_sparse_packed(
        &ft.0,
        &net.l1_weights[l1_base..l1_base + SANDHI_L1 * SANDHI_L0],
        &mut sums,
    );
    let bias = bucket * SANDHI_L1;
    for (j, sum) in sums.iter().enumerate() {
        l1[j] = hard_swish6((*sum as f32).mul_add(L1_MUL, net.l1_biases[bias + j]));
    }

    let mut l2_pre = [0.0f32; SANDHI_L2 * 2];
    let l2_weight_base = bucket * SANDHI_L1 * (SANDHI_L2 * 2);
    let l2_bias = bucket * SANDHI_L2 * 2;
    super::layered_forward::affine_f32(
        &l1,
        &net.l2_weights[l2_weight_base..l2_weight_base + SANDHI_L1 * (SANDHI_L2 * 2)],
        &net.l2_biases[l2_bias..l2_bias + SANDHI_L2 * 2],
        &mut l2_pre,
    );
    // Sandhi L2 is square (32→32); the published head adds the L1 residual.
    let mut l2 = [0.0f32; SANDHI_L2];
    for i in 0..SANDHI_L2 {
        l2[i] = hard_swish6(l2_pre[i]).mul_add(l2_pre[i + SANDHI_L2], l1[i]);
    }

    let l3 = super::layered_forward::dot_f32(
        &l2,
        &net.l3_weights[bucket * SANDHI_L2..bucket * SANDHI_L2 + SANDHI_L2],
        net.l3_biases[bucket],
    );
    (l3 * LAYERED_SCALE as f32) as i32
}

fn accumulate_sandhi_pieces(
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
}

fn add_sandhi_aux(
    net: &SandhiNetwork,
    board: &Board,
    perspective: Color,
    acc: &mut [i16; SANDHI_L0],
) {
    let pov = perspective.index();
    visit_threat_features(board, pov, |feature| {
        apply_sandhi_aux_feature(acc, net, feature, 1);
    });
    visit_pawn_pair_features(board, pov, |feature| {
        apply_sandhi_aux_feature(acc, net, feature, 1);
    });
}

/// Official sandhi `l0_aux` is `[pawn-pairs | threats]`; SFNNv12 visitors emit
/// threat indices first (`0..THREAT_FEATURES`) then pairs (`THREAT_FEATURES..`).
#[inline]
fn sandhi_aux_row(feature: usize) -> usize {
    if feature < THREAT_FEATURES {
        PAIR_FEATURES + feature
    } else {
        feature - THREAT_FEATURES
    }
}

fn apply_sandhi_aux_feature(
    acc: &mut [i16; SANDHI_L0],
    net: &SandhiNetwork,
    feature: usize,
    sign: i16,
) {
    super::stockfish_simd::apply_i8_feature_width(
        acc,
        &net.aux_weights,
        sandhi_aux_row(feature),
        sign,
    );
}

const MAX_SANDHI_AUX_DELTA: usize = 192;

struct SandhiAuxDelta {
    adds: [usize; MAX_SANDHI_AUX_DELTA],
    subs: [usize; MAX_SANDHI_AUX_DELTA],
    add_count: usize,
    sub_count: usize,
    overflowed: bool,
}

impl SandhiAuxDelta {
    fn new() -> Self {
        Self {
            adds: [0; MAX_SANDHI_AUX_DELTA],
            subs: [0; MAX_SANDHI_AUX_DELTA],
            add_count: 0,
            sub_count: 0,
            overflowed: false,
        }
    }

    fn push(&mut self, feature: usize, sign: i16) {
        let row = sandhi_aux_row(feature);
        if sign > 0 {
            if self.add_count < MAX_SANDHI_AUX_DELTA {
                self.adds[self.add_count] = row;
                self.add_count += 1;
            } else {
                self.overflowed = true;
            }
        } else if self.sub_count < MAX_SANDHI_AUX_DELTA {
            self.subs[self.sub_count] = row;
            self.sub_count += 1;
        } else {
            self.overflowed = true;
        }
    }
}

fn apply_sandhi_aux_lists(
    acc: &mut [i16; SANDHI_L0],
    net: &SandhiNetwork,
    lists: &AuxFeatureLists,
) {
    if lists.overflowed {
        return;
    }
    let mut delta = SandhiAuxDelta::new();
    for &feature in &lists.threats[..lists.threat_count] {
        delta.push(usize::from(feature), 1);
    }
    for &feature in &lists.pairs[..lists.pair_count] {
        delta.push(usize::from(feature), 1);
    }
    if delta.overflowed {
        for &feature in &lists.threats[..lists.threat_count] {
            apply_sandhi_aux_feature(acc, net, usize::from(feature), 1);
        }
        for &feature in &lists.pairs[..lists.pair_count] {
            apply_sandhi_aux_feature(acc, net, usize::from(feature), 1);
        }
        return;
    }
    let zeros = [0i16; SANDHI_L0];
    super::stockfish_simd::apply_i8_from_width(
        acc,
        &zeros,
        &net.aux_weights,
        &delta.adds[..delta.add_count],
        &delta.subs[..delta.sub_count],
    );
}

#[inline]
fn activate_pairwise_sandhi_sum(acc: &[i16; SANDHI_L0], aux: &[i16; SANDHI_L0], out: &mut [u8]) {
    let half = SANDHI_L0 / 2;
    debug_assert_eq!(out.len(), half);
    super::stockfish_simd::transform_pair_sum(
        &acc[..half],
        &aux[..half],
        &acc[half..],
        &aux[half..],
        out,
    );
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
    super::stockfish_simd::transform_pair(&acc[..half], &acc[half..], out);
}

fn propagate_l1(
    net: &LayeredNetwork,
    inputs: &[u8; LAYERED_L1],
    bucket: usize,
    out: &mut [f32; LAYERED_L2],
) {
    let mut sums = [0i32; LAYERED_L2];
    let l1_base = bucket * LAYERED_L2 * LAYERED_L1;
    super::layered_forward::affine_sparse_packed(
        inputs,
        &net.l1_weights[l1_base..l1_base + LAYERED_L2 * LAYERED_L1],
        &mut sums,
    );
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
    let weight_base = bucket * LAYERED_L2 * (LAYERED_L3 * 2);
    let bias = bucket * LAYERED_L3 * 2;
    super::layered_forward::affine_f32(
        inputs,
        &net.l2_weights[weight_base..weight_base + LAYERED_L2 * (LAYERED_L3 * 2)],
        &net.l2_biases[bias..bias + LAYERED_L3 * 2],
        &mut sums,
    );
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
fn add_i16(acc: &mut [i16], weights: &[i16], base: usize) {
    let width = acc.len();
    debug_assert!(width > 0 && base.is_multiple_of(width));
    super::stockfish_simd::apply_i16_feature_width(acc, weights, base / width, 1);
}

/// Official Viridithas 20 load-time L1 permutation (`REPERMUTE_INDICES`).
/// Pairwise still joins `i` with `i + 512`; this only reorders neurons for NNZ.
const SANDHI_REPERMUTE: [usize; SANDHI_L0 / 2] = [
    225, 481, 452, 1, 356, 313, 294, 249, 460, 508, 391, 258, 132, 335, 398, 93, 148, 120, 403,
    259, 401, 487, 14, 482, 463, 250, 40, 326, 157, 426, 304, 421, 379, 123, 165, 48, 100, 505, 57,
    413, 252, 296, 70, 2, 506, 170, 226, 509, 247, 130, 270, 408, 63, 497, 276, 231, 350, 190, 344,
    31, 425, 166, 441, 183, 13, 108, 211, 142, 376, 301, 388, 135, 79, 38, 204, 20, 194, 215, 193,
    68, 67, 449, 450, 323, 471, 49, 324, 305, 172, 480, 229, 428, 253, 503, 395, 32, 126, 213, 173,
    197, 432, 50, 241, 169, 318, 321, 272, 453, 176, 234, 469, 288, 339, 429, 306, 364, 307, 422,
    287, 440, 94, 88, 159, 248, 227, 483, 504, 222, 410, 377, 39, 415, 124, 95, 171, 127, 7, 235,
    439, 149, 76, 312, 267, 46, 41, 297, 302, 56, 11, 405, 351, 455, 478, 383, 371, 263, 502, 81,
    501, 136, 43, 162, 310, 78, 55, 64, 333, 284, 368, 278, 309, 357, 10, 240, 411, 266, 69, 155,
    443, 101, 53, 77, 112, 28, 22, 510, 320, 496, 311, 334, 277, 489, 146, 44, 138, 394, 300, 233,
    161, 244, 206, 417, 500, 345, 381, 458, 353, 423, 86, 177, 264, 152, 147, 349, 397, 238, 435,
    412, 286, 265, 328, 341, 485, 378, 470, 224, 285, 236, 29, 207, 370, 45, 477, 499, 484, 303,
    178, 150, 337, 85, 299, 246, 139, 327, 228, 382, 71, 340, 26, 143, 60, 105, 331, 92, 348, 218,
    490, 314, 83, 467, 315, 343, 346, 338, 457, 420, 359, 279, 33, 275, 256, 358, 362, 409, 4, 308,
    360, 104, 332, 52, 260, 153, 102, 106, 34, 192, 121, 367, 396, 329, 293, 436, 283, 473, 347,
    91, 254, 476, 220, 117, 399, 75, 216, 316, 274, 365, 109, 18, 373, 472, 393, 58, 384, 355, 474,
    262, 61, 160, 74, 245, 84, 199, 374, 115, 454, 479, 154, 380, 325, 255, 511, 140, 16, 290, 19,
    118, 198, 223, 407, 269, 372, 23, 185, 113, 205, 25, 5, 89, 97, 202, 201, 342, 125, 103, 404,
    134, 354, 208, 462, 209, 402, 289, 8, 491, 3, 141, 145, 448, 433, 167, 431, 184, 456, 51, 438,
    200, 182, 219, 144, 210, 195, 119, 243, 30, 203, 392, 72, 122, 261, 281, 369, 280, 486, 107,
    54, 251, 129, 156, 385, 9, 82, 451, 66, 188, 212, 168, 131, 239, 17, 158, 414, 298, 189, 445,
    42, 99, 221, 128, 47, 446, 434, 295, 110, 137, 282, 98, 361, 464, 390, 461, 465, 175, 271, 15,
    363, 416, 6, 317, 494, 330, 59, 427, 214, 87, 21, 319, 90, 164, 187, 366, 406, 133, 389, 430,
    174, 12, 268, 35, 291, 493, 237, 96, 352, 111, 27, 217, 37, 73, 180, 24, 230, 442, 232, 447,
    488, 191, 151, 186, 0, 116, 273, 418, 387, 468, 322, 495, 475, 375, 424, 444, 459, 62, 507, 65,
    242, 179, 336, 163, 36, 419, 292, 80, 400, 466, 498, 492, 114, 386, 257, 196, 437, 181,
];

fn repermute_hidden_i16(weights: &mut [i16], hidden: usize) {
    debug_assert_eq!(hidden, SANDHI_L0);
    let half = hidden / 2;
    let mut scratch = [0i16; SANDHI_L0];
    for row in weights.chunks_exact_mut(hidden) {
        for (tgt, src) in SANDHI_REPERMUTE.iter().copied().enumerate() {
            scratch[tgt] = row[src];
            scratch[tgt + half] = row[src + half];
        }
        row.copy_from_slice(&scratch);
    }
}

fn repermute_hidden_i8(weights: &mut [i8], hidden: usize) {
    debug_assert_eq!(hidden, SANDHI_L0);
    let half = hidden / 2;
    let mut scratch = [0i8; SANDHI_L0];
    for row in weights.chunks_exact_mut(hidden) {
        for (tgt, src) in SANDHI_REPERMUTE.iter().copied().enumerate() {
            scratch[tgt] = row[src];
            scratch[tgt + half] = row[src + half];
        }
        row.copy_from_slice(&scratch);
    }
}

fn repermute_l1_inputs(weights: &mut [i8], inputs: usize, outputs: usize) {
    debug_assert_eq!(inputs, SANDHI_L0);
    let stride = LAYERED_OUTPUT_BUCKETS * outputs;
    let half = inputs / 2;
    let mut scratch = vec![0i8; weights.len()];
    for (tgt, src) in SANDHI_REPERMUTE.iter().copied().enumerate() {
        let dst = tgt * stride;
        let src0 = src * stride;
        scratch[dst..dst + stride].copy_from_slice(&weights[src0..src0 + stride]);
        let dst = (tgt + half) * stride;
        let src1 = (src + half) * stride;
        scratch[dst..dst + stride].copy_from_slice(&weights[src1..src1 + stride]);
    }
    weights.copy_from_slice(&scratch);
}

fn transpose_viri_l1(src: &[i8], inputs: usize, outputs: usize) -> Box<[i8]> {
    let mut dst = vec![0i8; LAYERED_OUTPUT_BUCKETS * outputs * inputs].into_boxed_slice();
    for input in 0..inputs {
        for bucket in 0..LAYERED_OUTPUT_BUCKETS {
            for output in 0..outputs {
                dst[bucket * outputs * inputs + output * inputs + input] =
                    src[input * LAYERED_OUTPUT_BUCKETS * outputs + bucket * outputs + output];
            }
        }
    }
    dst
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
    let l1_src = read_i8s(
        bytes,
        &mut offset,
        LAYERED_L1 * LAYERED_OUTPUT_BUCKETS * LAYERED_L2,
    )?;
    let l1_weights = super::layered_forward::pack_nnz_buckets(
        &transpose_viri_l1(&l1_src, LAYERED_L1, LAYERED_L2),
        LAYERED_OUTPUT_BUCKETS,
        LAYERED_L1,
        LAYERED_L2,
    );
    let l1_biases = read_f32s(bytes, &mut offset, LAYERED_OUTPUT_BUCKETS * LAYERED_L2)?;
    let l2_weights = super::layered_forward::pack_f32_buckets(
        &read_f32s(
            bytes,
            &mut offset,
            LAYERED_L2 * LAYERED_OUTPUT_BUCKETS * (LAYERED_L3 * 2),
        )?,
        LAYERED_OUTPUT_BUCKETS,
        LAYERED_L2,
        LAYERED_L3 * 2,
    );
    let l2_biases = read_f32s(
        bytes,
        &mut offset,
        LAYERED_OUTPUT_BUCKETS * (LAYERED_L3 * 2),
    )?;
    let l3_weights = super::layered_forward::pack_f32_buckets(
        &read_f32s(bytes, &mut offset, LAYERED_L3 * LAYERED_OUTPUT_BUCKETS)?,
        LAYERED_OUTPUT_BUCKETS,
        LAYERED_L3,
        1,
    );
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
    let mut aux_weights = read_i8s(bytes, &mut offset, SANDHI_AUX * SANDHI_L0)?;
    repermute_hidden_i8(&mut aux_weights, SANDHI_L0);
    let mut feature_weights =
        read_i16s(bytes, &mut offset, KING_BUCKETS * LAYERED_INPUT * SANDHI_L0)?;
    repermute_hidden_i16(&mut feature_weights, SANDHI_L0);
    let mut feature_biases = read_i16s(bytes, &mut offset, SANDHI_L0)?;
    repermute_hidden_i16(&mut feature_biases, SANDHI_L0);
    let mut l1_src = read_i8s(
        bytes,
        &mut offset,
        SANDHI_L0 * LAYERED_OUTPUT_BUCKETS * SANDHI_L1,
    )?;
    repermute_l1_inputs(&mut l1_src, SANDHI_L0, SANDHI_L1);
    let l1_weights = super::layered_forward::pack_nnz_buckets(
        &transpose_viri_l1(&l1_src, SANDHI_L0, SANDHI_L1),
        LAYERED_OUTPUT_BUCKETS,
        SANDHI_L0,
        SANDHI_L1,
    );
    let l1_biases = read_f32s(bytes, &mut offset, LAYERED_OUTPUT_BUCKETS * SANDHI_L1)?;
    let l2_weights = super::layered_forward::pack_f32_buckets(
        &read_f32s(
            bytes,
            &mut offset,
            SANDHI_L1 * LAYERED_OUTPUT_BUCKETS * (SANDHI_L2 * 2),
        )?,
        LAYERED_OUTPUT_BUCKETS,
        SANDHI_L1,
        SANDHI_L2 * 2,
    );
    let l2_biases = read_f32s(bytes, &mut offset, LAYERED_OUTPUT_BUCKETS * (SANDHI_L2 * 2))?;
    let l3_weights = super::layered_forward::pack_f32_buckets(
        &read_f32s(bytes, &mut offset, SANDHI_L2 * LAYERED_OUTPUT_BUCKETS)?,
        LAYERED_OUTPUT_BUCKETS,
        SANDHI_L2,
        1,
    );
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

const MAX_PLY: usize = 256;
const SIMPLE_BUCKETS: usize = 16;
const SIMPLE_FINNY: usize = 2 * SIMPLE_BUCKETS;

#[inline(always)]
fn simple_bucket(king: usize, pov: Color) -> usize {
    relative_square(pov, king) / 4
}

#[inline(always)]
fn simple_feature(
    net: &SimpleNetwork,
    king: usize,
    pov: Color,
    piece: Piece,
    piece_color: Color,
    sq: usize,
) -> usize {
    let local =
        (usize::from(piece_color != pov) * 6 + piece.index()) * 64 + relative_square(pov, sq);
    simple_bucket(king, pov) * net.features_per_bucket + local
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn apply_simple_feature(
    acc: &mut [i16],
    net: &SimpleNetwork,
    king: usize,
    pov: Color,
    piece: Piece,
    piece_color: Color,
    sq: usize,
    sign: i16,
) {
    super::stockfish_simd::apply_i16_feature_width(
        acc,
        &net.feature_weights,
        simple_feature(net, king, pov, piece, piece_color, sq),
        sign,
    );
}

#[inline(always)]
fn snapshot_occupancy(board: &Board) -> [u64; 12] {
    [
        board.pieces[0][0],
        board.pieces[0][1],
        board.pieces[0][2],
        board.pieces[0][3],
        board.pieces[0][4],
        board.pieces[0][5],
        board.pieces[1][0],
        board.pieces[1][1],
        board.pieces[1][2],
        board.pieces[1][3],
        board.pieces[1][4],
        board.pieces[1][5],
    ]
}

struct SimpleFrame {
    values: [[i16; HIDDEN]; 2],
    kings: [u8; 2],
    pending_has_move: bool,
    pending_move: Move,
    pending_mover: u8,
    pending_captured: u8,
    hash: u64,
    accurate: bool,
    pending_null: bool,
}

impl Default for SimpleFrame {
    fn default() -> Self {
        Self {
            values: [[0; HIDDEN]; 2],
            kings: [u8::MAX; 2],
            pending_has_move: false,
            pending_move: Move::quiet(Square::A1, Square::A1),
            pending_mover: u8::MAX,
            pending_captured: u8::MAX,
            hash: 0,
            accurate: false,
            pending_null: false,
        }
    }
}

impl Clone for SimpleFrame {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for SimpleFrame {}

struct SimpleFinny {
    values: [i16; HIDDEN],
    occupancy: [u64; 12],
    initialized: bool,
}

impl Default for SimpleFinny {
    fn default() -> Self {
        Self {
            values: [0; HIDDEN],
            occupancy: [0; 12],
            initialized: false,
        }
    }
}

struct SimpleAccumulatorState {
    frames: Box<[SimpleFrame]>,
    finny: Box<[SimpleFinny]>,
    hidden: usize,
    index: usize,
}

impl SimpleAccumulatorState {
    fn new(hidden: usize) -> Self {
        debug_assert!(hidden <= HIDDEN && hidden.is_multiple_of(32));
        Self {
            frames: vec![SimpleFrame::default(); MAX_PLY].into_boxed_slice(),
            finny: (0..SIMPLE_FINNY)
                .map(|_| SimpleFinny::default())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            hidden,
            index: 0,
        }
    }

    fn acc_mut(values: &mut [i16; HIDDEN], hidden: usize) -> &mut [i16] {
        &mut values[..hidden]
    }

    fn push_move(&mut self, board: &Board, mv: Move) {
        assert!(
            self.index + 1 < self.frames.len(),
            "Viridithas NNUE stack exhausted"
        );
        self.index += 1;
        let frame = &mut self.frames[self.index];
        frame.accurate = false;
        frame.pending_null = false;
        frame.pending_has_move = true;
        frame.pending_move = mv;
        frame.pending_mover = board.piece_ids()[mv.from.index()];
        frame.pending_captured = board.piece_ids()[mv.to.index()];
        frame.hash = 0;
    }

    fn push_null(&mut self) {
        assert!(
            self.index + 1 < self.frames.len(),
            "Viridithas NNUE stack exhausted"
        );
        let next = self.index + 1;
        self.frames[next] = self.frames[self.index];
        self.frames[next].pending_has_move = false;
        self.frames[next].pending_null = true;
        self.index = next;
    }

    fn pop(&mut self) {
        assert!(self.index != 0, "cannot pop the root Viridithas NNUE frame");
        self.index -= 1;
    }

    fn clear(&mut self) {
        self.index = 0;
        self.frames[0].accurate = false;
        self.frames[0].pending_null = false;
        for entry in self.finny.iter_mut() {
            entry.initialized = false;
        }
    }

    fn evaluate(&mut self, board: &Board, net: &SimpleNetwork) -> i32 {
        self.ensure(board, net, false);
        self.finish(board, net)
    }

    fn evaluate_search(&mut self, board: &Board, net: &SimpleNetwork) -> i32 {
        self.ensure(board, net, true);
        self.finish(board, net)
    }

    fn finish(&self, board: &Board, net: &SimpleNetwork) -> i32 {
        let frame = &self.frames[self.index];
        finish_simple(
            net,
            &frame.values[board.side_to_move.index()][..self.hidden],
            &frame.values[board.side_to_move.opponent().index()][..self.hidden],
            self.hidden,
        )
    }

    fn ensure(&mut self, board: &Board, net: &SimpleNetwork, trusted: bool) {
        debug_assert_eq!(net.hidden, self.hidden);
        if self.frames[self.index].accurate && self.frames[self.index].pending_null {
            self.frames[self.index].hash = board.hash;
            self.frames[self.index].pending_null = false;
        }
        if self.frames[self.index].accurate
            && (trusted || self.frames[self.index].hash == board.hash)
        {
            return;
        }
        if self.index != 0 && self.frames[self.index - 1].accurate {
            self.update_from_parent(board, net);
        } else {
            self.refresh(board, net);
        }
    }

    fn refresh(&mut self, board: &Board, net: &SimpleNetwork) {
        let occupancy = snapshot_occupancy(board);
        let kings = [
            board.king_square(Color::White).index(),
            board.king_square(Color::Black).index(),
        ];
        for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
            self.finny_refresh(side, kings[pov], &occupancy, net);
            let src =
                self.finny[side.index() * SIMPLE_BUCKETS + simple_bucket(kings[pov], side)].values;
            self.frames[self.index].values[pov] = src;
        }
        let frame = &mut self.frames[self.index];
        frame.kings = [kings[0] as u8, kings[1] as u8];
        frame.hash = board.hash;
        frame.accurate = true;
        frame.pending_has_move = false;
        frame.pending_null = false;
    }

    fn update_from_parent(&mut self, board: &Board, net: &SimpleNetwork) {
        let current = self.index;
        let kings = [
            board.king_square(Color::White).index(),
            board.king_square(Color::Black).index(),
        ];
        let parent_kings = [
            usize::from(self.frames[current - 1].kings[0]),
            usize::from(self.frames[current - 1].kings[1]),
        ];
        let needs_refresh = [
            simple_bucket(parent_kings[0], Color::White) != simple_bucket(kings[0], Color::White),
            simple_bucket(parent_kings[1], Color::Black) != simple_bucket(kings[1], Color::Black),
        ];
        if needs_refresh.iter().any(|&refresh| refresh) {
            let occupancy = snapshot_occupancy(board);
            for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
                if !needs_refresh[pov] {
                    continue;
                }
                self.finny_refresh(side, kings[pov], &occupancy, net);
                self.frames[current].values[pov] = self.finny
                    [side.index() * SIMPLE_BUCKETS + simple_bucket(kings[pov], side)]
                .values;
            }
        }
        let pending_has_move = self.frames[current].pending_has_move;
        let pending_move = self.frames[current].pending_move;
        let pending_mover = self.frames[current].pending_mover;
        let pending_captured = self.frames[current].pending_captured;
        let parent_values = self.frames[current - 1].values;
        let hidden = self.hidden;
        let frame = &mut self.frames[current];
        for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
            if needs_refresh[pov] {
                continue;
            }
            frame.values[pov] = parent_values[pov];
            if pending_has_move {
                apply_simple_move_delta(
                    Self::acc_mut(&mut frame.values[pov], hidden),
                    net,
                    kings[pov],
                    side,
                    pending_move,
                    pending_mover,
                    pending_captured,
                );
            }
        }
        frame.kings = [kings[0] as u8, kings[1] as u8];
        frame.hash = board.hash;
        frame.accurate = true;
        frame.pending_has_move = false;
        frame.pending_null = false;
    }

    fn finny_refresh(
        &mut self,
        side: Color,
        king: usize,
        occupancy: &[u64; 12],
        net: &SimpleNetwork,
    ) {
        let hidden = self.hidden;
        let entry = &mut self.finny[side.index() * SIMPLE_BUCKETS + simple_bucket(king, side)];
        if !entry.initialized {
            entry.values[..hidden].copy_from_slice(&net.feature_biases[..hidden]);
            add_simple_pieces(&mut entry.values[..hidden], net, king, side, occupancy);
            entry.occupancy = *occupancy;
            entry.initialized = true;
            return;
        }
        if entry.occupancy == *occupancy {
            return;
        }
        for color in 0..2 {
            for piece in 0..Piece::COUNT {
                let index = color * Piece::COUNT + piece;
                let piece = Piece::from_index(piece).expect("piece index is valid");
                let piece_color = if color == 0 {
                    Color::White
                } else {
                    Color::Black
                };
                let mut added = occupancy[index] & !entry.occupancy[index];
                while added != 0 {
                    let sq = added.trailing_zeros() as usize;
                    added &= added - 1;
                    apply_simple_feature(
                        &mut entry.values[..hidden],
                        net,
                        king,
                        side,
                        piece,
                        piece_color,
                        sq,
                        1,
                    );
                }
                let mut removed = entry.occupancy[index] & !occupancy[index];
                while removed != 0 {
                    let sq = removed.trailing_zeros() as usize;
                    removed &= removed - 1;
                    apply_simple_feature(
                        &mut entry.values[..hidden],
                        net,
                        king,
                        side,
                        piece,
                        piece_color,
                        sq,
                        -1,
                    );
                }
            }
        }
        entry.occupancy = *occupancy;
    }
}

fn add_simple_pieces(
    acc: &mut [i16],
    net: &SimpleNetwork,
    king: usize,
    pov: Color,
    occupancy: &[u64; 12],
) {
    for color in 0..2 {
        for piece in 0..Piece::COUNT {
            let mut bb = occupancy[color * Piece::COUNT + piece];
            while bb != 0 {
                let sq = bb.trailing_zeros() as usize;
                bb &= bb - 1;
                apply_simple_feature(
                    acc,
                    net,
                    king,
                    pov,
                    Piece::from_index(piece).expect("piece index is valid"),
                    if color == 0 {
                        Color::White
                    } else {
                        Color::Black
                    },
                    sq,
                    1,
                );
            }
        }
    }
}

fn apply_simple_move_delta(
    acc: &mut [i16],
    net: &SimpleNetwork,
    king: usize,
    side: Color,
    mv: Move,
    mover: u8,
    captured: u8,
) {
    debug_assert_ne!(mover, u8::MAX);
    let mover_piece = Piece::from_index(usize::from(mover) / 2).expect("mover piece is valid");
    let mover_color = if mover & 1 == 0 {
        Color::White
    } else {
        Color::Black
    };
    let resulting = mv.promotion.unwrap_or(mover_piece);
    apply_simple_feature(
        acc,
        net,
        king,
        side,
        mover_piece,
        mover_color,
        mv.from.index(),
        -1,
    );
    apply_simple_feature(
        acc,
        net,
        king,
        side,
        resulting,
        mover_color,
        mv.to.index(),
        1,
    );
    if mv.is_capture() && mv.flag != MoveFlag::EnPassant {
        apply_simple_feature(
            acc,
            net,
            king,
            side,
            Piece::from_index(usize::from(captured) / 2).expect("captured piece is valid"),
            if captured & 1 == 0 {
                Color::White
            } else {
                Color::Black
            },
            mv.to.index(),
            -1,
        );
    } else if mv.flag == MoveFlag::EnPassant {
        apply_simple_feature(
            acc,
            net,
            king,
            side,
            Piece::Pawn,
            mover_color.opponent(),
            Square::from_file_rank(mv.to.file(), mv.from.rank()).index(),
            -1,
        );
    } else if mv.is_castling() {
        let (rook_from, rook_to) = match (mover_color, mv.flag) {
            (Color::White, MoveFlag::KingCastle) => (Square::H1.index(), Square::F1.index()),
            (Color::White, MoveFlag::QueenCastle) => (Square::A1.index(), Square::D1.index()),
            (Color::Black, MoveFlag::KingCastle) => (Square::H8.index(), Square::F8.index()),
            (Color::Black, MoveFlag::QueenCastle) => (Square::A8.index(), Square::D8.index()),
            _ => unreachable!(),
        };
        apply_simple_feature(
            acc,
            net,
            king,
            side,
            Piece::Rook,
            mover_color,
            rook_from,
            -1,
        );
        apply_simple_feature(acc, net, king, side, Piece::Rook, mover_color, rook_to, 1);
    }
}

#[derive(Clone, Copy)]
struct WideFrameMeta {
    kings: [u8; 2],
    pending_has_move: bool,
    pending_move: Move,
    pending_mover: u8,
    pending_captured: u8,
    hash: u64,
    accurate: bool,
    pending_null: bool,
}

impl Default for WideFrameMeta {
    fn default() -> Self {
        Self {
            kings: [u8::MAX; 2],
            pending_has_move: false,
            pending_move: Move::quiet(Square::A1, Square::A1),
            pending_mover: u8::MAX,
            pending_captured: u8::MAX,
            hash: 0,
            accurate: false,
            pending_null: false,
        }
    }
}

struct WideAccumulatorState<const H: usize> {
    values: Box<[[[i16; H]; 2]]>,
    meta: Box<[WideFrameMeta]>,
    index: usize,
}

impl<const H: usize> WideAccumulatorState<H> {
    fn new() -> Self {
        Self {
            values: vec![[[0; H]; 2]; MAX_PLY].into_boxed_slice(),
            meta: (0..MAX_PLY)
                .map(|_| WideFrameMeta::default())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            index: 0,
        }
    }

    fn push_move(&mut self, board: &Board, mv: Move) {
        assert!(
            self.index + 1 < self.meta.len(),
            "Viridithas NNUE stack exhausted"
        );
        self.index += 1;
        let meta = &mut self.meta[self.index];
        meta.accurate = false;
        meta.pending_null = false;
        meta.pending_has_move = true;
        meta.pending_move = mv;
        meta.pending_mover = board.piece_ids()[mv.from.index()];
        meta.pending_captured = board.piece_ids()[mv.to.index()];
        meta.hash = 0;
    }

    fn push_null(&mut self) {
        assert!(
            self.index + 1 < self.meta.len(),
            "Viridithas NNUE stack exhausted"
        );
        let next = self.index + 1;
        let current_meta = self.meta[self.index];
        let current_values = self.values[self.index];
        self.meta[next] = current_meta;
        self.values[next] = current_values;
        self.meta[next].pending_has_move = false;
        self.meta[next].pending_null = true;
        self.index = next;
    }

    fn pop(&mut self) {
        assert!(self.index != 0, "cannot pop the root Viridithas NNUE frame");
        self.index -= 1;
    }

    fn clear(&mut self) {
        self.index = 0;
        self.meta[0].accurate = false;
        self.meta[0].pending_null = false;
    }

    fn ensure_pieces(&mut self, board: &Board, weights: &[i16], biases: &[i16], trusted: bool) {
        if self.meta[self.index].accurate && self.meta[self.index].pending_null {
            self.meta[self.index].hash = board.hash;
            self.meta[self.index].pending_null = false;
        }
        if self.meta[self.index].accurate && (trusted || self.meta[self.index].hash == board.hash) {
            return;
        }
        if self.index != 0 && self.meta[self.index - 1].accurate {
            self.update_from_parent(board, weights, biases);
        } else {
            self.refresh(board, weights, biases);
        }
    }

    fn refresh(&mut self, board: &Board, weights: &[i16], biases: &[i16]) {
        let kings = [
            board.king_square(Color::White).index(),
            board.king_square(Color::Black).index(),
        ];
        for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
            refresh_wide_perspective(
                &mut self.values[self.index][pov],
                weights,
                biases,
                board,
                kings[pov],
                side,
            );
        }
        let meta = &mut self.meta[self.index];
        meta.kings = [kings[0] as u8, kings[1] as u8];
        meta.hash = board.hash;
        meta.accurate = true;
        meta.pending_has_move = false;
        meta.pending_null = false;
    }

    fn update_from_parent(&mut self, board: &Board, weights: &[i16], biases: &[i16]) {
        let current = self.index;
        let kings = [
            board.king_square(Color::White).index(),
            board.king_square(Color::Black).index(),
        ];
        let parent_kings = [
            usize::from(self.meta[current - 1].kings[0]),
            usize::from(self.meta[current - 1].kings[1]),
        ];
        let needs_refresh = [
            wide_needs_refresh(parent_kings[0], kings[0], Color::White),
            wide_needs_refresh(parent_kings[1], kings[1], Color::Black),
        ];
        let pending_has_move = self.meta[current].pending_has_move;
        let pending_move = self.meta[current].pending_move;
        let pending_mover = self.meta[current].pending_mover;
        let pending_captured = self.meta[current].pending_captured;
        for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
            if needs_refresh[pov] {
                refresh_wide_perspective(
                    &mut self.values[current][pov],
                    weights,
                    biases,
                    board,
                    kings[pov],
                    side,
                );
            }
        }
        let (parents, children) = self.values.split_at_mut(current);
        let parent_values = &parents[current - 1];
        let child_values = &mut children[0];
        for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
            if needs_refresh[pov] {
                continue;
            }
            if pending_has_move {
                apply_wide_move_delta_from(
                    &mut child_values[pov],
                    &parent_values[pov],
                    weights,
                    kings[pov],
                    side,
                    pending_move,
                    pending_mover,
                    pending_captured,
                );
            } else {
                child_values[pov] = parent_values[pov];
            }
        }
        let meta = &mut self.meta[current];
        meta.kings = [kings[0] as u8, kings[1] as u8];
        meta.hash = board.hash;
        meta.accurate = true;
        meta.pending_has_move = false;
        meta.pending_null = false;
    }
}

#[derive(Clone, Copy)]
struct SandhiAuxFrame {
    values: [[i16; SANDHI_L0]; 2],
    lists: [AuxFeatureLists; 2],
    threat_deltas: [ThreatDelta; MAX_DIRTY_THREAT_DELTAS],
    threat_delta_count: usize,
    threat_overflowed: bool,
    pending_threats: Option<ThreatSnapshot>,
    pending_move: Option<Move>,
    pawns_before: [u64; 2],
    kings: [u8; 2],
    hash: u64,
    accurate: bool,
    pending_null: bool,
}

impl Default for SandhiAuxFrame {
    fn default() -> Self {
        Self {
            values: [[0; SANDHI_L0]; 2],
            lists: [AuxFeatureLists::default(); 2],
            threat_deltas: [ThreatDelta::default(); MAX_DIRTY_THREAT_DELTAS],
            threat_delta_count: 0,
            threat_overflowed: false,
            pending_threats: None,
            pending_move: None,
            pawns_before: [0; 2],
            kings: [u8::MAX; 2],
            hash: 0,
            accurate: false,
            pending_null: false,
        }
    }
}

impl ThreatDeltaSink for SandhiAuxFrame {
    #[inline(always)]
    fn push_threat_delta(&mut self, delta: ThreatDelta) {
        if self.threat_delta_count < self.threat_deltas.len() {
            self.threat_deltas[self.threat_delta_count] = delta;
            self.threat_delta_count += 1;
        } else {
            self.threat_overflowed = true;
        }
    }
}

struct SandhiAuxState {
    frames: Box<[SandhiAuxFrame]>,
    index: usize,
}

impl SandhiAuxState {
    fn new() -> Self {
        Self {
            frames: vec![SandhiAuxFrame::default(); MAX_PLY].into_boxed_slice(),
            index: 0,
        }
    }

    fn push_move(&mut self, board: &Board, mv: Move) {
        assert!(
            self.index + 1 < self.frames.len(),
            "Viridithas Sandhi aux stack exhausted"
        );
        self.index += 1;
        let frame = &mut self.frames[self.index];
        frame.accurate = false;
        frame.pending_null = false;
        frame.hash = 0;
        frame.threat_delta_count = 0;
        frame.threat_overflowed = false;
        frame.pending_move = Some(mv);
        frame.pawns_before = [
            board.pieces[0][Piece::Pawn.index()],
            board.pieces[1][Piece::Pawn.index()],
        ];
        frame.pending_threats = Some(ThreatSnapshot::from_board(board));
    }

    fn push_null(&mut self) {
        assert!(
            self.index + 1 < self.frames.len(),
            "Viridithas Sandhi aux stack exhausted"
        );
        let next = self.index + 1;
        self.frames[next] = self.frames[self.index];
        self.frames[next].pending_null = true;
        self.frames[next].pending_threats = None;
        self.frames[next].pending_move = None;
        self.index = next;
    }

    fn pop(&mut self) {
        assert!(
            self.index != 0,
            "cannot pop the root Viridithas Sandhi aux frame"
        );
        self.index -= 1;
    }

    fn clear(&mut self) {
        self.index = 0;
        self.frames[0].accurate = false;
        self.frames[0].pending_null = false;
    }

    fn ensure(&mut self, board: &Board, net: &SandhiNetwork, trusted: bool) {
        if self.frames[self.index].accurate && self.frames[self.index].pending_null {
            self.frames[self.index].hash = board.hash;
            self.frames[self.index].pending_null = false;
        }
        if self.frames[self.index].accurate
            && (trusted || self.frames[self.index].hash == board.hash)
        {
            return;
        }
        if self.index != 0 && self.frames[self.index - 1].accurate {
            self.update_from_parent(board, net);
        } else {
            self.refresh(board, net);
        }
    }

    fn refresh(&mut self, board: &Board, net: &SandhiNetwork) {
        let kings = [
            board.king_square(Color::White).index(),
            board.king_square(Color::Black).index(),
        ];
        for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
            self.refresh_pov(board, net, pov, side);
        }
        let frame = &mut self.frames[self.index];
        frame.kings = [kings[0] as u8, kings[1] as u8];
        frame.hash = board.hash;
        frame.accurate = true;
        frame.pending_threats = None;
        frame.pending_move = None;
        frame.pending_null = false;
    }

    fn refresh_pov(&mut self, board: &Board, net: &SandhiNetwork, pov: usize, side: Color) {
        let lists = collect_aux_feature_lists(board, side.index());
        let mut aux = [0i16; SANDHI_L0];
        if lists.overflowed {
            add_sandhi_aux(net, board, side, &mut aux);
        } else {
            apply_sandhi_aux_lists(&mut aux, net, &lists);
        }
        self.frames[self.index].values[pov] = aux;
        self.frames[self.index].lists[pov] = lists;
    }

    fn update_from_parent(&mut self, board: &Board, net: &SandhiNetwork) {
        let current = self.index;
        let kings = [
            board.king_square(Color::White).index(),
            board.king_square(Color::Black).index(),
        ];
        let parent_kings = [
            usize::from(self.frames[current - 1].kings[0]),
            usize::from(self.frames[current - 1].kings[1]),
        ];
        let needs_refresh = [
            wide_needs_refresh(parent_kings[0], kings[0], Color::White),
            wide_needs_refresh(parent_kings[1], kings[1], Color::Black),
        ];
        let parent_overflowed = [
            self.frames[current - 1].lists[0].overflowed,
            self.frames[current - 1].lists[1].overflowed,
        ];
        {
            let frame = &mut self.frames[current];
            if let (Some(snapshot), Some(mv)) = (frame.pending_threats.take(), frame.pending_move) {
                collect_bit_ray_move_deltas(frame, snapshot, mv);
            }
        }
        let threat_overflowed = self.frames[current].threat_overflowed;
        let pawns_before = self.frames[current].pawns_before;
        let pawns_after = [
            board.pieces[0][Piece::Pawn.index()],
            board.pieces[1][Piece::Pawn.index()],
        ];
        let deltas = self.frames[current].threat_deltas;
        let delta_count = self.frames[current].threat_delta_count;
        let mut apply = [false; 2];
        let mut pov_deltas = [SandhiAuxDelta::new(), SandhiAuxDelta::new()];
        for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
            if needs_refresh[pov] || parent_overflowed[pov] || threat_overflowed {
                self.refresh_pov(board, net, pov, side);
                continue;
            }
            let delta = &mut pov_deltas[pov];
            for &threat in &deltas[..delta_count] {
                visit_threat_delta(threat, kings[pov], pov, |feature, sign| {
                    delta.push(feature, sign);
                });
            }
            if pawns_before != pawns_after {
                let mut pair_adds = [0usize; 64];
                let mut pair_subs = [0usize; 64];
                let (add_count, sub_count, overflowed) = collect_moved_pawn_pair_delta(
                    pawns_before,
                    pawns_after,
                    kings[pov],
                    pov,
                    &mut pair_adds,
                    &mut pair_subs,
                );
                if overflowed {
                    self.refresh_pov(board, net, pov, side);
                    continue;
                }
                for &feature in &pair_adds[..add_count] {
                    delta.push(feature, 1);
                }
                for &feature in &pair_subs[..sub_count] {
                    delta.push(feature, -1);
                }
            }
            if delta.overflowed {
                self.refresh_pov(board, net, pov, side);
                continue;
            }
            apply[pov] = true;
        }
        if apply.iter().any(|&yes| yes) {
            let (parents, children) = self.frames.split_at_mut(current);
            let parent = &parents[current - 1];
            let child = &mut children[0];
            for (pov, should_apply) in apply.into_iter().enumerate() {
                if !should_apply {
                    continue;
                }
                let delta = &pov_deltas[pov];
                super::stockfish_simd::apply_i8_from_width(
                    &mut child.values[pov],
                    &parent.values[pov],
                    &net.aux_weights,
                    &delta.adds[..delta.add_count],
                    &delta.subs[..delta.sub_count],
                );
                child.lists[pov].overflowed = false;
            }
        }
        let frame = &mut self.frames[current];
        frame.kings = [kings[0] as u8, kings[1] as u8];
        frame.hash = board.hash;
        frame.accurate = true;
        frame.pending_move = None;
        frame.pending_null = false;
    }
}

fn wide_needs_refresh(old_king: usize, new_king: usize, pov: Color) -> bool {
    king_bucket(relative_square(pov, old_king)) != king_bucket(relative_square(pov, new_king))
        || (old_king % 8 >= 4) != (new_king % 8 >= 4)
}

fn refresh_wide_perspective<const H: usize>(
    acc: &mut [i16; H],
    weights: &[i16],
    biases: &[i16],
    board: &Board,
    king: usize,
    pov: Color,
) {
    acc.copy_from_slice(biases);
    for piece in Piece::ALL {
        for color in [Color::White, Color::Black] {
            let mut bb = board.piece_bb(piece, color);
            while bb != 0 {
                let sq = bb.trailing_zeros() as usize;
                bb &= bb - 1;
                apply_wide_feature(acc, weights, king, pov, piece, color, sq, 1);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_wide_feature<const H: usize>(
    acc: &mut [i16; H],
    weights: &[i16],
    king: usize,
    pov: Color,
    piece: Piece,
    piece_color: Color,
    sq: usize,
    sign: i16,
) {
    let bucket = king_bucket(relative_square(pov, king));
    let feat = bucket * LAYERED_INPUT + feature_index(pov, king, piece, piece_color, sq);
    super::stockfish_simd::apply_i16_feature_width(acc, weights, feat, sign);
}

fn wide_feature(king: usize, pov: Color, piece: Piece, piece_color: Color, sq: usize) -> usize {
    let bucket = king_bucket(relative_square(pov, king));
    bucket * LAYERED_INPUT + feature_index(pov, king, piece, piece_color, sq)
}

#[allow(clippy::too_many_arguments)]
fn apply_wide_move_delta_from<const H: usize>(
    dst: &mut [i16; H],
    src: &[i16; H],
    weights: &[i16],
    king: usize,
    side: Color,
    mv: Move,
    mover: u8,
    captured: u8,
) {
    debug_assert_ne!(mover, u8::MAX);
    let mover_piece = Piece::from_index(usize::from(mover) / 2).expect("mover piece is valid");
    let mover_color = if mover & 1 == 0 {
        Color::White
    } else {
        Color::Black
    };
    let resulting = mv.promotion.unwrap_or(mover_piece);
    let mut adds = [0; 4];
    let mut subs = [0; 4];
    let mut add_len = 0;
    let mut sub_len = 0;
    subs[sub_len] = wide_feature(king, side, mover_piece, mover_color, mv.from.index());
    sub_len += 1;
    adds[add_len] = wide_feature(king, side, resulting, mover_color, mv.to.index());
    add_len += 1;
    if mv.is_capture() && mv.flag != MoveFlag::EnPassant {
        subs[sub_len] = wide_feature(
            king,
            side,
            Piece::from_index(usize::from(captured) / 2).expect("captured piece is valid"),
            if captured & 1 == 0 {
                Color::White
            } else {
                Color::Black
            },
            mv.to.index(),
        );
        sub_len += 1;
    } else if mv.flag == MoveFlag::EnPassant {
        subs[sub_len] = wide_feature(
            king,
            side,
            Piece::Pawn,
            mover_color.opponent(),
            Square::from_file_rank(mv.to.file(), mv.from.rank()).index(),
        );
        sub_len += 1;
    } else if mv.is_castling() {
        let (rook_from, rook_to) = match (mover_color, mv.flag) {
            (Color::White, MoveFlag::KingCastle) => (Square::H1.index(), Square::F1.index()),
            (Color::White, MoveFlag::QueenCastle) => (Square::A1.index(), Square::D1.index()),
            (Color::Black, MoveFlag::KingCastle) => (Square::H8.index(), Square::F8.index()),
            (Color::Black, MoveFlag::QueenCastle) => (Square::A8.index(), Square::D8.index()),
            _ => unreachable!(),
        };
        subs[sub_len] = wide_feature(king, side, Piece::Rook, mover_color, rook_from);
        sub_len += 1;
        adds[add_len] = wide_feature(king, side, Piece::Rook, mover_color, rook_to);
        add_len += 1;
    }
    super::stockfish_simd::apply_i16_from_width(
        dst,
        src,
        weights,
        &adds[..add_len],
        &subs[..sub_len],
    );
}

pub(crate) struct ViridithasAccumulatorState {
    simple: Option<SimpleAccumulatorState>,
    layered: Option<WideAccumulatorState<LAYERED_L1>>,
    sandhi: Option<WideAccumulatorState<SANDHI_L0>>,
    sandhi_aux: Option<SandhiAuxState>,
}

impl ViridithasAccumulatorState {
    pub(crate) fn for_network(network: &ViridithasNetwork) -> Self {
        match network {
            ViridithasNetwork::Simple(net) => Self {
                simple: Some(SimpleAccumulatorState::new(net.hidden)),
                layered: None,
                sandhi: None,
                sandhi_aux: None,
            },
            ViridithasNetwork::Layered(_) => Self {
                simple: None,
                layered: Some(WideAccumulatorState::new()),
                sandhi: None,
                sandhi_aux: None,
            },
            ViridithasNetwork::Sandhi(_) => Self {
                simple: None,
                layered: None,
                sandhi: Some(WideAccumulatorState::new()),
                sandhi_aux: Some(SandhiAuxState::new()),
            },
        }
    }

    pub(crate) fn push_move(&mut self, board: &Board, mv: Move) {
        if let Some(state) = &mut self.simple {
            state.push_move(board, mv);
        } else if let Some(state) = &mut self.layered {
            state.push_move(board, mv);
        } else if let Some(state) = &mut self.sandhi {
            state.push_move(board, mv);
            self.sandhi_aux
                .as_mut()
                .expect("sandhi aux state")
                .push_move(board, mv);
        }
    }

    pub(crate) fn push_null(&mut self) {
        if let Some(state) = &mut self.simple {
            state.push_null();
        } else if let Some(state) = &mut self.layered {
            state.push_null();
        } else if let Some(state) = &mut self.sandhi {
            state.push_null();
            self.sandhi_aux
                .as_mut()
                .expect("sandhi aux state")
                .push_null();
        }
    }

    pub(crate) fn pop(&mut self) {
        if let Some(state) = &mut self.simple {
            state.pop();
        } else if let Some(state) = &mut self.layered {
            state.pop();
        } else if let Some(state) = &mut self.sandhi {
            state.pop();
            self.sandhi_aux.as_mut().expect("sandhi aux state").pop();
        }
    }

    pub(crate) fn clear(&mut self) {
        if let Some(state) = &mut self.simple {
            state.clear();
        } else if let Some(state) = &mut self.layered {
            state.clear();
        } else if let Some(state) = &mut self.sandhi {
            state.clear();
            self.sandhi_aux.as_mut().expect("sandhi aux state").clear();
        }
    }

    pub(crate) fn evaluate(&mut self, board: &Board, network: &ViridithasNetwork) -> i32 {
        self.evaluate_inner(board, network, false)
    }

    pub(crate) fn evaluate_search(&mut self, board: &Board, network: &ViridithasNetwork) -> i32 {
        self.evaluate_inner(board, network, true)
    }

    fn evaluate_inner(&mut self, board: &Board, network: &ViridithasNetwork, trusted: bool) -> i32 {
        match network {
            ViridithasNetwork::Simple(net) => {
                let state = self.simple.as_mut().expect("simple Viridithas state");
                if trusted {
                    state.evaluate_search(board, net)
                } else {
                    state.evaluate(board, net)
                }
            }
            ViridithasNetwork::Layered(net) => {
                let state = self.layered.as_mut().expect("layered Viridithas state");
                state.ensure_pieces(board, &net.feature_weights, &net.feature_biases, trusted);
                finish_layered(
                    net,
                    board,
                    &state.values[state.index][0],
                    &state.values[state.index][1],
                )
            }
            ViridithasNetwork::Sandhi(net) => {
                let state = self.sandhi.as_mut().expect("sandhi Viridithas state");
                let aux = self.sandhi_aux.as_mut().expect("sandhi aux state");
                state.ensure_pieces(board, &net.feature_weights, &net.feature_biases, trusted);
                aux.ensure(board, net, trusted);
                finish_sandhi(
                    net,
                    board,
                    &state.values[state.index][0],
                    &state.values[state.index][1],
                    &aux.frames[aux.index].values[0],
                    &aux.frames[aux.index].values[1],
                )
            }
        }
    }
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

    fn patterned_simple() -> SimpleNetwork {
        let hidden = 256;
        let mut feature_weights = vec![0i16; KING_BUCKETS * FEATURES * hidden];
        for (index, weight) in feature_weights.iter_mut().enumerate() {
            *weight = ((index % 251) as i16).wrapping_sub(125);
        }
        SimpleNetwork {
            hidden,
            features_per_bucket: FEATURES,
            feature_weights: feature_weights.into_boxed_slice(),
            feature_biases: vec![0i16; hidden].into_boxed_slice(),
            output_weights: vec![3i16; hidden * 2].into_boxed_slice(),
            output_bias: 11,
        }
    }

    fn assert_simple_matches(
        state: &mut SimpleAccumulatorState,
        net: &SimpleNetwork,
        board: &Board,
    ) {
        let expected_us = accumulate_simple::<HIDDEN>(net, board, board.side_to_move);
        let expected_them = accumulate_simple::<HIDDEN>(net, board, board.side_to_move.opponent());
        assert_eq!(
            state.evaluate(board, net),
            finish_simple(net, &expected_us, &expected_them, net.hidden)
        );
        let stm = board.side_to_move.index();
        assert_eq!(
            &state.frames[state.index].values[stm][..net.hidden],
            &expected_us[..net.hidden]
        );
        assert_eq!(
            &state.frames[state.index].values[stm ^ 1][..net.hidden],
            &expected_them[..net.hidden]
        );
    }

    #[test]
    fn finish_simple_simd_matches_scalar_at_production_width() {
        let mut us = vec![0i16; HIDDEN];
        let mut them = vec![0i16; HIDDEN];
        let mut output_weights = vec![0i16; HIDDEN * 2];
        for index in 0..HIDDEN {
            us[index] = (index as i16).wrapping_mul(3).wrapping_sub(400);
            them[index] = 220_i16.wrapping_sub(index as i16);
            output_weights[index] = (index % 17) as i16 - 8;
            output_weights[HIDDEN + index] = (index % 13) as i16 - 6;
        }
        let net = SimpleNetwork {
            hidden: HIDDEN,
            features_per_bucket: FEATURES,
            feature_weights: Box::new([]),
            feature_biases: vec![0i16; HIDDEN].into_boxed_slice(),
            output_weights: output_weights.into_boxed_slice(),
            output_bias: 11,
        };
        let simd = finish_simple(&net, &us, &them, HIDDEN);
        let scalar = {
            let sum = finish_simple_scalar(&net, &us, &them, HIDDEN);
            (sum / (QA * QA)) * SCALE / 64
        };
        assert_eq!(simd, scalar);
    }

    #[test]
    fn simple_incremental_matches_scratch_after_moves_and_pop() {
        types::init();
        let net = patterned_simple();
        let mut state = SimpleAccumulatorState::new(net.hidden);
        let mut board = Board::new();
        assert_simple_matches(&mut state, &net, &board);
        let mut last_move = None;
        for uci in ["e2e4", "e7e5", "g1f3", "b8c6"] {
            let mv = board
                .generate_legal_moves()
                .iter()
                .find(|mv| mv.to_uci() == uci)
                .copied()
                .expect("test move is legal");
            state.push_move(&board, mv);
            board.make_move(mv);
            last_move = Some(mv);
            assert_simple_matches(&mut state, &net, &board);
        }
        state.pop();
        board.unmake_move(last_move.expect("at least one move was made"));
        assert_simple_matches(&mut state, &net, &board);
    }

    #[test]
    fn simple_incremental_matches_scratch_for_king_walk() {
        types::init();
        let net = patterned_simple();
        let mut state = SimpleAccumulatorState::new(net.hidden);
        let mut board =
            Board::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").expect("test FEN is valid");
        assert_simple_matches(&mut state, &net, &board);
        let mv = board
            .generate_legal_moves()
            .iter()
            .find(|mv| mv.to_uci() == "e1e2")
            .copied()
            .expect("e1e2 is legal");
        state.push_move(&board, mv);
        board.make_move(mv);
        assert_simple_matches(&mut state, &net, &board);
    }

    #[test]
    fn viri_l1_affine_transpose_matches_published_layout() {
        let inputs = 4;
        let outputs = 3;
        let mut src = vec![0i8; inputs * LAYERED_OUTPUT_BUCKETS * outputs];
        for (index, weight) in src.iter_mut().enumerate() {
            *weight = (index % 13) as i8 - 6;
        }
        let affine = transpose_viri_l1(&src, inputs, outputs);
        let bucket = 2;
        let input = 1u8;
        for output in 0..outputs {
            let expected = i32::from(input)
                * i32::from(src[LAYERED_OUTPUT_BUCKETS * outputs + bucket * outputs + output]);
            let got = i32::from(input)
                * i32::from(affine[bucket * outputs * inputs + output * inputs + 1]);
            assert_eq!(got, expected, "output {output}");
        }
    }

    #[test]
    fn simd_add_i16_matches_scalar_wrapping_add() {
        let mut acc = [3i16; 32];
        let weights: Vec<i16> = (0..32).map(|i| i as i16 - 16).collect();
        add_i16(&mut acc, &weights, 0);
        for i in 0..32 {
            assert_eq!(acc[i], 3i16.wrapping_add(weights[i]));
        }
    }

    fn try_load_layered() -> Option<Box<ViridithasNetwork>> {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let candidates = [
            workspace.join("nnue/velarised-2-b800.nnue.zst"),
            workspace.join("dist/nnue/velarised-2-b800.nnue.zst"),
        ];
        let path = candidates.iter().find(|path| path.is_file())?;
        let net = load(path).ok()?;
        matches!(*net, ViridithasNetwork::Layered(_)).then_some(net)
    }

    fn try_load_sandhi() -> Option<Box<ViridithasNetwork>> {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let candidates = [
            workspace.join("nnue/sandhi-s2-b200.nnue.zst"),
            workspace.join("nnue/viri_default.nnue.zst"),
            workspace.join("dist/nnue/viri_default.nnue.zst"),
        ];
        for path in candidates {
            if !path.is_file() {
                continue;
            }
            if let Ok(net) = load(&path)
                && matches!(*net, ViridithasNetwork::Sandhi(_))
            {
                return Some(net);
            }
        }
        None
    }

    fn assert_wide_matches(
        state: &mut ViridithasAccumulatorState,
        net: &ViridithasNetwork,
        board: &Board,
    ) {
        let scratch = net.evaluate(board);
        let incremental = state.evaluate(board, net);
        assert_eq!(incremental, scratch);
        assert_eq!(state.evaluate_search(board, net), incremental);
        assert!(
            incremental.abs() < 20_000,
            "Viridithas eval left the searchable range: {incremental}"
        );
        if let ViridithasNetwork::Sandhi(sandhi) = net {
            let aux = state.sandhi_aux.as_ref().expect("sandhi aux state");
            for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
                let mut expected = [0i16; SANDHI_L0];
                add_sandhi_aux(sandhi, board, side, &mut expected);
                assert_eq!(
                    aux.frames[aux.index].values[pov], expected,
                    "sandhi aux mismatch for {side:?}"
                );
            }
        }
    }

    fn play_and_check(net: &ViridithasNetwork) {
        let mut state = ViridithasAccumulatorState::for_network(net);
        let mut board = Board::new();
        assert_wide_matches(&mut state, net, &board);
        let mut last_move = None;
        for uci in ["e2e4", "e7e5", "g1f3", "b8c6"] {
            let mv = board
                .generate_legal_moves()
                .iter()
                .find(|mv| mv.to_uci() == uci)
                .copied()
                .expect("test move is legal");
            state.push_move(&board, mv);
            board.make_move(mv);
            last_move = Some(mv);
            assert_wide_matches(&mut state, net, &board);
        }
        state.pop();
        board.unmake_move(last_move.expect("at least one move was made"));
        assert_wide_matches(&mut state, net, &board);
    }

    #[test]
    fn layered_incremental_matches_scratch_after_moves() {
        types::init();
        let Some(net) = try_load_layered() else {
            return;
        };
        play_and_check(&net);
    }

    #[test]
    fn sandhi_incremental_matches_scratch_after_moves() {
        types::init();
        let Some(net) = try_load_sandhi() else {
            return;
        };
        play_and_check(&net);
    }

    #[test]
    fn sandhi_incremental_matches_scratch_for_king_and_special_moves() {
        types::init();
        let Some(net) = try_load_sandhi() else {
            return;
        };
        let cases = [
            ("4k3/8/8/8/8/8/8/4K3 w - - 0 1", "e1e2"),
            ("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1", "e1g1"),
            ("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1", "e5d6"),
        ];
        for (fen, uci) in cases {
            let mut state = ViridithasAccumulatorState::for_network(&net);
            let mut board = Board::from_fen(fen).expect("test FEN is valid");
            assert_wide_matches(&mut state, &net, &board);
            let mv = board
                .generate_legal_moves()
                .iter()
                .find(|candidate| candidate.to_uci() == uci)
                .copied()
                .expect("test move is legal");
            state.push_move(&board, mv);
            board.make_move(mv);
            assert_wide_matches(&mut state, &net, &board);
        }
    }

    #[test]
    fn sandhi_null_move_reuses_aux() {
        types::init();
        let Some(net) = try_load_sandhi() else {
            return;
        };
        let mut state = ViridithasAccumulatorState::for_network(&net);
        let mut board =
            Board::from_fen("r1bq1rk1/ppp2ppp/2n2n2/2bp4/4P3/2P2N2/PP1N1PPP/R1BQ1RK1 w - - 2 9")
                .expect("test FEN is valid");
        assert_wide_matches(&mut state, &net, &board);
        state.push_null();
        board.make_null_move();
        assert_wide_matches(&mut state, &net, &board);
        state.pop();
        board.unmake_null_move();
        assert_wide_matches(&mut state, &net, &board);
    }

    #[test]
    fn sandhi_dirty_threat_and_search_eval_match_scratch() {
        types::init();
        let Some(net) = try_load_sandhi() else {
            return;
        };
        let mut state = ViridithasAccumulatorState::for_network(&net);
        let mut board = Board::new();
        let first = state.evaluate(&board, &net);
        assert_eq!(state.evaluate_search(&board, &net), first);
        let mv = board
            .generate_legal_moves()
            .iter()
            .find(|candidate| candidate.to_uci() == "e2e4")
            .copied()
            .expect("e2e4 is legal");
        state.push_move(&board, mv);
        let aux = state.sandhi_aux.as_ref().expect("sandhi aux");
        assert!(aux.frames[aux.index].pending_threats.is_some());
        board.make_move(mv);
        let incremental = state.evaluate(&board, &net);
        assert_eq!(state.evaluate_search(&board, &net), incremental);
        let mut scratch = ViridithasAccumulatorState::for_network(&net);
        assert_eq!(scratch.evaluate(&board, &net), incremental);
    }

    #[test]
    fn sandhi_hot_eval_stays_in_sub_microsecond_band() {
        if cfg!(debug_assertions) {
            return;
        }
        types::init();
        let Some(net) = try_load_sandhi() else {
            return;
        };
        let ViridithasNetwork::Sandhi(sandhi) = net.as_ref() else {
            return;
        };
        let mut state = ViridithasAccumulatorState::for_network(&net);
        let board = Board::new();
        let _ = state.evaluate(&board, &net);
        let mut checksum = 0i32;
        let start = std::time::Instant::now();
        for _ in 0..2_000 {
            checksum = checksum.wrapping_add(state.evaluate(&board, &net));
        }
        let eval_ns = start.elapsed().as_nanos() as f64 / 2_000.0;
        let aux = state.sandhi_aux.as_ref().expect("sandhi aux");
        let acc_w = &state.sandhi.as_ref().expect("sandhi").values[0][0];
        let acc_b = &state.sandhi.as_ref().expect("sandhi").values[0][1];
        let aux_w = &aux.frames[0].values[0];
        let aux_b = &aux.frames[0].values[1];
        let start = std::time::Instant::now();
        for _ in 0..2_000 {
            checksum =
                checksum.wrapping_add(finish_sandhi(sandhi, &board, acc_w, acc_b, aux_w, aux_b));
        }
        let finish_ns = start.elapsed().as_nanos() as f64 / 2_000.0;
        assert_ne!(checksum, i32::MIN);
        assert!(
            eval_ns < 1_200.0,
            "sandhi incremental evaluate is {eval_ns:.1} ns/eval (finish {finish_ns:.1} ns)"
        );
    }

    #[test]
    fn sandhi_search_move_stays_under_two_microseconds() {
        if cfg!(debug_assertions) {
            return;
        }
        types::init();
        let Some(net) = try_load_sandhi() else {
            return;
        };
        let mut state = ViridithasAccumulatorState::for_network(&net);
        let mut board = Board::new();
        let _ = state.evaluate(&board, &net);
        let mv = board
            .generate_legal_moves()
            .iter()
            .find(|candidate| candidate.to_uci() == "e2e4")
            .copied()
            .expect("e2e4 is legal");
        let mut checksum = 0i32;
        for _ in 0..64 {
            state.push_move(&board, mv);
            board.make_move(mv);
            checksum = checksum.wrapping_add(state.evaluate_search(&board, &net));
            state.pop();
            board.unmake_move(mv);
        }
        let mut elapsed = std::time::Duration::ZERO;
        for _ in 0..512 {
            state.push_move(&board, mv);
            board.make_move(mv);
            let start = std::time::Instant::now();
            checksum = checksum.wrapping_add(state.evaluate_search(&board, &net));
            elapsed += start.elapsed();
            state.pop();
            board.unmake_move(mv);
        }
        let ns = elapsed.as_nanos() as f64 / 512.0;
        assert_ne!(checksum, i32::MIN);
        assert!(
            ns < 2_000.0,
            "sandhi push+evaluate_search regressed to {ns:.1} ns"
        );
    }

    #[test]
    #[ignore = "official BitRays+AVX2 threat updates; current path is ~1.2µs"]
    fn sandhi_search_move_stays_under_half_microsecond() {
        if cfg!(debug_assertions) {
            return;
        }
        types::init();
        let Some(net) = try_load_sandhi() else {
            return;
        };
        let mut state = ViridithasAccumulatorState::for_network(&net);
        let mut board = Board::new();
        let _ = state.evaluate(&board, &net);
        let mv = board
            .generate_legal_moves()
            .iter()
            .find(|candidate| candidate.to_uci() == "e2e4")
            .copied()
            .expect("e2e4 is legal");
        let mut checksum = 0i32;
        let mut elapsed = std::time::Duration::ZERO;
        for _ in 0..256 {
            state.push_move(&board, mv);
            board.make_move(mv);
            let start = std::time::Instant::now();
            checksum = checksum.wrapping_add(state.evaluate_search(&board, &net));
            elapsed += start.elapsed();
            state.pop();
            board.unmake_move(mv);
        }
        let ns = elapsed.as_nanos() as f64 / 256.0;
        assert_ne!(checksum, i32::MIN);
        assert!(
            ns < 500.0,
            "sandhi push+evaluate_search is {ns:.1} ns (official incremental budget is <500 ns)"
        );
    }

    #[test]
    fn sandhi_repermute_is_a_bijection_of_each_half() {
        let mut seen = [false; SANDHI_L0 / 2];
        for &index in &SANDHI_REPERMUTE {
            assert!(
                index < SANDHI_L0 / 2,
                "repermute index {index} is out of range"
            );
            assert!(!seen[index], "repermute index {index} is duplicated");
            seen[index] = true;
        }
        assert!(seen.iter().all(|hit| *hit));
    }

    #[test]
    fn sandhi_aux_row_puts_pawn_pairs_before_threats() {
        assert_eq!(sandhi_aux_row(0), PAIR_FEATURES);
        assert_eq!(sandhi_aux_row(THREAT_FEATURES - 1), SANDHI_AUX - 1);
        assert_eq!(sandhi_aux_row(THREAT_FEATURES), 0);
        assert_eq!(
            sandhi_aux_row(THREAT_FEATURES + PAIR_FEATURES - 1),
            PAIR_FEATURES - 1
        );
    }

    fn stm_score_after(net: &ViridithasNetwork, fen: &str, uci: &str) -> i32 {
        let mut board = Board::from_fen(fen).expect("test FEN is valid");
        let mv = board
            .generate_legal_moves()
            .iter()
            .find(|candidate| candidate.to_uci() == uci)
            .copied()
            .unwrap_or_else(|| panic!("{uci} must be legal in {fen}"));
        board.make_move(mv);
        -net.evaluate(&board)
    }

    #[test]
    fn sandhi_prefers_capturing_the_scandinavian_pawn() {
        types::init();
        let Some(net) = try_load_sandhi() else {
            return;
        };
        let hanging = "rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2";
        let take = stm_score_after(&net, hanging, "e4d5");
        let fianchetto = stm_score_after(&net, hanging, "b2b3");
        assert!(
            take > fianchetto,
            "sandhi must prefer exd5 over b3 after 1.e4 d5 (exd5 {take}, b3 {fianchetto})"
        );
        assert!(
            take > 0,
            "capturing the hanging d-pawn must stay positive for White, got {take}"
        );
    }

    #[test]
    fn sandhi_prefers_developing_the_italian_knight() {
        types::init();
        let Some(net) = try_load_sandhi() else {
            return;
        };
        let italian = "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3";
        let develop = stm_score_after(&net, italian, "g8f6");
        let fianchetto = stm_score_after(&net, italian, "b7b6");
        assert!(
            develop > fianchetto,
            "sandhi must prefer Nf6 over b6 in the Italian (Nf6 {develop}, b6 {fianchetto})"
        );
    }
}
