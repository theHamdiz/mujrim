//! Obsidian layered NNUE (768→1536→16→32→1, 13 king buckets, 8 output buckets).
//!
//! Layout matches the published `Net` in gab8192/Obsidian `src/nnue.h`.
//! Incremental eval uses a per-ply stack and per-perspective Finny cache.

use std::path::Path;

use types::chess_move::MoveFlag;
use types::{Board, Color, Move, Piece, Square};

pub const L1: usize = 1536;
pub const L2: usize = 16;
pub const L3: usize = 32;
pub const KING_BUCKETS: usize = 13;
pub const OUTPUT_BUCKETS: usize = 8;
pub const FEATURES: usize = 768;
pub const NETWORK_SCALE: i32 = 400;
pub const NETWORK_QA: i32 = 255;
pub const NETWORK_QB: i32 = 128;
const FT_SHIFT: i32 = 9;

/// Packed on-disk size of the published Obsidian `Net` (no trailing pad).
pub const FILE_SIZE: u64 = (KING_BUCKETS * 2 * 6 * 64 * L1 * 2
    + L1 * 2
    + OUTPUT_BUCKETS * L1 * L2
    + OUTPUT_BUCKETS * L2 * 4
    + OUTPUT_BUCKETS * (L2 * 2) * L3 * 4
    + OUTPUT_BUCKETS * L3 * 4
    + OUTPUT_BUCKETS * L3 * 4
    + OUTPUT_BUCKETS * 4) as u64;

#[rustfmt::skip]
const KING_BUCKETS_SCHEME: [usize; 64] = [
    0, 1, 2, 3, 3, 2, 1, 0,
    4, 5, 6, 7, 7, 6, 5, 4,
    8, 8, 9, 9, 9, 9, 8, 8,
    10, 10, 10, 10, 10, 10, 10, 10,
    11, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 11, 11,
    12, 12, 12, 12, 12, 12, 12, 12,
    12, 12, 12, 12, 12, 12, 12, 12,
];

pub struct ObsidianNetwork {
    feature_weights: Box<[i16]>,
    feature_biases: Box<[i16]>,
    l1_weights: Box<[i8]>,
    l1_biases: Box<[f32]>,
    l2_weights: Box<[f32]>,
    l2_biases: Box<[f32]>,
    l3_weights: Box<[f32]>,
    l3_biases: Box<[f32]>,
}

impl ObsidianNetwork {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < FILE_SIZE as usize {
            return Err(format!(
                "Obsidian NNUE too small: expected at least {FILE_SIZE} bytes, found {}",
                bytes.len()
            ));
        }
        let mut offset = 0;
        let feature_weights = read_i16s(bytes, &mut offset, KING_BUCKETS * 2 * 6 * 64 * L1)?;
        let feature_biases = read_i16s(bytes, &mut offset, L1)?;
        let l1_weights = read_i8s(bytes, &mut offset, OUTPUT_BUCKETS * L1 * L2)?;
        let l1_biases = read_f32s(bytes, &mut offset, OUTPUT_BUCKETS * L2)?;
        let l2_weights = read_f32s(bytes, &mut offset, OUTPUT_BUCKETS * (L2 * 2) * L3)?;
        let l2_biases = read_f32s(bytes, &mut offset, OUTPUT_BUCKETS * L3)?;
        let l3_weights = read_f32s(bytes, &mut offset, OUTPUT_BUCKETS * L3)?;
        let l3_biases = read_f32s(bytes, &mut offset, OUTPUT_BUCKETS)?;
        Ok(Self {
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

    #[inline(always)]
    pub fn evaluate(&self, board: &Board) -> i32 {
        let [acc_white, acc_black] = scratch_accumulators(self, board);
        finish_eval(self, board, &acc_white, &acc_black)
    }
}

const MAX_PLY: usize = 256;
const FINNY_ENTRIES: usize = 2 * 2 * KING_BUCKETS;

#[inline(always)]
fn feature_index(
    king_sq: usize,
    side: Color,
    piece: Piece,
    piece_color: Color,
    mut sq: usize,
) -> usize {
    if king_sq & 0b100 != 0 {
        sq ^= 7;
    }
    let rel_king = relative_square(side, king_sq);
    let rel_sq = relative_square(side, sq);
    let bucket = KING_BUCKETS_SCHEME[rel_king];
    let them = usize::from(side != piece_color);
    (((bucket * 2 + them) * 6 + piece.index()) * 64) + rel_sq
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn apply_feature(
    acc: &mut [i16; L1],
    weights: &[i16],
    king_sq: usize,
    side: Color,
    piece: Piece,
    piece_color: Color,
    sq: usize,
    sign: i16,
) {
    let feature = feature_index(king_sq, side, piece, piece_color, sq);
    super::stockfish_simd::apply_i16_feature_width(acc, weights, feature, sign);
}

#[inline(always)]
fn king_bucket(king_sq: usize, side: Color) -> usize {
    KING_BUCKETS_SCHEME[relative_square(side, king_sq)]
}

#[inline(always)]
fn king_mirrored(king_sq: usize) -> bool {
    king_sq & 0b100 != 0
}

#[inline(always)]
fn king_needs_refresh(old: usize, new: usize, side: Color) -> bool {
    king_bucket(old, side) != king_bucket(new, side) || king_mirrored(old) != king_mirrored(new)
}

#[inline(always)]
fn finny_index(side: Color, king_sq: usize) -> usize {
    side.index() * 2 * KING_BUCKETS
        + usize::from(king_mirrored(king_sq)) * KING_BUCKETS
        + king_bucket(king_sq, side)
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

fn scratch_accumulators(net: &ObsidianNetwork, board: &Board) -> [[i16; L1]; 2] {
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
    side: Color,
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
                    side,
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

fn finish_eval(
    net: &ObsidianNetwork,
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

struct ObsidianFrame {
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

impl Default for ObsidianFrame {
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

pub(crate) struct ObsidianAccumulatorState {
    frames: Box<[ObsidianFrame]>,
    finny: Box<[FinnyEntry]>,
    index: usize,
}

impl ObsidianAccumulatorState {
    pub(crate) fn new() -> Self {
        Self {
            frames: vec![ObsidianFrame::default(); MAX_PLY].into_boxed_slice(),
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
            "Obsidian NNUE stack exhausted"
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
            "Obsidian NNUE stack exhausted"
        );
        let next = self.index + 1;
        let (before, after) = self.frames.split_at_mut(next);
        after[0].clone_from(&before[self.index]);
        after[0].pending_has_move = false;
        after[0].pending_null = true;
        self.index = next;
    }

    #[inline]
    pub(crate) fn pop(&mut self) {
        assert!(self.index != 0, "cannot pop the root Obsidian NNUE frame");
        self.index -= 1;
    }

    pub(crate) fn evaluate(&mut self, board: &Board, network: &ObsidianNetwork) -> i32 {
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
        finish_eval(network, board, &frame.values[0], &frame.values[1])
    }

    fn refresh(&mut self, board: &Board, network: &ObsidianNetwork) {
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

    fn update_from_parent(&mut self, board: &Board, network: &ObsidianNetwork) {
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
        let (before, after) = self.frames.split_at_mut(current);
        let parent = &before[current - 1];
        let frame = &mut after[0];
        for (pov, side) in [Color::White, Color::Black].into_iter().enumerate() {
            if needs_refresh[pov] {
                continue;
            }
            frame.values[pov] = parent.values[pov];
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
        network: &ObsidianNetwork,
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

impl Clone for ObsidianFrame {
    fn clone(&self) -> Self {
        Self {
            values: self.values,
            kings: self.kings,
            pending_has_move: self.pending_has_move,
            pending_move: self.pending_move,
            pending_mover: self.pending_mover,
            pending_captured: self.pending_captured,
            hash: self.hash,
            accurate: self.accurate,
            pending_null: self.pending_null,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.values = source.values;
        self.kings = source.kings;
        self.pending_has_move = source.pending_has_move;
        self.pending_move = source.pending_move;
        self.pending_mover = source.pending_mover;
        self.pending_captured = source.pending_captured;
        self.hash = source.hash;
        self.accurate = source.accurate;
        self.pending_null = source.pending_null;
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
        debug_assert_ne!(captured, u8::MAX);
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
        let captured_square = Square::from_file_rank(mv.to.file(), mv.from.rank()).index();
        apply_feature(
            acc,
            weights,
            king,
            side,
            Piece::Pawn,
            mover_color.opponent(),
            captured_square,
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

#[inline(always)]
fn relative_square(side: Color, sq: usize) -> usize {
    if side == Color::Black { sq ^ 56 } else { sq }
}

#[inline(always)]
fn propagate(net: &ObsidianNetwork, us: &[i16; L1], them: &[i16; L1], bucket: usize) -> i32 {
    let mut ft_out = [0u8; L1];
    activate_ft(us, &mut ft_out[..L1 / 2]);
    activate_ft(them, &mut ft_out[L1 / 2..]);

    let scale = 1.0 / ((NETWORK_QA * NETWORK_QA * NETWORK_QB) >> FT_SHIFT) as f32;
    let mut l1 = [0.0f32; L2 * 2];
    let (l1_linear, l1_sqr) = l1.split_at_mut(L2);
    let l1_weight_base = bucket * L1 * L2;
    let l1_bias_base = bucket * L2;
    for (j, (linear, squared)) in l1_linear.iter_mut().zip(l1_sqr.iter_mut()).enumerate() {
        let sum = ft_out.iter().enumerate().fold(0i32, |acc, (i, feature)| {
            acc + i32::from(*feature) * i32::from(net.l1_weights[l1_weight_base + i * L2 + j])
        });
        let biased = sum as f32 * scale + net.l1_biases[l1_bias_base + j];
        *linear = biased.clamp(0.0, 1.0);
        *squared = (biased * biased).clamp(0.0, 1.0);
    }

    let mut l2 = [0.0f32; L3];
    let l2_weight_base = bucket * (L2 * 2) * L3;
    let l2_bias_base = bucket * L3;
    for (j, value) in l2.iter_mut().enumerate() {
        let sum = l1
            .iter()
            .enumerate()
            .fold(net.l2_biases[l2_bias_base + j], |acc, (i, feature)| {
                acc + net.l2_weights[l2_weight_base + i * L3 + j] * *feature
            });
        *value = sum.clamp(0.0, 1.0);
    }

    let l3 = l2
        .iter()
        .enumerate()
        .fold(net.l3_biases[bucket], |acc, (j, feature)| {
            acc + net.l3_weights[bucket * L3 + j] * *feature
        });
    (l3 * NETWORK_SCALE as f32) as i32
}

#[inline(always)]
fn activate_ft(acc: &[i16], out: &mut [u8]) {
    let half = L1 / 2;
    debug_assert_eq!(out.len(), half);
    for i in 0..half {
        let c0 = i32::from(acc[i]).clamp(0, NETWORK_QA);
        let c1 = i32::from(acc[i + half]).clamp(i32::MIN, NETWORK_QA);
        let shifted = c0 << (16 - FT_SHIFT);
        let prod = ((shifted * c1) >> 16).clamp(0, 255);
        out[i] = prod as u8;
    }
}

pub fn load(path: &Path) -> Result<Box<ObsidianNetwork>, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read Obsidian NNUE '{}': {error}", path.display()))?;
    ObsidianNetwork::from_bytes(&bytes).map(Box::new)
}

pub fn is_obsidian_path(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.contains("obsidian")
        || name.contains("net89")
        || name.contains("obs_default")
        || name.ends_with("perm.bin")
}

fn read_i16s(bytes: &[u8], offset: &mut usize, count: usize) -> Result<Box<[i16]>, String> {
    let need = count * 2;
    let slice = bytes
        .get(*offset..*offset + need)
        .ok_or_else(|| "truncated Obsidian i16 weights".to_string())?;
    *offset += need;
    Ok(slice
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect())
}

fn read_i8s(bytes: &[u8], offset: &mut usize, count: usize) -> Result<Box<[i8]>, String> {
    let slice = bytes
        .get(*offset..*offset + count)
        .ok_or_else(|| "truncated Obsidian i8 weights".to_string())?;
    *offset += count;
    Ok(slice.iter().map(|byte| *byte as i8).collect())
}

fn read_f32s(bytes: &[u8], offset: &mut usize, count: usize) -> Result<Box<[f32]>, String> {
    let need = count * 4;
    let slice = bytes
        .get(*offset..*offset + need)
        .ok_or_else(|| "truncated Obsidian f32 weights".to_string())?;
    *offset += need;
    Ok(slice
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero_net() -> ObsidianNetwork {
        let zeros = vec![0u8; FILE_SIZE as usize];
        ObsidianNetwork::from_bytes(&zeros).unwrap()
    }

    #[test]
    fn packed_file_size_is_stable() {
        assert_eq!(FILE_SIZE, 30_905_888);
    }

    #[test]
    fn zero_network_evaluates_startpos_to_zero() {
        types::init();
        let net = zero_net();
        let board = Board::new();
        assert_eq!(net.evaluate(&board), 0);
    }

    #[test]
    fn rejects_truncated_payload() {
        assert!(ObsidianNetwork::from_bytes(&[0u8; 16]).is_err());
    }

    #[test]
    fn detects_obsidian_filenames() {
        assert!(is_obsidian_path(Path::new("net89perm.bin")));
        assert!(is_obsidian_path(Path::new("obs_default.bin")));
        assert!(is_obsidian_path(Path::new("Obsidian-16.bin")));
        assert!(!is_obsidian_path(Path::new("ak_default.bin")));
    }

    fn patterned_net() -> ObsidianNetwork {
        let mut net = zero_net();
        for (index, weight) in net.feature_weights.iter_mut().enumerate() {
            *weight = ((index % 251) as i16).wrapping_sub(125);
        }
        net
    }

    fn assert_incremental_matches(
        state: &mut ObsidianAccumulatorState,
        net: &ObsidianNetwork,
        board: &Board,
    ) {
        let expected = scratch_accumulators(net, board);
        assert_eq!(state.evaluate(board, net), net.evaluate(board));
        assert_eq!(state.frames[state.index].values, expected);
    }

    #[test]
    fn incremental_state_matches_scratch_after_moves_and_pop() {
        types::init();
        let net = patterned_net();
        let mut state = ObsidianAccumulatorState::new();
        let mut board = Board::new();
        assert_incremental_matches(&mut state, &net, &board);

        let mut last_move = None;
        for uci in ["e2e4", "e7e5", "g1f3", "b8c6", "f1b5", "a7a6"] {
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
    fn incremental_state_matches_scratch_for_special_moves() {
        types::init();
        let net = patterned_net();
        let cases = [
            ("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1", "e1g1"),
            ("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1", "e5d6"),
            ("4k3/P7/8/8/8/8/8/4K3 w - - 0 1", "a7a8q"),
        ];
        for (fen, uci) in cases {
            let mut state = ObsidianAccumulatorState::new();
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
    fn king_refresh_uses_finny_and_matches_scratch() {
        types::init();
        let net = patterned_net();
        let mut state = ObsidianAccumulatorState::new();
        let mut board =
            Board::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").expect("test FEN is valid");
        assert_incremental_matches(&mut state, &net, &board);
        for uci in ["e1e2", "e8e7", "e2d3"] {
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
        assert!(state.finny[finny_index(Color::White, 4)].initialized);
        assert!(state.finny[finny_index(Color::White, 12)].initialized);
    }

    #[test]
    fn null_move_reuses_the_obsidian_accumulator() {
        types::init();
        let net = patterned_net();
        let mut state = ObsidianAccumulatorState::new();
        let mut board =
            Board::from_fen("r1bq1rk1/ppp2ppp/2n2n2/2bp4/4P3/2P2N2/PP1N1PPP/R1BQ1RK1 w - - 2 9")
                .expect("test FEN is valid");
        assert_incremental_matches(&mut state, &net, &board);
        state.push_null();
        board.make_null_move();
        assert_incremental_matches(&mut state, &net, &board);
        state.pop();
        board.unmake_null_move();
        assert_incremental_matches(&mut state, &net, &board);
    }
}
