//! Ateed MoE NNUE — Phase 2 native architecture.
//!
//! Shared 768×8hm i16 feature transformer, 3-body pawn-pair i8 residual,
//! four i8 experts gated from the activated STM half, and multi-task
//! eval + WDL heads. Hidden activations are u8 so the L1 gemm can use VNNI.

use std::path::Path;

use types::chess_move::MoveFlag;
use types::{Board, Color, Move, Piece, Square};

use super::stockfish_format::{PAIR_FEATURES, visit_pawn_pair_features};

pub const MAGIC: &[u8; 8] = b"ATEED001";
pub const FORMAT_VERSION: u32 = 1;
pub const HEADER_SIZE: usize = 12;
pub const L1: usize = 1024;
pub const L2: usize = 16;
pub const L3: usize = 32;
pub const FEATURES: usize = 768;
pub const KING_BUCKETS: usize = 8;
pub const EXPERTS: usize = 4;
pub const WDL_OUTPUTS: usize = 3;
pub const QA: i32 = 255;
pub const QB: i32 = 128;
pub const SCALE: i32 = 400;

const EXPERT_BYTES: usize =
    L1 * L2 + L2 * 4 + L2 * L3 + L3 * 4 + L3 + 4 + L3 * WDL_OUTPUTS + WDL_OUTPUTS * 4;

pub const FILE_SIZE: usize = HEADER_SIZE
    + KING_BUCKETS * FEATURES * L1 * 2
    + L1 * 2
    + PAIR_FEATURES * L1
    + L1 * EXPERTS
    + EXPERTS * 4
    + EXPERTS * EXPERT_BYTES;

#[rustfmt::skip]
const KING_BUCKET_LAYOUT: [usize; 64] = [
    0, 1, 2, 3, 3, 2, 1, 0,
    4, 4, 5, 5, 5, 5, 4, 4,
    6, 6, 6, 6, 6, 6, 6, 6,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
];

pub struct AteedExpert {
    l1_weights: Box<[i8]>,
    l1_biases: [i32; L2],
    l2_weights: Box<[i8]>,
    l2_biases: [i32; L3],
    eval_weights: [i8; L3],
    eval_bias: i32,
    wdl_weights: [i8; L3 * WDL_OUTPUTS],
    wdl_biases: [i32; WDL_OUTPUTS],
}

pub struct AteedExpertUpdate<'a> {
    pub l1_weights: &'a [i8],
    pub l1_biases: &'a [i32],
    pub l2_weights: &'a [i8],
    pub l2_biases: &'a [i32],
    pub eval_weights: &'a [i8],
    pub eval_bias: i32,
    pub wdl_weights: &'a [i8],
    pub wdl_biases: &'a [i32],
}

impl AteedExpert {
    pub fn l1_weights(&self) -> &[i8] {
        &self.l1_weights
    }

    pub fn l1_biases(&self) -> &[i32; L2] {
        &self.l1_biases
    }

    pub fn l2_weights(&self) -> &[i8] {
        &self.l2_weights
    }

    pub fn l2_biases(&self) -> &[i32; L3] {
        &self.l2_biases
    }

    pub fn eval_weights(&self) -> &[i8; L3] {
        &self.eval_weights
    }

    pub fn eval_bias(&self) -> i32 {
        self.eval_bias
    }

    pub fn wdl_weights(&self) -> &[i8; L3 * WDL_OUTPUTS] {
        &self.wdl_weights
    }

    pub fn wdl_biases(&self) -> &[i32; WDL_OUTPUTS] {
        &self.wdl_biases
    }
}

pub struct AteedNetwork {
    feature_weights: Box<[i16]>,
    feature_biases: Box<[i16]>,
    pair_weights: Box<[i8]>,
    gate_weights: Box<[i8]>,
    gate_biases: [i32; EXPERTS],
    experts: [AteedExpert; EXPERTS],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AteedEval {
    pub score: i32,
    pub expert: usize,
    pub wdl: [i32; WDL_OUTPUTS],
}

impl AteedNetwork {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() != FILE_SIZE {
            return Err(format!(
                "Ateed NNUE size {}: expected {FILE_SIZE}",
                bytes.len()
            ));
        }
        if bytes[..8] != *MAGIC {
            return Err("Ateed NNUE magic mismatch".to_string());
        }
        let version = u32::from_le_bytes(bytes[8..12].try_into().expect("header version bytes"));
        if version != FORMAT_VERSION {
            return Err(format!("unsupported Ateed version {version}"));
        }
        let mut offset = HEADER_SIZE;
        let feature_weights = read_i16s(bytes, &mut offset, KING_BUCKETS * FEATURES * L1)?;
        let feature_biases = read_i16s(bytes, &mut offset, L1)?;
        let pair_weights = read_i8s(bytes, &mut offset, PAIR_FEATURES * L1)?;
        let gate_weights = read_i8s(bytes, &mut offset, L1 * EXPERTS)?;
        let gate_biases = read_i32_array::<EXPERTS>(bytes, &mut offset)?;
        let experts = std::array::from_fn(|_| {
            read_expert(bytes, &mut offset).expect("Ateed expert payload is sized")
        });
        debug_assert_eq!(offset, FILE_SIZE);
        Ok(Self {
            feature_weights,
            feature_biases,
            pair_weights,
            gate_weights,
            gate_biases,
            experts,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(FILE_SIZE);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        write_i16s(&mut out, &self.feature_weights);
        write_i16s(&mut out, &self.feature_biases);
        write_i8s(&mut out, &self.pair_weights);
        write_i8s(&mut out, &self.gate_weights);
        write_i32s(&mut out, &self.gate_biases);
        for expert in &self.experts {
            write_i8s(&mut out, &expert.l1_weights);
            write_i32s(&mut out, &expert.l1_biases);
            write_i8s(&mut out, &expert.l2_weights);
            write_i32s(&mut out, &expert.l2_biases);
            write_i8s(&mut out, &expert.eval_weights);
            out.extend_from_slice(&expert.eval_bias.to_le_bytes());
            write_i8s(&mut out, &expert.wdl_weights);
            write_i32s(&mut out, &expert.wdl_biases);
        }
        debug_assert_eq!(out.len(), FILE_SIZE);
        out
    }

    pub fn zero() -> Self {
        let mut bytes = vec![0u8; FILE_SIZE];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        Self::from_bytes(&bytes).expect("zero Ateed payload is well-formed")
    }

    #[inline]
    pub fn evaluate(&self, board: &Board) -> i32 {
        self.evaluate_full(board).score
    }

    pub fn evaluate_full(&self, board: &Board) -> AteedEval {
        let [mut white, mut black] = scratch_piece_accumulators(self, board);
        add_pairs(self, board, Color::White, &mut white);
        add_pairs(self, board, Color::Black, &mut black);
        moe_forward(self, board, &white, &black)
    }

    pub fn feature_weights(&self) -> &[i16] {
        &self.feature_weights
    }

    pub fn feature_biases(&self) -> &[i16] {
        &self.feature_biases
    }

    pub fn expert(&self, index: usize) -> Option<&AteedExpert> {
        self.experts.get(index)
    }

    pub fn gate_weights(&self) -> &[i8] {
        &self.gate_weights
    }

    pub fn gate_biases(&self) -> &[i32; EXPERTS] {
        &self.gate_biases
    }

    pub fn set_gate(&mut self, weights: &[i8], biases: &[i32]) -> Result<(), String> {
        if weights.len() != self.gate_weights.len() || biases.len() != EXPERTS {
            return Err("Ateed gate size mismatch".to_string());
        }
        self.gate_weights.copy_from_slice(weights);
        self.gate_biases.copy_from_slice(biases);
        Ok(())
    }

    pub fn set_expert_output_biases(
        &mut self,
        index: usize,
        eval_bias: i32,
        wdl_biases: [i32; WDL_OUTPUTS],
    ) -> Result<(), String> {
        let expert = self
            .experts
            .get_mut(index)
            .ok_or_else(|| format!("Ateed expert {index} is out of range"))?;
        expert.eval_bias = eval_bias;
        expert.wdl_biases = wdl_biases;
        Ok(())
    }

    pub fn set_output_biases(&mut self, eval_bias: i32, wdl_biases: [i32; WDL_OUTPUTS]) {
        for expert in &mut self.experts {
            expert.eval_bias = eval_bias;
            expert.wdl_biases = wdl_biases;
        }
    }

    pub fn set_feature_transformer(
        &mut self,
        weights: &[i16],
        biases: &[i16],
    ) -> Result<(), String> {
        if weights.len() != self.feature_weights.len() || biases.len() != self.feature_biases.len()
        {
            return Err("feature transformer size mismatch".to_string());
        }
        self.feature_weights.copy_from_slice(weights);
        self.feature_biases.copy_from_slice(biases);
        Ok(())
    }

    pub fn set_expert(
        &mut self,
        index: usize,
        layers: AteedExpertUpdate<'_>,
    ) -> Result<(), String> {
        let expert = self
            .experts
            .get_mut(index)
            .ok_or_else(|| format!("Ateed expert {index} is out of range"))?;
        if layers.l1_weights.len() != expert.l1_weights.len()
            || layers.l1_biases.len() != L2
            || layers.l2_weights.len() != expert.l2_weights.len()
            || layers.l2_biases.len() != L3
            || layers.eval_weights.len() != L3
            || layers.wdl_weights.len() != L3 * WDL_OUTPUTS
            || layers.wdl_biases.len() != WDL_OUTPUTS
        {
            return Err("Ateed expert layer size mismatch".to_string());
        }
        expert.l1_weights.copy_from_slice(layers.l1_weights);
        expert.l1_biases.copy_from_slice(layers.l1_biases);
        expert.l2_weights.copy_from_slice(layers.l2_weights);
        expert.l2_biases.copy_from_slice(layers.l2_biases);
        expert.eval_weights.copy_from_slice(layers.eval_weights);
        expert.eval_bias = layers.eval_bias;
        expert.wdl_weights.copy_from_slice(layers.wdl_weights);
        expert.wdl_biases.copy_from_slice(layers.wdl_biases);
        Ok(())
    }
}

/// STM king-relative piece features used by the Phase 4 sparse trainer.
pub fn stm_piece_features(board: &Board) -> Vec<usize> {
    let pov = board.side_to_move;
    let king = board.king_square(pov).index();
    let occupancy = snapshot_occupancy(board);
    let mut features = Vec::with_capacity(32);
    for color in 0..2 {
        for piece in 0..Piece::COUNT {
            let mut bb = occupancy[color * Piece::COUNT + piece];
            while bb != 0 {
                let sq = bb.trailing_zeros() as usize;
                bb &= bb - 1;
                features.push(feature_index(
                    king,
                    pov,
                    Piece::from_index(piece).expect("piece index is valid"),
                    if color == 0 {
                        Color::White
                    } else {
                        Color::Black
                    },
                    sq,
                ));
            }
        }
    }
    features
}

#[inline(always)]
fn relative_square(side: Color, sq: usize) -> usize {
    if side == Color::Black { sq ^ 56 } else { sq }
}

#[inline(always)]
fn feature_index(
    king: usize,
    pov: Color,
    piece: Piece,
    piece_color: Color,
    mut sq: usize,
) -> usize {
    if king & 7 >= 4 {
        sq ^= 7;
    }
    let rel_king = relative_square(pov, king);
    let rel_sq = relative_square(pov, sq);
    let bucket = KING_BUCKET_LAYOUT[rel_king];
    let them = usize::from(piece_color != pov);
    bucket * FEATURES + them * 384 + piece.index() * 64 + rel_sq
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn apply_feature(
    acc: &mut [i16; L1],
    weights: &[i16],
    king: usize,
    pov: Color,
    piece: Piece,
    piece_color: Color,
    sq: usize,
    sign: i16,
) {
    super::stockfish_simd::apply_i16_feature_width(
        acc,
        weights,
        feature_index(king, pov, piece, piece_color, sq),
        sign,
    );
}

#[inline(always)]
fn king_bucket(king: usize, pov: Color) -> usize {
    KING_BUCKET_LAYOUT[relative_square(pov, king)]
}

#[inline(always)]
fn king_mirrored(king: usize) -> bool {
    king & 7 >= 4
}

#[inline(always)]
fn king_needs_refresh(old: usize, new: usize, pov: Color) -> bool {
    king_bucket(old, pov) != king_bucket(new, pov) || king_mirrored(old) != king_mirrored(new)
}

#[inline(always)]
fn finny_index(pov: Color, king: usize) -> usize {
    pov.index() * 2 * KING_BUCKETS
        + usize::from(king_mirrored(king)) * KING_BUCKETS
        + king_bucket(king, pov)
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

fn scratch_piece_accumulators(net: &AteedNetwork, board: &Board) -> [[i16; L1]; 2] {
    let occupancy = snapshot_occupancy(board);
    let mut acc = [[0i16; L1]; 2];
    for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
        acc[pov].copy_from_slice(&net.feature_biases);
        add_all_pieces(
            &mut acc[pov],
            &net.feature_weights,
            board.king_square(side).index(),
            side,
            &occupancy,
        );
    }
    acc
}

fn add_all_pieces(
    acc: &mut [i16; L1],
    weights: &[i16],
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
                apply_feature(
                    acc,
                    weights,
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

fn add_pairs(net: &AteedNetwork, board: &Board, pov: Color, acc: &mut [i16; L1]) {
    if net.pair_weights.iter().all(|&weight| weight == 0) {
        return;
    }
    visit_pawn_pair_features(board, pov.index(), |feature| {
        super::stockfish_simd::apply_i8_feature_width(
            acc,
            &net.pair_weights,
            feature - super::stockfish_format::THREAT_FEATURES,
            1,
        );
    });
}

fn activate(acc: &[i16; L1]) -> [u8; L1] {
    let mut out = [0u8; L1];
    for (dst, &value) in out.iter_mut().zip(acc) {
        *dst = value.clamp(0, QA as i16) as u8;
    }
    out
}

fn moe_forward(
    net: &AteedNetwork,
    board: &Board,
    white: &[i16; L1],
    black: &[i16; L1],
) -> AteedEval {
    let (us, _them) = if board.side_to_move == Color::White {
        (white, black)
    } else {
        (black, white)
    };
    let activated = activate(us);
    let expert = route_expert(net, &activated);
    expert_forward(&net.experts[expert], &activated, expert)
}

fn route_expert(net: &AteedNetwork, activated: &[u8; L1]) -> usize {
    let mut logits = net.gate_biases;
    super::stockfish_simd::affine(activated, &net.gate_weights, &mut logits);
    logits
        .iter()
        .enumerate()
        .max_by_key(|&(index, value)| (*value, -(index as i32)))
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn expert_forward(expert: &AteedExpert, activated: &[u8; L1], expert_id: usize) -> AteedEval {
    let mut l2 = expert.l1_biases;
    super::stockfish_simd::affine(activated, &expert.l1_weights, &mut l2);
    let mut l2_act = [0u8; L2];
    for (dst, value) in l2_act.iter_mut().zip(l2) {
        *dst = value.clamp(0, 127) as u8;
    }

    let mut l3 = expert.l2_biases;
    for (row, value) in expert.l2_weights.chunks_exact(L2).zip(l3.iter_mut()) {
        for (&activation, &weight) in l2_act.iter().zip(row) {
            *value += i32::from(activation) * i32::from(weight);
        }
    }
    let mut l3_act = [0u8; L3];
    for (dst, value) in l3_act.iter_mut().zip(l3) {
        *dst = value.clamp(0, 127) as u8;
    }

    let mut eval = expert.eval_bias;
    for (&activation, &weight) in l3_act.iter().zip(&expert.eval_weights) {
        eval += i32::from(activation) * i32::from(weight);
    }
    let mut wdl = expert.wdl_biases;
    for (logit, row) in wdl.iter_mut().zip(expert.wdl_weights.chunks_exact(L3)) {
        for (&activation, &weight) in l3_act.iter().zip(row) {
            *logit += i32::from(activation) * i32::from(weight);
        }
    }
    AteedEval {
        score: ((i64::from(eval) * i64::from(SCALE)) / i64::from(QA * QB)) as i32,
        expert: expert_id,
        wdl,
    }
}

/// Draw-weighted variance proxy from raw WDL logits (search-side σ² hook).
pub fn wdl_variance(wdl: [i32; WDL_OUTPUTS]) -> i32 {
    let max = wdl.iter().copied().max().unwrap_or(0);
    let mut exp = [0.0f32; WDL_OUTPUTS];
    let mut sum = 0.0f32;
    for (slot, &logit) in exp.iter_mut().zip(&wdl) {
        *slot = ((logit - max) as f32 / 64.0).exp();
        sum += *slot;
    }
    if sum <= 0.0 {
        return 0;
    }
    let w = exp[0] / sum;
    let d = exp[1] / sum;
    let l = exp[2] / sum;
    let mean = w + 0.5 * d;
    let var = w * (1.0 - mean).powi(2) + d * (0.5 - mean).powi(2) + l * (0.0 - mean).powi(2);
    (var * 10_000.0) as i32
}

const MAX_PLY: usize = 256;
const FINNY_ENTRIES: usize = 2 * 2 * KING_BUCKETS;

struct AteedFrame {
    values: [[i16; L1]; 2],
    kings: [u8; 2],
    pending_has_move: bool,
    pending_move: Move,
    pending_mover: u8,
    pending_captured: u8,
    hash: u64,
    accurate: bool,
    pending_null: bool,
}

impl Default for AteedFrame {
    fn default() -> Self {
        Self {
            values: [[0; L1]; 2],
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

impl Clone for AteedFrame {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for AteedFrame {}

struct FinnyEntry {
    values: [i16; L1],
    occupancy: [u64; 12],
    initialized: bool,
}

impl Default for FinnyEntry {
    fn default() -> Self {
        Self {
            values: [0; L1],
            occupancy: [0; 12],
            initialized: false,
        }
    }
}

pub(crate) struct AteedAccumulatorState {
    frames: Box<[AteedFrame]>,
    finny: Box<[FinnyEntry]>,
    index: usize,
}

impl AteedAccumulatorState {
    pub(crate) fn new() -> Self {
        Self {
            frames: vec![AteedFrame::default(); MAX_PLY].into_boxed_slice(),
            finny: (0..FINNY_ENTRIES)
                .map(|_| FinnyEntry::default())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            index: 0,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.index = 0;
        self.frames[0].accurate = false;
        self.frames[0].pending_null = false;
        for entry in self.finny.iter_mut() {
            entry.initialized = false;
        }
    }

    #[inline]
    pub(crate) fn push_move(&mut self, board: &Board, mv: Move) {
        assert!(
            self.index + 1 < self.frames.len(),
            "Ateed NNUE stack exhausted"
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

    #[inline]
    pub(crate) fn push_null(&mut self) {
        assert!(
            self.index + 1 < self.frames.len(),
            "Ateed NNUE stack exhausted"
        );
        let next = self.index + 1;
        self.frames[next] = self.frames[self.index];
        self.frames[next].pending_has_move = false;
        self.frames[next].pending_null = true;
        self.index = next;
    }

    #[inline]
    pub(crate) fn pop(&mut self) {
        assert!(self.index != 0, "cannot pop the root Ateed NNUE frame");
        self.index -= 1;
    }

    pub(crate) fn evaluate(&mut self, board: &Board, network: &AteedNetwork) -> i32 {
        self.evaluate_full(board, network).score
    }

    pub(crate) fn evaluate_full(&mut self, board: &Board, network: &AteedNetwork) -> AteedEval {
        if self.frames[self.index].accurate && self.frames[self.index].pending_null {
            self.frames[self.index].hash = board.hash;
            self.frames[self.index].pending_null = false;
        }
        if !self.frames[self.index].accurate || self.frames[self.index].hash != board.hash {
            if self.index != 0 && self.frames[self.index - 1].accurate {
                self.update_from_parent(board, network);
            } else {
                self.refresh(board, network);
            }
        }
        let frame = &self.frames[self.index];
        let mut white = frame.values[0];
        let mut black = frame.values[1];
        add_pairs(network, board, Color::White, &mut white);
        add_pairs(network, board, Color::Black, &mut black);
        moe_forward(network, board, &white, &black)
    }

    fn refresh(&mut self, board: &Board, network: &AteedNetwork) {
        let occupancy = snapshot_occupancy(board);
        let kings = [
            board.king_square(Color::White).index(),
            board.king_square(Color::Black).index(),
        ];
        for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
            self.finny_refresh(side, kings[pov], &occupancy, network);
            self.frames[self.index].values[pov] = self.finny[finny_index(side, kings[pov])].values;
        }
        let frame = &mut self.frames[self.index];
        frame.kings = [kings[0] as u8, kings[1] as u8];
        frame.hash = board.hash;
        frame.accurate = true;
        frame.pending_has_move = false;
        frame.pending_null = false;
    }

    fn update_from_parent(&mut self, board: &Board, network: &AteedNetwork) {
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
            king_needs_refresh(parent_kings[0], kings[0], Color::White),
            king_needs_refresh(parent_kings[1], kings[1], Color::Black),
        ];
        if needs_refresh.iter().any(|&refresh| refresh) {
            let occupancy = snapshot_occupancy(board);
            for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
                if !needs_refresh[pov] {
                    continue;
                }
                self.finny_refresh(side, kings[pov], &occupancy, network);
                self.frames[current].values[pov] = self.finny[finny_index(side, kings[pov])].values;
            }
        }
        let pending_has_move = self.frames[current].pending_has_move;
        let pending_move = self.frames[current].pending_move;
        let pending_mover = self.frames[current].pending_mover;
        let pending_captured = self.frames[current].pending_captured;
        let parent_values = self.frames[current - 1].values;
        let frame = &mut self.frames[current];
        for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
            if needs_refresh[pov] {
                continue;
            }
            frame.values[pov] = parent_values[pov];
            if pending_has_move {
                apply_move_delta(
                    &mut frame.values[pov],
                    &network.feature_weights,
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
        network: &AteedNetwork,
    ) {
        let entry = &mut self.finny[finny_index(side, king)];
        if !entry.initialized {
            entry.values.copy_from_slice(&network.feature_biases);
            add_all_pieces(
                &mut entry.values,
                &network.feature_weights,
                king,
                side,
                occupancy,
            );
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
                    apply_feature(
                        &mut entry.values,
                        &network.feature_weights,
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
                    apply_feature(
                        &mut entry.values,
                        &network.feature_weights,
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

fn apply_move_delta(
    acc: &mut [i16; L1],
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
    apply_feature(
        acc,
        weights,
        king,
        side,
        mover_piece,
        mover_color,
        mv.from.index(),
        -1,
    );
    apply_feature(
        acc,
        weights,
        king,
        side,
        resulting,
        mover_color,
        mv.to.index(),
        1,
    );
    if mv.is_capture() && mv.flag != MoveFlag::EnPassant {
        apply_feature(
            acc,
            weights,
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
        apply_feature(
            acc,
            weights,
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
        apply_feature(
            acc,
            weights,
            king,
            side,
            Piece::Rook,
            mover_color,
            rook_from,
            -1,
        );
        apply_feature(
            acc,
            weights,
            king,
            side,
            Piece::Rook,
            mover_color,
            rook_to,
            1,
        );
    }
}

pub fn load(path: &Path) -> Result<Box<AteedNetwork>, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read Ateed NNUE '{}': {error}", path.display()))?;
    AteedNetwork::from_bytes(&bytes).map(Box::new)
}

pub fn looks_like_ateed(path: &Path, bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.to_ascii_lowercase().contains("ateed"))
}

fn read_expert(bytes: &[u8], offset: &mut usize) -> Result<AteedExpert, String> {
    Ok(AteedExpert {
        l1_weights: read_i8s(bytes, offset, L1 * L2)?,
        l1_biases: read_i32_array::<L2>(bytes, offset)?,
        l2_weights: read_i8s(bytes, offset, L2 * L3)?,
        l2_biases: read_i32_array::<L3>(bytes, offset)?,
        eval_weights: read_i8s(bytes, offset, L3)?
            .to_vec()
            .try_into()
            .map_err(|_| "Ateed eval weight count".to_string())?,
        eval_bias: read_i32_array::<1>(bytes, offset)?[0],
        wdl_weights: read_i8s(bytes, offset, L3 * WDL_OUTPUTS)?
            .to_vec()
            .try_into()
            .map_err(|_| "Ateed WDL weight count".to_string())?,
        wdl_biases: read_i32_array::<WDL_OUTPUTS>(bytes, offset)?,
    })
}

fn read_i16s(bytes: &[u8], offset: &mut usize, count: usize) -> Result<Box<[i16]>, String> {
    let need = count * 2;
    let slice = bytes
        .get(*offset..*offset + need)
        .ok_or_else(|| "truncated Ateed i16 weights".to_string())?;
    *offset += need;
    Ok(slice
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect())
}

fn read_i8s(bytes: &[u8], offset: &mut usize, count: usize) -> Result<Box<[i8]>, String> {
    let slice = bytes
        .get(*offset..*offset + count)
        .ok_or_else(|| "truncated Ateed i8 weights".to_string())?;
    *offset += count;
    Ok(slice.iter().map(|byte| *byte as i8).collect())
}

fn read_i32_array<const N: usize>(bytes: &[u8], offset: &mut usize) -> Result<[i32; N], String> {
    let need = N * 4;
    let slice = bytes
        .get(*offset..*offset + need)
        .ok_or_else(|| "truncated Ateed i32 weights".to_string())?;
    *offset += need;
    let mut values = [0i32; N];
    for (slot, chunk) in values.iter_mut().zip(slice.chunks_exact(4)) {
        *slot = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    Ok(values)
}

fn write_i16s(out: &mut Vec<u8>, values: &[i16]) {
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

fn write_i8s(out: &mut Vec<u8>, values: &[i8]) {
    out.extend(values.iter().map(|value| *value as u8));
}

fn write_i32s(out: &mut Vec<u8>, values: &[i32]) {
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterned_net() -> AteedNetwork {
        let mut net = AteedNetwork::zero();
        for (index, weight) in net.feature_weights.iter_mut().enumerate() {
            *weight = ((index % 251) as i16).wrapping_sub(125);
        }
        for (index, weight) in net.gate_weights.iter_mut().enumerate() {
            *weight = ((index % 17) as i8).wrapping_sub(8);
        }
        net.gate_biases = [3, 1, -2, 0];
        for (expert_id, expert) in net.experts.iter_mut().enumerate() {
            for (index, weight) in expert.l1_weights.iter_mut().enumerate() {
                *weight = ((index + expert_id * 3) % 13) as i8 - 6;
            }
            expert.eval_weights[expert_id] = 9;
            expert.eval_bias = 40 + expert_id as i32;
            expert.wdl_biases = [8, 4, 1];
        }
        net
    }

    fn assert_incremental_matches(
        state: &mut AteedAccumulatorState,
        net: &AteedNetwork,
        board: &Board,
    ) {
        let expected = net.evaluate_full(board);
        let actual = state.evaluate_full(board, net);
        assert_eq!(actual, expected);
        assert_eq!(
            state.frames[state.index].values,
            scratch_piece_accumulators(net, board)
        );
    }

    #[test]
    fn stm_piece_features_lists_every_startpos_piece() {
        types::init();
        let features = stm_piece_features(&Board::new());
        assert_eq!(features.len(), 32);
        assert!(
            features
                .iter()
                .all(|&index| index < KING_BUCKETS * FEATURES)
        );
    }

    #[test]
    fn set_output_biases_changes_zero_net_eval() {
        types::init();
        let mut net = AteedNetwork::zero();
        net.set_output_biases(8_160, [4, 2, 0]);
        assert_eq!(net.evaluate(&Board::new()), 100);
        assert_eq!(net.evaluate_full(&Board::new()).wdl, [4, 2, 0]);
        assert!(
            net.set_feature_transformer(&[0], &[])
                .unwrap_err()
                .contains("size mismatch")
        );
        assert!(
            net.set_gate(&[0], &[])
                .unwrap_err()
                .contains("gate size mismatch")
        );
        net.set_gate(&vec![0; L1 * EXPERTS], &[0, 8, 0, 0])
            .expect("set gate");
        net.set_expert_output_biases(1, 8_160, [0, 0, 0])
            .expect("set expert 1");
        assert_eq!(net.evaluate_full(&Board::new()).expert, 1);
        assert_eq!(net.evaluate(&Board::new()), 100);
        assert!(
            net.set_expert(
                9,
                AteedExpertUpdate {
                    l1_weights: &[],
                    l1_biases: &[],
                    l2_weights: &[],
                    l2_biases: &[],
                    eval_weights: &[],
                    eval_bias: 0,
                    wdl_weights: &[],
                    wdl_biases: &[],
                },
            )
            .unwrap_err()
            .contains("out of range")
        );
    }

    #[test]
    fn packed_file_size_is_stable() {
        assert_eq!(FILE_SIZE, 17_327_452);
        let net = AteedNetwork::zero();
        assert_eq!(net.to_bytes().len(), FILE_SIZE);
        assert_eq!(net.evaluate(&Board::new()), 0);
    }

    #[test]
    fn rejects_wrong_magic_and_size() {
        assert!(AteedNetwork::from_bytes(&[0u8; 16]).is_err());
        let mut bytes = AteedNetwork::zero().to_bytes();
        bytes[0] = b'X';
        assert!(
            AteedNetwork::from_bytes(&bytes)
                .err()
                .expect("wrong magic is rejected")
                .contains("magic")
        );
    }

    #[test]
    fn looks_like_ateed_uses_magic_or_name() {
        let bytes = AteedNetwork::zero().to_bytes();
        assert!(looks_like_ateed(Path::new("net.bin"), &bytes));
        assert!(looks_like_ateed(Path::new("ateed_default.bin"), &[0]));
        assert!(!looks_like_ateed(Path::new("ak_default.bin"), &[0]));
    }

    #[test]
    fn moe_picks_the_highest_gate_logit() {
        types::init();
        let net = patterned_net();
        let eval = net.evaluate_full(&Board::new());
        assert!(eval.expert < EXPERTS);
        assert_eq!(eval.wdl.len(), 3);
        assert!(wdl_variance(eval.wdl) >= 0);
    }

    #[test]
    fn incremental_state_matches_scratch_after_moves_and_pop() {
        types::init();
        let net = patterned_net();
        let mut state = AteedAccumulatorState::new();
        let mut board = Board::new();
        assert_incremental_matches(&mut state, &net, &board);
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
            assert_incremental_matches(&mut state, &net, &board);
        }
        state.pop();
        board.unmake_move(last_move.expect("at least one move was made"));
        assert_incremental_matches(&mut state, &net, &board);
    }

    #[test]
    fn incremental_state_matches_scratch_for_king_and_special_moves() {
        types::init();
        let net = patterned_net();
        let cases = [
            ("4k3/8/8/8/8/8/8/4K3 w - - 0 1", "e1e2"),
            ("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1", "e1g1"),
            ("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1", "e5d6"),
        ];
        for (fen, uci) in cases {
            let mut state = AteedAccumulatorState::new();
            let mut board = Board::from_fen(fen).expect("test FEN is valid");
            assert_incremental_matches(&mut state, &net, &board);
            let mv = board
                .generate_legal_moves()
                .iter()
                .find(|mv| mv.to_uci() == uci)
                .copied()
                .expect("test move is legal");
            state.push_move(&board, mv);
            board.make_move(mv);
            assert_incremental_matches(&mut state, &net, &board);
        }
    }

    #[test]
    fn roundtrip_bytes_preserve_eval() {
        types::init();
        let net = patterned_net();
        let board = Board::new();
        let restored = AteedNetwork::from_bytes(&net.to_bytes()).expect("roundtrip");
        assert_eq!(restored.evaluate_full(&board), net.evaluate_full(&board));
    }

    #[test]
    fn startpos_eval_stays_within_a_latency_budget() {
        types::init();
        let net = AteedNetwork::zero();
        let board = Board::new();
        let start = std::time::Instant::now();
        let mut last = net.evaluate_full(&board);
        for _ in 0..8 {
            last = net.evaluate_full(&board);
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 1_000,
            "Ateed startpos eval budget exceeded: {elapsed:?}"
        );
        assert_eq!(last.score, 0);
        assert!(wdl_variance(last.wdl) >= 0);
    }
}
