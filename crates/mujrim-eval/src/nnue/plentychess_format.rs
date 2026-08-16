//! PlentyChess 0179r NNUE (PSQ + pawn-pair + threat FT → 1024 → 16 → 32 → 1 ×8).
//!
//! Published `0179r.bin` files are SLEB128/ULEB128-compressed `tmp` weights from
//! PlentyChess `process_net` (`infile_is_floats=false`). L1 is stored as
//! `[bucket][output][input]` for the SIMD affine kernel. Search evaluates
//! through `PlentyChessAccumulatorState` only; scratch `Network::evaluate` is
//! the bit-exact reference. Threat / pawn-pair indices reuse the Stockfish
//! 59808+4560 scheme already in this crate.

use std::path::Path;

use types::chess_move::MoveFlag;
use types::{Board, Color, Move, Piece, Square};

use super::dirty_threats::{
    MAX_DIRTY_THREAT_DELTAS, ThreatDelta, ThreatDeltaSink, ThreatSnapshot,
    collect_snapshot_move_deltas,
};
use super::stockfish_format::{
    AuxFeatureLists, MAX_AUX_FEATURES, PAIR_FEATURES, THREAT_FEATURES, apply_diff,
    collect_aux_feature_lists, collect_pawn_pair_aux, visit_pawn_pair_features, visit_threat_delta,
    visit_threat_features,
};

pub const L1: usize = 1024;
pub const L2: usize = 16;
pub const L3: usize = 32;
pub const KING_BUCKETS: usize = 12;
pub const OUTPUT_BUCKETS: usize = 8;
pub const FEATURES: usize = 768;
pub const NETWORK_SCALE: i32 = 287;
pub const NETWORK_QA: i32 = 255;
pub const NETWORK_QB: i32 = 64;
const FT_SHIFT: i32 = 9;
const L1_NORMALISATION: f32 =
    ((1 << FT_SHIFT) as f32) / ((NETWORK_QA * NETWORK_QA * NETWORK_QB) as f32);

#[rustfmt::skip]
const KING_BUCKET_LAYOUT: [usize; 64] = [
    0, 1, 2, 3, 3, 2, 1, 0,
    4, 5, 6, 7, 7, 6, 5, 4,
    8, 8, 9, 9, 9, 9, 8, 8,
    10, 10, 10, 10, 10, 10, 10, 10,
    11, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 11, 11,
];

pub struct PlentyChessNetwork {
    psq_weights: Box<[i16]>,
    pawn_pair_weights: Box<[i8]>,
    threat_weights: Box<[i8]>,
    feature_biases: Box<[i16]>,
    l1_weights: Box<[i8]>,
    l1_biases: Box<[f32]>,
    l2_weights: Box<[f32]>,
    l2_biases: Box<[f32]>,
    l3_weights: Box<[f32]>,
    l3_biases: Box<[f32]>,
}

impl PlentyChessNetwork {
    pub fn from_compressed_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut pos = 0;
        let psq_weights = read_sleb_i16s(bytes, &mut pos, FEATURES * KING_BUCKETS * L1)?;
        let pawn_tmp = read_sleb_i8s(bytes, &mut pos, PAIR_FEATURES * L1)?;
        let threat_tmp = read_sleb_i8s(bytes, &mut pos, THREAT_FEATURES * L1)?;
        let feature_biases = read_sleb_i16s(bytes, &mut pos, L1)?;
        let l1_tmp = read_sleb_i8s(bytes, &mut pos, OUTPUT_BUCKETS * L1 * L2)?;
        let l1_biases = read_uleb_f32s(bytes, &mut pos, OUTPUT_BUCKETS * L2)?;
        let l2_tmp = read_uleb_f32s(bytes, &mut pos, OUTPUT_BUCKETS * (L2 * 2) * L3)?;
        let l2_biases = read_uleb_f32s(bytes, &mut pos, OUTPUT_BUCKETS * L3)?;
        let l3_tmp = read_uleb_f32s(bytes, &mut pos, OUTPUT_BUCKETS * (L3 + 2 * L2))?;
        let l3_biases = read_uleb_f32s(bytes, &mut pos, OUTPUT_BUCKETS)?;
        if pos != bytes.len() {
            return Err(format!(
                "PlentyChess NNUE leftover bytes: decoded {pos}, file {}",
                bytes.len()
            ));
        }

        let mut l1_weights = vec![0i8; OUTPUT_BUCKETS * L2 * L1].into_boxed_slice();
        for bucket in 0..OUTPUT_BUCKETS {
            for l1 in 0..L1 {
                for l2 in 0..L2 {
                    l1_weights[bucket * L2 * L1 + l2 * L1 + l1] =
                        l1_tmp[l1 * OUTPUT_BUCKETS * L2 + bucket * L2 + l2];
                }
            }
        }

        let mut l2_weights = vec![0f32; OUTPUT_BUCKETS * (L2 * 2) * L3].into_boxed_slice();
        for bucket in 0..OUTPUT_BUCKETS {
            for l2 in 0..(L2 * 2) {
                for l3 in 0..L3 {
                    l2_weights[bucket * (L2 * 2) * L3 + l2 * L3 + l3] =
                        l2_tmp[l2 * OUTPUT_BUCKETS * L3 + bucket * L3 + l3];
                }
            }
        }

        let mut l3_weights = vec![0f32; OUTPUT_BUCKETS * (L3 + 2 * L2)].into_boxed_slice();
        for bucket in 0..OUTPUT_BUCKETS {
            for l3 in 0..(L3 + 2 * L2) {
                l3_weights[bucket * (L3 + 2 * L2) + l3] = l3_tmp[l3 * OUTPUT_BUCKETS + bucket];
            }
        }

        let l1_weights =
            super::layered_forward::pack_nnz_buckets(&l1_weights, OUTPUT_BUCKETS, L1, L2);

        Ok(Self {
            psq_weights,
            pawn_pair_weights: pawn_tmp,
            threat_weights: threat_tmp,
            feature_biases,
            l1_weights,
            l1_biases,
            l2_weights,
            l2_biases,
            l3_weights,
            l3_biases,
        })
    }

    #[inline(always)]
    pub fn evaluate(&self, board: &Board) -> i32 {
        let [mut acc_white, mut acc_black] = scratch_piece_accumulators(self, board);
        add_aux(self, board, Color::White, &mut acc_white);
        add_aux(self, board, Color::Black, &mut acc_black);
        finish_eval(self, board, &acc_white, &acc_black)
    }
}

#[inline(always)]
fn psq_feature(king: usize, pov: Color, piece: Piece, piece_color: Color, sq: usize) -> usize {
    let bucket = KING_BUCKET_LAYOUT[king ^ (56 * pov.index())];
    let mirror = king & 7 >= 4;
    let oriented = sq ^ (7 * usize::from(mirror)) ^ (56 * pov.index());
    let them = usize::from(piece_color != pov);
    bucket * FEATURES + them * 384 + piece.index() * 64 + oriented
}

#[inline(always)]
fn king_bucket(king: usize, pov: Color) -> usize {
    KING_BUCKET_LAYOUT[king ^ (56 * pov.index())]
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

fn scratch_piece_accumulators(net: &PlentyChessNetwork, board: &Board) -> [[i16; L1]; 2] {
    let occupancy = snapshot_occupancy(board);
    let mut acc = [[0i16; L1]; 2];
    for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
        acc[pov].copy_from_slice(&net.feature_biases);
        add_all_pieces(
            &mut acc[pov],
            &net.psq_weights,
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
                apply_psq(
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

#[allow(clippy::too_many_arguments)]
fn apply_psq(
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
        psq_feature(king, pov, piece, piece_color, sq),
        sign,
    );
}

fn aux_enabled(net: &PlentyChessNetwork) -> bool {
    !net.pawn_pair_weights.is_empty() || !net.threat_weights.is_empty()
}

fn add_aux(net: &PlentyChessNetwork, board: &Board, pov: Color, acc: &mut [i16; L1]) {
    if !net.pawn_pair_weights.is_empty() {
        visit_pawn_pair_features(board, pov.index(), |feature| {
            apply_aux_pair(acc, net, feature, 1);
        });
    }
    if !net.threat_weights.is_empty() {
        visit_threat_features(board, pov.index(), |feature| {
            apply_aux_threat(acc, net, feature, 1);
        });
    }
}

fn apply_aux_threat(acc: &mut [i16; L1], net: &PlentyChessNetwork, feature: usize, sign: i16) {
    if net.threat_weights.is_empty() {
        return;
    }
    super::stockfish_simd::apply_i8_feature_width(acc, &net.threat_weights, feature, sign);
}

fn apply_aux_pair(acc: &mut [i16; L1], net: &PlentyChessNetwork, feature: usize, sign: i16) {
    if net.pawn_pair_weights.is_empty() {
        return;
    }
    super::stockfish_simd::apply_i8_feature_width(
        acc,
        &net.pawn_pair_weights,
        feature - THREAT_FEATURES,
        sign,
    );
}

fn collect_aux_lists(board: &Board, pov: Color) -> AuxFeatureLists {
    collect_aux_feature_lists(board, pov.index())
}

fn apply_aux_lists(acc: &mut [i16; L1], net: &PlentyChessNetwork, lists: &AuxFeatureLists) {
    if lists.overflowed {
        return;
    }
    for &feature in &lists.threats[..lists.threat_count] {
        apply_aux_threat(acc, net, usize::from(feature), 1);
    }
    for &feature in &lists.pairs[..lists.pair_count] {
        apply_aux_pair(acc, net, usize::from(feature), 1);
    }
}

fn finish_eval_sum(
    net: &PlentyChessNetwork,
    board: &Board,
    acc_white: &[i16; L1],
    acc_black: &[i16; L1],
    aux_white: &[i16; L1],
    aux_black: &[i16; L1],
) -> i32 {
    let pieces = board.all_occupancy().count_ones() as i32;
    let divisor = (32 + OUTPUT_BUCKETS as i32 - 1) / OUTPUT_BUCKETS as i32;
    let bucket = ((pieces - 2) / divisor).clamp(0, OUTPUT_BUCKETS as i32 - 1) as usize;
    if board.side_to_move == Color::White {
        propagate_sum(net, acc_white, acc_black, aux_white, aux_black, bucket)
    } else {
        propagate_sum(net, acc_black, acc_white, aux_black, aux_white, bucket)
    }
}

fn finish_eval(
    net: &PlentyChessNetwork,
    board: &Board,
    acc_white: &[i16; L1],
    acc_black: &[i16; L1],
) -> i32 {
    let pieces = board.all_occupancy().count_ones() as i32;
    let divisor = (32 + OUTPUT_BUCKETS as i32 - 1) / OUTPUT_BUCKETS as i32;
    let bucket = ((pieces - 2) / divisor).clamp(0, OUTPUT_BUCKETS as i32 - 1) as usize;
    if board.side_to_move == Color::White {
        propagate(net, acc_white, acc_black, bucket)
    } else {
        propagate(net, acc_black, acc_white, bucket)
    }
}

#[inline(always)]
fn propagate(net: &PlentyChessNetwork, us: &[i16; L1], them: &[i16; L1], bucket: usize) -> i32 {
    let mut ft_out = super::layered_forward::Align64::new([0u8; L1]);
    activate_ft(us, &mut ft_out.0[..L1 / 2]);
    activate_ft(them, &mut ft_out.0[L1 / 2..]);

    finish_from_ft(net, &ft_out.0, bucket)
}

#[inline(always)]
fn propagate_sum(
    net: &PlentyChessNetwork,
    us: &[i16; L1],
    them: &[i16; L1],
    us_aux: &[i16; L1],
    them_aux: &[i16; L1],
    bucket: usize,
) -> i32 {
    let mut ft_out = super::layered_forward::Align64::new([0u8; L1]);
    activate_ft_sum(us, us_aux, &mut ft_out.0[..L1 / 2]);
    activate_ft_sum(them, them_aux, &mut ft_out.0[L1 / 2..]);
    finish_from_ft(net, &ft_out.0, bucket)
}

#[inline(always)]
fn finish_from_ft(net: &PlentyChessNetwork, ft_out: &[u8; L1], bucket: usize) -> i32 {
    let mut l1_sum = [0i32; L2];
    let l1_weight_base = bucket * L2 * L1;
    super::layered_forward::affine_sparse_packed(
        ft_out,
        &net.l1_weights[l1_weight_base..l1_weight_base + L2 * L1],
        &mut l1_sum,
    );

    let mut l1 = [0.0f32; L2 * 2];
    let l1_bias_base = bucket * L2;
    for j in 0..L2 {
        let biased = l1_sum[j] as f32 * L1_NORMALISATION + net.l1_biases[l1_bias_base + j];
        l1[j] = biased.clamp(0.0, 1.0);
        l1[j + L2] = (biased * biased).clamp(0.0, 1.0);
    }

    let mut l2 = [0.0f32; L3];
    let l2_weight_base = bucket * (L2 * 2) * L3;
    let l2_bias_base = bucket * L3;
    super::layered_forward::affine_f32(
        &l1,
        &net.l2_weights[l2_weight_base..l2_weight_base + (L2 * 2) * L3],
        &net.l2_biases[l2_bias_base..l2_bias_base + L3],
        &mut l2,
    );
    super::layered_forward::square_clamp01(&mut l2);

    let l3_base = bucket * (L3 + 2 * L2);
    let l2_dot = super::layered_forward::dot_f32(
        &l2,
        &net.l3_weights[l3_base..l3_base + L3],
        net.l3_biases[bucket],
    );
    let l1_dot = super::layered_forward::dot_f32(
        &l1,
        &net.l3_weights[l3_base + L3..l3_base + L3 + 2 * L2],
        0.0,
    );
    ((l2_dot + l1_dot) * NETWORK_SCALE as f32) as i32
}

#[inline(always)]
fn activate_ft(acc: &[i16], out: &mut [u8]) {
    let half = L1 / 2;
    debug_assert_eq!(out.len(), half);
    super::stockfish_simd::activate_shifted_pair(&acc[..half], &acc[half..], out);
}

#[inline(always)]
fn activate_ft_sum(acc: &[i16], aux: &[i16], out: &mut [u8]) {
    let half = L1 / 2;
    debug_assert_eq!(out.len(), half);
    super::stockfish_simd::activate_shifted_pair_sum(
        &acc[..half],
        &aux[..half],
        &acc[half..],
        &aux[half..],
        out,
    );
}

const MAX_PLY: usize = 256;
const FINNY_ENTRIES: usize = 2 * 2 * KING_BUCKETS;

struct PlentyChessFrame {
    values: [[i16; L1]; 2],
    aux: [[i16; L1]; 2],
    threats: [[u16; MAX_AUX_FEATURES]; 2],
    threat_count: [usize; 2],
    pairs: [[u16; MAX_AUX_FEATURES]; 2],
    pair_count: [usize; 2],
    aux_overflowed: [bool; 2],
    threat_deltas: [ThreatDelta; MAX_DIRTY_THREAT_DELTAS],
    threat_delta_count: usize,
    threat_overflowed: bool,
    pending_threats: Option<ThreatSnapshot>,
    pawns_before: [u64; 2],
    kings: [u8; 2],
    pending_has_move: bool,
    pending_move: Move,
    pending_mover: u8,
    pending_captured: u8,
    hash: u64,
    accurate: bool,
    pending_null: bool,
}

impl Default for PlentyChessFrame {
    fn default() -> Self {
        Self {
            values: [[0; L1]; 2],
            aux: [[0; L1]; 2],
            threats: [[0; MAX_AUX_FEATURES]; 2],
            threat_count: [0; 2],
            pairs: [[0; MAX_AUX_FEATURES]; 2],
            pair_count: [0; 2],
            aux_overflowed: [false; 2],
            threat_deltas: [ThreatDelta::default(); MAX_DIRTY_THREAT_DELTAS],
            threat_delta_count: 0,
            threat_overflowed: false,
            pending_threats: None,
            pawns_before: [0; 2],
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

impl Clone for PlentyChessFrame {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for PlentyChessFrame {}

impl ThreatDeltaSink for PlentyChessFrame {
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

pub(crate) struct PlentyChessAccumulatorState {
    frames: Box<[PlentyChessFrame]>,
    finny: Box<[FinnyEntry]>,
    index: usize,
}

impl PlentyChessAccumulatorState {
    pub(crate) fn new() -> Self {
        Self {
            frames: vec![PlentyChessFrame::default(); MAX_PLY].into_boxed_slice(),
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
            "PlentyChess NNUE stack exhausted"
        );
        self.index += 1;
        let frame = &mut self.frames[self.index];
        frame.accurate = false;
        frame.pending_null = false;
        frame.pending_has_move = true;
        frame.pending_move = mv;
        frame.pending_mover = board.piece_ids()[mv.from.index()];
        frame.pending_captured = board.piece_ids()[mv.to.index()];
        frame.pawns_before = [
            board.pieces[0][Piece::Pawn.index()],
            board.pieces[1][Piece::Pawn.index()],
        ];
        frame.threat_delta_count = 0;
        frame.threat_overflowed = false;
        frame.pending_threats = Some(ThreatSnapshot::from_board(board));
        frame.hash = 0;
    }

    #[inline]
    pub(crate) fn push_null(&mut self) {
        assert!(
            self.index + 1 < self.frames.len(),
            "PlentyChess NNUE stack exhausted"
        );
        let next = self.index + 1;
        self.frames[next] = self.frames[self.index];
        self.frames[next].pending_has_move = false;
        self.frames[next].pending_threats = None;
        self.frames[next].pending_null = true;
        self.index = next;
    }

    #[inline]
    pub(crate) fn pop(&mut self) {
        assert!(
            self.index != 0,
            "cannot pop the root PlentyChess NNUE frame"
        );
        self.index -= 1;
    }

    pub(crate) fn evaluate(&mut self, board: &Board, network: &PlentyChessNetwork) -> i32 {
        self.ensure(board, network, false);
        self.finish(board, network)
    }

    pub(crate) fn evaluate_search(&mut self, board: &Board, network: &PlentyChessNetwork) -> i32 {
        self.ensure(board, network, true);
        self.finish(board, network)
    }

    fn finish(&self, board: &Board, network: &PlentyChessNetwork) -> i32 {
        let frame = &self.frames[self.index];
        if aux_enabled(network) {
            finish_eval_sum(
                network,
                board,
                &frame.values[0],
                &frame.values[1],
                &frame.aux[0],
                &frame.aux[1],
            )
        } else {
            finish_eval(network, board, &frame.values[0], &frame.values[1])
        }
    }

    fn ensure(&mut self, board: &Board, network: &PlentyChessNetwork, trusted: bool) {
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
            self.update_from_parent(board, network);
        } else {
            self.refresh(board, network);
        }
    }

    fn refresh(&mut self, board: &Board, network: &PlentyChessNetwork) {
        let occupancy = snapshot_occupancy(board);
        let kings = [
            board.king_square(Color::White).index(),
            board.king_square(Color::Black).index(),
        ];
        for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
            self.finny_refresh(side, kings[pov], &occupancy, network);
            self.frames[self.index].values[pov] = self.finny[finny_index(side, kings[pov])].values;
        }
        {
            let frame = &mut self.frames[self.index];
            frame.kings = [kings[0] as u8, kings[1] as u8];
            frame.hash = board.hash;
            frame.accurate = true;
            frame.pending_has_move = false;
            frame.pending_threats = None;
            frame.pending_null = false;
        }
        self.refresh_aux(board, network);
    }

    fn update_from_parent(&mut self, board: &Board, network: &PlentyChessNetwork) {
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
        {
            let frame = &mut self.frames[current];
            if let (Some(snapshot), true) = (frame.pending_threats.take(), frame.pending_has_move) {
                collect_snapshot_move_deltas(frame, snapshot, frame.pending_move);
            }
        }
        let pending_has_move = self.frames[current].pending_has_move;
        let pending_move = self.frames[current].pending_move;
        let pending_mover = self.frames[current].pending_mover;
        let pending_captured = self.frames[current].pending_captured;
        let parent_values = self.frames[current - 1].values;
        {
            let frame = &mut self.frames[current];
            for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
                if needs_refresh[pov] {
                    continue;
                }
                if pending_has_move {
                    apply_move_delta_from(
                        &mut frame.values[pov],
                        &parent_values[pov],
                        &network.psq_weights,
                        kings[pov],
                        side,
                        pending_move,
                        pending_mover,
                        pending_captured,
                    );
                } else {
                    frame.values[pov] = parent_values[pov];
                }
            }
            frame.kings = [kings[0] as u8, kings[1] as u8];
            frame.hash = board.hash;
            frame.accurate = true;
            frame.pending_has_move = false;
            frame.pending_null = false;
        }
        self.update_aux_from_parent(board, network, needs_refresh);
    }

    fn store_aux_lists(&mut self, pov: usize, lists: &AuxFeatureLists) {
        let frame = &mut self.frames[self.index];
        frame.threats[pov] = lists.threats;
        frame.threat_count[pov] = lists.threat_count;
        frame.pairs[pov] = lists.pairs;
        frame.pair_count[pov] = lists.pair_count;
        frame.aux_overflowed[pov] = lists.overflowed;
    }

    fn refresh_aux(&mut self, board: &Board, network: &PlentyChessNetwork) {
        if !aux_enabled(network) {
            let frame = &mut self.frames[self.index];
            frame.aux = [[0; L1]; 2];
            frame.threat_count = [0; 2];
            frame.pair_count = [0; 2];
            frame.aux_overflowed = [false; 2];
            return;
        }
        for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
            self.refresh_aux_pov(board, network, pov, side);
        }
    }

    fn refresh_aux_pov(
        &mut self,
        board: &Board,
        network: &PlentyChessNetwork,
        pov: usize,
        side: Color,
    ) {
        let lists = collect_aux_lists(board, side);
        let mut aux = [0i16; L1];
        if lists.overflowed {
            add_aux(network, board, side, &mut aux);
        } else {
            apply_aux_lists(&mut aux, network, &lists);
        }
        self.frames[self.index].aux[pov] = aux;
        self.store_aux_lists(pov, &lists);
    }

    fn update_aux_from_parent(
        &mut self,
        board: &Board,
        network: &PlentyChessNetwork,
        needs_refresh: [bool; 2],
    ) {
        if !aux_enabled(network) {
            let frame = &mut self.frames[self.index];
            frame.aux = [[0; L1]; 2];
            frame.threat_count = [0; 2];
            frame.pair_count = [0; 2];
            frame.aux_overflowed = [false; 2];
            return;
        }
        let current = self.index;
        let parent_aux = self.frames[current - 1].aux;
        let parent_overflowed = self.frames[current - 1].aux_overflowed;
        let threat_overflowed = self.frames[current].threat_overflowed;
        let pawns_before = self.frames[current].pawns_before;
        let pawns_after = [
            board.pieces[0][Piece::Pawn.index()],
            board.pieces[1][Piece::Pawn.index()],
        ];
        let kings = [
            board.king_square(Color::White).index(),
            board.king_square(Color::Black).index(),
        ];
        let deltas = self.frames[current].threat_deltas;
        let delta_count = self.frames[current].threat_delta_count;
        for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
            if needs_refresh[pov] || parent_overflowed[pov] || threat_overflowed {
                self.refresh_aux_pov(board, network, pov, side);
                continue;
            }
            let mut aux = parent_aux[pov];
            for &delta in &deltas[..delta_count] {
                visit_threat_delta(delta, kings[pov], pov, |feature, sign| {
                    apply_aux_threat(&mut aux, network, feature, sign);
                });
            }
            if pawns_before != pawns_after {
                let old = collect_pawn_pair_aux(pawns_before, kings[pov], pov);
                let new = collect_pawn_pair_aux(pawns_after, kings[pov], pov);
                if old.overflowed || new.overflowed {
                    self.refresh_aux_pov(board, network, pov, side);
                    continue;
                }
                apply_diff(
                    &old.pairs[..old.pair_count],
                    &new.pairs[..new.pair_count],
                    |feature, sign| apply_aux_pair(&mut aux, network, feature, sign),
                );
            }
            self.frames[current].aux[pov] = aux;
            self.frames[current].aux_overflowed[pov] = false;
        }
    }

    fn finny_refresh(
        &mut self,
        side: Color,
        king: usize,
        occupancy: &[u64; 12],
        network: &PlentyChessNetwork,
    ) {
        let entry = &mut self.finny[finny_index(side, king)];
        if !entry.initialized {
            entry.values.copy_from_slice(&network.feature_biases);
            add_all_pieces(
                &mut entry.values,
                &network.psq_weights,
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
                    apply_psq(
                        &mut entry.values,
                        &network.psq_weights,
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
                    apply_psq(
                        &mut entry.values,
                        &network.psq_weights,
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

#[allow(clippy::too_many_arguments)]
fn apply_move_delta_from(
    dst: &mut [i16; L1],
    src: &[i16; L1],
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
    subs[sub_len] = psq_feature(king, side, mover_piece, mover_color, mv.from.index());
    sub_len += 1;
    adds[add_len] = psq_feature(king, side, resulting, mover_color, mv.to.index());
    add_len += 1;
    if mv.is_capture() && mv.flag != MoveFlag::EnPassant {
        debug_assert_ne!(captured, u8::MAX);
        subs[sub_len] = psq_feature(
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
        subs[sub_len] = psq_feature(
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
        subs[sub_len] = psq_feature(king, side, Piece::Rook, mover_color, rook_from);
        sub_len += 1;
        adds[add_len] = psq_feature(king, side, Piece::Rook, mover_color, rook_to);
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

pub fn is_plentychess_path(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.contains("plenty") || name.contains("0179") || name.contains("plenty_default")
}

fn read_sleb128(bytes: &[u8], pos: &mut usize) -> Result<i64, String> {
    let mut result = 0i64;
    let mut shift = 0;
    loop {
        let byte = *bytes
            .get(*pos)
            .ok_or_else(|| "truncated PlentyChess SLEB128".to_string())?;
        *pos += 1;
        result |= i64::from(byte & 0x7F) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            if shift < 64 && byte & 0x40 != 0 {
                result |= -1i64 << shift;
            }
            return Ok(result);
        }
        if shift >= 64 {
            return Err("PlentyChess SLEB128 overflow".to_string());
        }
    }
}

fn read_uleb128(bytes: &[u8], pos: &mut usize) -> Result<u64, String> {
    let mut result = 0u64;
    let mut shift = 0;
    loop {
        let byte = *bytes
            .get(*pos)
            .ok_or_else(|| "truncated PlentyChess ULEB128".to_string())?;
        *pos += 1;
        result |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift >= 64 {
            return Err("PlentyChess ULEB128 overflow".to_string());
        }
    }
}

fn read_sleb_i16s(bytes: &[u8], pos: &mut usize, count: usize) -> Result<Box<[i16]>, String> {
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(
            i16::try_from(read_sleb128(bytes, pos)?)
                .map_err(|_| "PlentyChess i16 weight out of range".to_string())?,
        );
    }
    Ok(values.into_boxed_slice())
}

fn read_sleb_i8s(bytes: &[u8], pos: &mut usize, count: usize) -> Result<Box<[i8]>, String> {
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(
            i8::try_from(read_sleb128(bytes, pos)?)
                .map_err(|_| "PlentyChess i8 weight out of range".to_string())?,
        );
    }
    Ok(values.into_boxed_slice())
}

fn read_uleb_f32s(bytes: &[u8], pos: &mut usize, count: usize) -> Result<Box<[f32]>, String> {
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let bits = u32::try_from(read_uleb128(bytes, pos)?)
            .map_err(|_| "PlentyChess f32 payload out of range".to_string())?;
        values.push(f32::from_bits(bits));
    }
    Ok(values.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_plentychess_filenames() {
        assert!(is_plentychess_path(Path::new("plenty_default.bin")));
        assert!(is_plentychess_path(Path::new("0179r.bin")));
        assert!(is_plentychess_path(Path::new("PlentyChess-0179.bin")));
        assert!(!is_plentychess_path(Path::new("obs_default.bin")));
        assert!(!is_plentychess_path(Path::new("ak_default.bin")));
    }

    #[test]
    fn sleb128_roundtrip_values() {
        // 0, -1, 127, -128 encoded the same way process_net writes them.
        let encoded = [0x00, 0x7F, 0xFF, 0x00, 0x80, 0x7F];
        let mut pos = 0;
        assert_eq!(read_sleb128(&encoded, &mut pos).unwrap(), 0);
        assert_eq!(read_sleb128(&encoded, &mut pos).unwrap(), -1);
        assert_eq!(read_sleb128(&encoded, &mut pos).unwrap(), 127);
        assert_eq!(read_sleb128(&encoded, &mut pos).unwrap(), -128);
        assert_eq!(pos, encoded.len());
    }

    #[test]
    fn rejects_truncated_payload() {
        assert!(PlentyChessNetwork::from_compressed_bytes(&[0u8; 16]).is_err());
    }

    #[test]
    fn king_bucket_layout_has_twelve_ids() {
        let mut seen = [false; KING_BUCKETS];
        for bucket in KING_BUCKET_LAYOUT {
            seen[bucket] = true;
        }
        assert!(seen.iter().all(|used| *used));
        assert!(KING_BUCKET_LAYOUT[32..].iter().all(|bucket| *bucket == 11));
    }

    fn patterned_net() -> PlentyChessNetwork {
        let mut psq = vec![0i16; FEATURES * KING_BUCKETS * L1];
        for (index, weight) in psq.iter_mut().enumerate() {
            *weight = ((index % 251) as i16).wrapping_sub(125);
        }
        PlentyChessNetwork {
            psq_weights: psq.into_boxed_slice(),
            pawn_pair_weights: Box::new([]),
            threat_weights: Box::new([]),
            feature_biases: vec![0i16; L1].into_boxed_slice(),
            l1_weights: vec![0i8; OUTPUT_BUCKETS * L2 * L1].into_boxed_slice(),
            l1_biases: vec![0.0; OUTPUT_BUCKETS * L2].into_boxed_slice(),
            l2_weights: vec![0.0; OUTPUT_BUCKETS * (L2 * 2) * L3].into_boxed_slice(),
            l2_biases: vec![0.0; OUTPUT_BUCKETS * L3].into_boxed_slice(),
            l3_weights: vec![0.0; OUTPUT_BUCKETS * (L3 + 2 * L2)].into_boxed_slice(),
            l3_biases: vec![0.0; OUTPUT_BUCKETS].into_boxed_slice(),
        }
    }

    fn assert_incremental_matches(
        state: &mut PlentyChessAccumulatorState,
        net: &PlentyChessNetwork,
        board: &Board,
    ) {
        let expected = scratch_piece_accumulators(net, board);
        let incremental = state.evaluate(board, net);
        assert_eq!(incremental, net.evaluate(board));
        assert_eq!(state.evaluate_search(board, net), incremental);
        assert_eq!(state.frames[state.index].values, expected);
    }

    #[test]
    fn incremental_state_matches_scratch_after_moves_and_pop() {
        types::init();
        let net = patterned_net();
        let mut state = PlentyChessAccumulatorState::new();
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
    fn activate_ft_matches_shifted_pair_formula() {
        let mut acc = [0i16; L1];
        for (index, value) in acc.iter_mut().enumerate() {
            *value = (index as i16).wrapping_mul(7).wrapping_sub(300);
        }
        let mut out = [0u8; L1 / 2];
        activate_ft(&acc, &mut out);
        for (index, &actual) in out.iter().enumerate() {
            let c0 = i32::from(acc[index]).clamp(0, NETWORK_QA);
            let c1 = i32::from(acc[index + L1 / 2]).min(NETWORK_QA);
            let expected = (((c0 << (16 - FT_SHIFT)) * c1) >> 16).clamp(0, 255) as u8;
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn aux_feature_diff_reconstructs_child_lists() {
        types::init();
        let mut board = Board::new();
        let parent = collect_aux_lists(&board, Color::White);
        assert!(!parent.overflowed);
        assert!(parent.threat_count > 0);
        assert!(
            parent.threats[..parent.threat_count]
                .windows(2)
                .all(|window| window[0] <= window[1])
        );
        assert!(
            parent.pairs[..parent.pair_count]
                .windows(2)
                .all(|window| window[0] <= window[1])
        );

        let mv = board
            .generate_legal_moves()
            .iter()
            .find(|mv| mv.to_uci() == "e2e4")
            .copied()
            .expect("e2e4 is legal");
        board.make_move(mv);
        let child = collect_aux_lists(&board, Color::White);
        assert!(!child.overflowed);

        let mut reconstructed = parent.threats[..parent.threat_count].to_vec();
        apply_diff(
            &parent.threats[..parent.threat_count],
            &child.threats[..child.threat_count],
            |feature, sign| {
                if sign > 0 {
                    reconstructed.push(feature as u16);
                } else {
                    reconstructed.retain(|&existing| existing != feature as u16);
                }
            },
        );
        reconstructed.sort_unstable();
        assert_eq!(reconstructed, child.threats[..child.threat_count].to_vec());

        let mut pair_reconstructed = parent.pairs[..parent.pair_count].to_vec();
        apply_diff(
            &parent.pairs[..parent.pair_count],
            &child.pairs[..child.pair_count],
            |feature, sign| {
                if sign > 0 {
                    pair_reconstructed.push(feature as u16);
                } else {
                    pair_reconstructed.retain(|&existing| existing != feature as u16);
                }
            },
        );
        pair_reconstructed.sort_unstable();
        assert_eq!(pair_reconstructed, child.pairs[..child.pair_count].to_vec());
    }

    #[test]
    fn incremental_aux_matches_scratch_on_published_net() {
        let Some(path) = crate::nnue::adapter::discover_named_network("plenty_default.bin")
            .or_else(|| crate::nnue::adapter::discover_named_network("0179r.bin"))
        else {
            return;
        };
        types::init();
        let bytes = std::fs::read(&path).expect("published PlentyChess net is readable");
        let net = PlentyChessNetwork::from_compressed_bytes(&bytes)
            .expect("published PlentyChess net decodes");
        assert!(aux_enabled(&net));
        let mut state = PlentyChessAccumulatorState::new();
        let mut board = Board::new();
        assert_incremental_matches(&mut state, &net, &board);
        for uci in ["e2e4", "e7e5", "g1f3", "b8c6"] {
            let mv = board
                .generate_legal_moves()
                .iter()
                .find(|candidate| candidate.to_uci() == uci)
                .copied()
                .expect("test move is legal");
            state.push_move(&board, mv);
            board.make_move(mv);
            assert_incremental_matches(&mut state, &net, &board);
            let expected_white = {
                let mut acc = [0i16; L1];
                add_aux(&net, &board, Color::White, &mut acc);
                acc
            };
            assert_eq!(state.frames[state.index].aux[0], expected_white);
        }
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
            let mut state = PlentyChessAccumulatorState::new();
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
    fn dirty_threat_push_records_snapshot_and_search_eval_skips_hash() {
        types::init();
        let net = patterned_net();
        let mut state = PlentyChessAccumulatorState::new();
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
        assert!(state.frames[state.index].pending_threats.is_some());
        assert_eq!(
            state.frames[state.index].pawns_before[0] & (1u64 << 12),
            1u64 << 12
        );
        board.make_move(mv);
        let incremental = state.evaluate(&board, &net);
        assert!(state.frames[state.index].pending_threats.is_none());
        assert_eq!(state.evaluate_search(&board, &net), incremental);
        let mut scratch = PlentyChessAccumulatorState::new();
        assert_eq!(scratch.evaluate(&board, &net), incremental);
    }
}
