//! Stockfish NNUE v1 format support for the pinned `nn-ab28990d4ea3` network.
//!
//! This module owns format validation separately from evaluation. Stockfish networks must never
//! be interpreted through Mujrim's Akimbo or Reckless layouts: their feature transformers and
//! layer stacks are structurally different.

use std::path::Path;
use std::sync::OnceLock;
use std::{fs::File, io::Read};

use types::board::attack_tables::{
    bishop_attacks, king_attacks, knight_attacks, pawn_attacks, queen_attacks, rook_attacks,
};
use types::chess_move::MoveFlag;
use types::{Board, Color, Move, Piece, Square};

use super::dirty_threats::{
    ThreatDelta, ThreatDeltaSink, ThreatSnapshot, collect_snapshot_move_deltas,
};

pub const NETWORK_FILENAME: &str = "nn-ab28990d4ea3.nnue";
pub const NETWORK_SHA256: &str = "ab28990d4ea3d5c97f7d3918bc5dd5061609330369fe00c2d93a34d4777b5552";
pub const FILE_SIZE: usize = 95_144_073;
pub const FORMAT_VERSION: u32 = 0x6A44_8AFA;
pub const NETWORK_HASH: u32 = 0xA85B_2205;
pub const FEATURE_TRANSFORMER_HASH: u32 = 0xCB68_5313;
pub const INPUT_DIMENSIONS: usize = 86_896;
pub const L1: usize = 1_024;
pub const L2: usize = 32;
pub const L3: usize = 32;
pub const PSQT_BUCKETS: usize = 8;
pub const LAYER_STACKS: usize = 8;
pub const PSQ_FEATURES: usize = 22_528;
pub const THREAT_FEATURES: usize = 59_808;
pub const PAIR_FEATURES: usize = 4_560;

const ARCHITECTURE_HASH: u32 = 0x6333_7116;
const LEB128_MAGIC: &[u8; 17] = b"COMPRESSED_LEB128";

const HEADER_FIXED_BYTES: usize = 12;
#[cfg(feature = "embedded-networks")]
const EMBEDDED_BYTES: &[u8; FILE_SIZE] = include_bytes!("../../resources/nn-ab28990d4ea3.nnue");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header<'a> {
    pub version: u32,
    pub network_hash: u32,
    pub description: &'a str,
    pub feature_transformer_hash: u32,
    pub parameters_offset: usize,
}

pub struct StockfishNetwork {
    feature_biases: Box<[i16]>,
    threat_weights: I8Weights,
    pair_weights: I8Weights,
    threat_and_pair_psqt: Box<[i32]>,
    piece_weights: Box<[i16]>,
    piece_psqt: Box<[i32]>,
    layers: Box<[LayerStack; LAYER_STACKS]>,
}

enum I8Weights {
    Borrowed(&'static [i8]),
    Owned(Box<[i8]>),
}

impl I8Weights {
    #[inline]
    fn as_slice(&self) -> &[i8] {
        match self {
            Self::Borrowed(weights) => weights,
            Self::Owned(weights) => weights,
        }
    }

    #[inline]
    fn len(&self) -> usize {
        self.as_slice().len()
    }
}

struct LayerStack {
    fc0_biases: [i32; L2],
    fc0_weights: Box<[i8]>,
    fc1_biases: [i32; L3],
    fc1_weights: Box<[i8]>,
    fc2_bias: i32,
    fc2_weights: [i8; 128],
}

#[inline]
pub fn embedded_bytes() -> &'static [u8] {
    #[cfg(feature = "embedded-networks")]
    {
        EMBEDDED_BYTES
    }
    #[cfg(not(feature = "embedded-networks"))]
    {
        static BYTES: OnceLock<Box<[u8]>> = OnceLock::new();
        BYTES.get_or_init(|| {
            let path = super::adapter::discover_network_file(FILE_SIZE as u64, NETWORK_SHA256)
                .unwrap_or_else(|error| panic!("Mujrim Stockfish NNUE discovery failed: {error}"));
            std::fs::read(&path)
                .unwrap_or_else(|error| {
                    panic!(
                        "failed to load Stockfish NNUE '{}': {error}",
                        path.display()
                    )
                })
                .into_boxed_slice()
        })
    }
}

pub fn parse_header(bytes: &[u8]) -> Result<Header<'_>, String> {
    if bytes.len() < HEADER_FIXED_BYTES {
        return Err(format!(
            "Stockfish NNUE header is truncated: expected at least {HEADER_FIXED_BYTES} bytes, found {}",
            bytes.len()
        ));
    }

    let version = read_u32(bytes, 0)?;
    if version != FORMAT_VERSION {
        return Err(format!(
            "unsupported Stockfish NNUE version 0x{version:08x}; expected 0x{FORMAT_VERSION:08x}"
        ));
    }

    let network_hash = read_u32(bytes, 4)?;
    if network_hash != NETWORK_HASH {
        return Err(format!(
            "incompatible Stockfish architecture hash 0x{network_hash:08x}; expected 0x{NETWORK_HASH:08x} for {NETWORK_FILENAME}"
        ));
    }

    let description_len = read_u32(bytes, 8)? as usize;
    let description_end = HEADER_FIXED_BYTES
        .checked_add(description_len)
        .ok_or_else(|| "Stockfish NNUE description length overflow".to_string())?;
    let transformer_hash_end = description_end
        .checked_add(4)
        .ok_or_else(|| "Stockfish NNUE header length overflow".to_string())?;
    if bytes.len() < transformer_hash_end {
        return Err(format!(
            "Stockfish NNUE description is truncated: header requires {transformer_hash_end} bytes, found {}",
            bytes.len()
        ));
    }

    let description = std::str::from_utf8(&bytes[HEADER_FIXED_BYTES..description_end])
        .map_err(|error| format!("Stockfish NNUE description is not UTF-8: {error}"))?;
    let feature_transformer_hash = read_u32(bytes, description_end)?;
    if feature_transformer_hash != FEATURE_TRANSFORMER_HASH {
        return Err(format!(
            "incompatible Stockfish feature-transformer hash 0x{feature_transformer_hash:08x}; expected 0x{FEATURE_TRANSFORMER_HASH:08x}"
        ));
    }

    Ok(Header {
        version,
        network_hash,
        description,
        feature_transformer_hash,
        parameters_offset: transformer_hash_end,
    })
}

pub fn validate_embedded() -> Result<Header<'static>, String> {
    parse_header(embedded_bytes())
}

pub fn load_embedded() -> Result<Box<StockfishNetwork>, String> {
    StockfishNetwork::from_static_bytes(embedded_bytes())
}

/// Returns the lazily decoded embedded current Stockfish network.
pub fn embedded() -> &'static StockfishNetwork {
    static NETWORK: OnceLock<Box<StockfishNetwork>> = OnceLock::new();
    NETWORK
        .get_or_init(|| load_embedded().expect("embedded Stockfish network must decode"))
        .as_ref()
}

pub fn load(path: &Path) -> Result<Box<StockfishNetwork>, String> {
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "failed to read Stockfish NNUE '{}': {error}",
            path.display()
        )
    })?;
    StockfishNetwork::from_bytes(&bytes)
}

impl StockfishNetwork {
    pub fn from_bytes(bytes: &[u8]) -> Result<Box<Self>, String> {
        Self::from_bytes_inner(bytes, None)
    }

    fn from_static_bytes(bytes: &'static [u8]) -> Result<Box<Self>, String> {
        Self::from_bytes_inner(bytes, Some(bytes))
    }

    fn from_bytes_inner(
        bytes: &[u8],
        static_bytes: Option<&'static [u8]>,
    ) -> Result<Box<Self>, String> {
        let header = parse_header(bytes)?;
        let mut reader = ParameterReader::new(bytes, header.parameters_offset);

        let feature_biases = reader.read_sleb_i16(L1, "feature biases")?;

        let threat_weights =
            reader.read_i8_weights(THREAT_FEATURES * L1, "threat weights", static_bytes)?;

        let mut threat_and_pair_psqt =
            Vec::with_capacity((THREAT_FEATURES + PAIR_FEATURES) * PSQT_BUCKETS);
        reader.read_sleb_i32_into(
            &mut threat_and_pair_psqt,
            THREAT_FEATURES * PSQT_BUCKETS,
            "threat PSQT weights",
        )?;

        let pair_weights =
            reader.read_i8_weights(PAIR_FEATURES * L1, "pawn-pair weights", static_bytes)?;
        reader.read_sleb_i32_into(
            &mut threat_and_pair_psqt,
            PAIR_FEATURES * PSQT_BUCKETS,
            "pawn-pair PSQT weights",
        )?;

        let piece_weights = reader.read_sleb_i16(PSQ_FEATURES * L1, "piece weights")?;
        let piece_psqt = reader.read_sleb_i32(PSQ_FEATURES * PSQT_BUCKETS, "piece PSQT weights")?;

        let mut layers = Vec::with_capacity(LAYER_STACKS);
        for bucket in 0..LAYER_STACKS {
            let architecture_hash = reader.read_u32("layer-stack architecture hash")?;
            if architecture_hash != ARCHITECTURE_HASH {
                return Err(format!(
                    "Stockfish layer stack {bucket} has architecture hash 0x{architecture_hash:08x}; expected 0x{ARCHITECTURE_HASH:08x}"
                ));
            }
            layers.push(LayerStack::read(&mut reader, bucket)?);
        }

        if reader.remaining() != 0 {
            return Err(format!(
                "Stockfish NNUE contains {} trailing bytes",
                reader.remaining()
            ));
        }

        let layers: Box<[LayerStack; LAYER_STACKS]> = layers
            .into_boxed_slice()
            .try_into()
            .map_err(|_| "Stockfish NNUE layer-stack count mismatch".to_string())?;

        Ok(Box::new(Self {
            feature_biases,
            threat_weights,
            pair_weights,
            threat_and_pair_psqt: threat_and_pair_psqt.into_boxed_slice(),
            piece_weights,
            piece_psqt,
            layers,
        }))
    }

    pub fn parameter_bytes(&self) -> usize {
        self.feature_biases.len() * size_of::<i16>()
            + self.threat_weights.len() * size_of::<i8>()
            + self.pair_weights.len() * size_of::<i8>()
            + self.threat_and_pair_psqt.len() * size_of::<i32>()
            + self.piece_weights.len() * size_of::<i16>()
            + self.piece_psqt.len() * size_of::<i32>()
            + self
                .layers
                .iter()
                .map(LayerStack::parameter_bytes)
                .sum::<usize>()
    }

    /// Evaluates a position with Stockfish's current HalfKAv2_hm + FullThreats + PP_3Wide
    /// architecture. This reference path deliberately performs a full accumulator refresh; it is
    /// used to establish bit-exact compatibility before incremental and SIMD acceleration are
    /// enabled.
    pub fn evaluate(&self, board: &Board) -> i32 {
        let piece_count = board.all_occupancy().count_ones() as usize;
        let bucket = piece_count.saturating_sub(1) / 4;
        let mut accumulators = [[0_i32; L1]; 2];
        let mut psqt = [[0_i32; PSQT_BUCKETS]; 2];

        for perspective in 0..2 {
            accumulators[perspective]
                .iter_mut()
                .zip(self.feature_biases.iter().copied())
                .for_each(|(target, bias)| *target = i32::from(bias));
            self.accumulate_piece_features(
                board,
                perspective,
                &mut accumulators[perspective],
                &mut psqt[perspective],
            );
            self.accumulate_threat_features(
                board,
                perspective,
                &mut accumulators[perspective],
                &mut psqt[perspective],
            );
            self.accumulate_pawn_pair_features(
                board,
                perspective,
                &mut accumulators[perspective],
                &mut psqt[perspective],
            );
        }

        let stm = board.side_to_move.index();
        let material = (psqt[stm][bucket] - psqt[stm ^ 1][bucket]) / 2;
        let mut transformed = [0_u8; L1];
        for (half, perspective) in [stm, stm ^ 1].into_iter().enumerate() {
            let output = &mut transformed[half * (L1 / 2)..(half + 1) * (L1 / 2)];
            let accumulator = &accumulators[perspective];
            for (index, target) in output.iter_mut().enumerate() {
                let first = accumulator[index].clamp(0, 255) as u32;
                let second = accumulator[index + L1 / 2].clamp(0, 255) as u32;
                *target = ((first * second) / 512) as u8;
            }
        }

        material / 16 + self.layers[bucket].forward(&transformed) / 16
    }

    fn accumulate_piece_features(
        &self,
        board: &Board,
        perspective: usize,
        accumulator: &mut [i32; L1],
        psqt: &mut [i32; PSQT_BUCKETS],
    ) {
        let king_square = board.king_square(color(perspective)).index();
        for piece_color in 0..2 {
            for piece in Piece::ALL {
                let mut pieces = board.pieces[piece_color][piece.index()];
                while pieces != 0 {
                    let square = pieces.trailing_zeros() as usize;
                    pieces &= pieces - 1;
                    let feature = piece_feature_index(
                        piece_color,
                        piece.index(),
                        square,
                        king_square,
                        perspective,
                    );
                    accumulate_i16_row(accumulator, &self.piece_weights, feature);
                    accumulate_i32_row(psqt, &self.piece_psqt, feature);
                }
            }
        }
    }

    fn accumulate_threat_features(
        &self,
        board: &Board,
        perspective: usize,
        accumulator: &mut [i32; L1],
        psqt: &mut [i32; PSQT_BUCKETS],
    ) {
        let king_square = board.king_square(color(perspective)).index();
        let occupied = board.all_occupancy();
        let pawn_targets = pieces_of(board, Piece::Knight) | pieces_of(board, Piece::Rook);
        let minor_slider_targets = pieces_of(board, Piece::Pawn)
            | pieces_of(board, Piece::Knight)
            | pieces_of(board, Piece::Bishop)
            | pieces_of(board, Piece::Rook);
        let queen_targets = minor_slider_targets | pieces_of(board, Piece::Queen);

        for attacker_color in 0..2 {
            let mut pawns = board.pieces[attacker_color][Piece::Pawn.index()];
            while pawns != 0 {
                let from = pawns.trailing_zeros() as usize;
                pawns &= pawns - 1;
                let mut targets = pawn_attacks(attacker_color, from) & pawn_targets;
                while targets != 0 {
                    let to = targets.trailing_zeros() as usize;
                    targets &= targets - 1;
                    self.accumulate_threat(
                        board,
                        perspective,
                        king_square,
                        Piece::Pawn.index(),
                        attacker_color,
                        from,
                        to,
                        accumulator,
                        psqt,
                    );
                }
            }

            for attacker_piece in [Piece::Knight, Piece::Bishop, Piece::Rook, Piece::Queen] {
                let targets_mask = if matches!(attacker_piece, Piece::Knight | Piece::Queen) {
                    queen_targets
                } else {
                    minor_slider_targets
                };
                let mut attackers = board.pieces[attacker_color][attacker_piece.index()];
                while attackers != 0 {
                    let from = attackers.trailing_zeros() as usize;
                    attackers &= attackers - 1;
                    let mut targets =
                        piece_attacks(attacker_piece.index(), attacker_color, from, occupied)
                            & targets_mask;
                    while targets != 0 {
                        let to = targets.trailing_zeros() as usize;
                        targets &= targets - 1;
                        self.accumulate_threat(
                            board,
                            perspective,
                            king_square,
                            attacker_piece.index(),
                            attacker_color,
                            from,
                            to,
                            accumulator,
                            psqt,
                        );
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn accumulate_threat(
        &self,
        board: &Board,
        perspective: usize,
        king_square: usize,
        attacker_piece: usize,
        attacker_color: usize,
        from: usize,
        to: usize,
        accumulator: &mut [i32; L1],
        psqt: &mut [i32; PSQT_BUCKETS],
    ) {
        let Some((attacked_piece, attacked_color)) = board.piece_on(types::Square::from_index(to))
        else {
            return;
        };
        if let Some(feature) = threat_feature_index(
            attacker_piece,
            attacker_color,
            from,
            attacked_piece.index(),
            attacked_color.index(),
            to,
            king_square,
            perspective,
        ) {
            accumulate_i8_row(accumulator, self.threat_weights.as_slice(), feature);
            accumulate_i32_row(psqt, &self.threat_and_pair_psqt, feature);
        }
    }

    fn accumulate_pawn_pair_features(
        &self,
        board: &Board,
        perspective: usize,
        accumulator: &mut [i32; L1],
        psqt: &mut [i32; PSQT_BUCKETS],
    ) {
        let king_square = board.king_square(color(perspective)).index();
        let pawns = [
            board.pieces[0][Piece::Pawn.index()],
            board.pieces[1][Piece::Pawn.index()],
        ];
        for first_color in 0..2 {
            let mut first_pawns = pawns[first_color];
            while first_pawns != 0 {
                let first = first_pawns.trailing_zeros() as usize;
                first_pawns &= first_pawns - 1;
                for (second_color, &second_pawns) in pawns.iter().enumerate().skip(first_color) {
                    let mut partners = second_pawns & pawn_pair_mask(first);
                    if second_color == first_color {
                        partners &= !((1_u64 << (first + 1)) - 1);
                    }
                    while partners != 0 {
                        let second = partners.trailing_zeros() as usize;
                        partners &= partners - 1;
                        let feature = pawn_pair_feature_index(
                            first_color,
                            first,
                            second_color,
                            second,
                            king_square,
                            perspective,
                        );
                        accumulate_i8_row(
                            accumulator,
                            self.pair_weights.as_slice(),
                            feature - THREAT_FEATURES,
                        );
                        accumulate_i32_row(psqt, &self.threat_and_pair_psqt, feature);
                    }
                }
            }
        }
    }

    fn evaluate_accumulators(
        &self,
        board: &Board,
        accumulators: &[[i16; L1]; 2],
        psqt: &[[i32; PSQT_BUCKETS]; 2],
    ) -> i32 {
        let piece_count = board.all_occupancy().count_ones() as usize;
        let bucket = piece_count.saturating_sub(1) / 4;
        let stm = board.side_to_move.index();
        let material = (psqt[stm][bucket] - psqt[stm ^ 1][bucket]) / 2;
        let mut transformed = [0_u8; L1];
        for (half, perspective) in [stm, stm ^ 1].into_iter().enumerate() {
            let output = &mut transformed[half * (L1 / 2)..(half + 1) * (L1 / 2)];
            let accumulator = &accumulators[perspective];
            super::stockfish_simd::transform_pair(
                &accumulator[..L1 / 2],
                &accumulator[L1 / 2..],
                output,
            );
        }
        material / 16 + self.layers[bucket].forward(&transformed) / 16
    }
}

const STOCKFISH_STATE_FRAMES: usize = 256;
const MAX_DIRTY_THREAT_DELTAS: usize = 96;
const MAX_PIECE_FEATURES: usize = 32;
const MAX_THREAT_FEATURES: usize = 256;
const MAX_PAIR_FEATURES: usize = 256;

#[derive(Clone)]
struct FeatureLists {
    pieces: [u16; MAX_PIECE_FEATURES],
    piece_count: usize,
    threats: [u16; MAX_THREAT_FEATURES],
    threat_count: usize,
    pairs: [u16; MAX_PAIR_FEATURES],
    pair_count: usize,
}

impl Default for FeatureLists {
    fn default() -> Self {
        Self {
            pieces: [0; MAX_PIECE_FEATURES],
            piece_count: 0,
            threats: [0; MAX_THREAT_FEATURES],
            threat_count: 0,
            pairs: [0; MAX_PAIR_FEATURES],
            pair_count: 0,
        }
    }
}

impl FeatureLists {
    fn collect_both(board: &Board) -> [Self; 2] {
        let mut lists = std::array::from_fn(|_| Self::default());
        let king_squares = [
            board.king_square(Color::White).index(),
            board.king_square(Color::Black).index(),
        ];

        for piece_color in 0..2 {
            for piece in Piece::ALL {
                let mut pieces = board.pieces[piece_color][piece.index()];
                while pieces != 0 {
                    let square = pieces.trailing_zeros() as usize;
                    pieces &= pieces - 1;
                    for (perspective, list) in lists.iter_mut().enumerate() {
                        list.push_piece(piece_feature_index(
                            piece_color,
                            piece.index(),
                            square,
                            king_squares[perspective],
                            perspective,
                        ));
                    }
                }
            }
        }

        let occupied = board.all_occupancy();
        let pawn_targets = pieces_of(board, Piece::Knight) | pieces_of(board, Piece::Rook);
        let minor_slider_targets = pieces_of(board, Piece::Pawn)
            | pieces_of(board, Piece::Knight)
            | pieces_of(board, Piece::Bishop)
            | pieces_of(board, Piece::Rook);
        let queen_targets = minor_slider_targets | pieces_of(board, Piece::Queen);
        for attacker_color in 0..2 {
            let mut pawns = board.pieces[attacker_color][Piece::Pawn.index()];
            while pawns != 0 {
                let from = pawns.trailing_zeros() as usize;
                pawns &= pawns - 1;
                let mut targets = pawn_attacks(attacker_color, from) & pawn_targets;
                while targets != 0 {
                    let to = targets.trailing_zeros() as usize;
                    targets &= targets - 1;
                    let (attacked_piece, attacked_color) = board
                        .piece_on(types::Square::from_index(to))
                        .expect("threat targets are occupied");
                    for (perspective, list) in lists.iter_mut().enumerate() {
                        if let Some(feature) = threat_feature_index(
                            Piece::Pawn.index(),
                            attacker_color,
                            from,
                            attacked_piece.index(),
                            attacked_color.index(),
                            to,
                            king_squares[perspective],
                            perspective,
                        ) {
                            list.push_threat(feature);
                        }
                    }
                }
            }
            for attacker_piece in [Piece::Knight, Piece::Bishop, Piece::Rook, Piece::Queen] {
                let targets_mask = if matches!(attacker_piece, Piece::Knight | Piece::Queen) {
                    queen_targets
                } else {
                    minor_slider_targets
                };
                let mut attackers = board.pieces[attacker_color][attacker_piece.index()];
                while attackers != 0 {
                    let from = attackers.trailing_zeros() as usize;
                    attackers &= attackers - 1;
                    let mut targets =
                        piece_attacks(attacker_piece.index(), attacker_color, from, occupied)
                            & targets_mask;
                    while targets != 0 {
                        let to = targets.trailing_zeros() as usize;
                        targets &= targets - 1;
                        let (attacked_piece, attacked_color) = board
                            .piece_on(types::Square::from_index(to))
                            .expect("threat targets are occupied");
                        for (perspective, list) in lists.iter_mut().enumerate() {
                            if let Some(feature) = threat_feature_index(
                                attacker_piece.index(),
                                attacker_color,
                                from,
                                attacked_piece.index(),
                                attacked_color.index(),
                                to,
                                king_squares[perspective],
                                perspective,
                            ) {
                                list.push_threat(feature);
                            }
                        }
                    }
                }
            }
        }

        let pawns = [
            board.pieces[0][Piece::Pawn.index()],
            board.pieces[1][Piece::Pawn.index()],
        ];
        for first_color in 0..2 {
            let mut first_pawns = pawns[first_color];
            while first_pawns != 0 {
                let first = first_pawns.trailing_zeros() as usize;
                first_pawns &= first_pawns - 1;
                for (second_color, &second_pawns) in pawns.iter().enumerate().skip(first_color) {
                    let mut partners = second_pawns & pawn_pair_mask(first);
                    if second_color == first_color {
                        partners &= !((1_u64 << (first + 1)) - 1);
                    }
                    while partners != 0 {
                        let second = partners.trailing_zeros() as usize;
                        partners &= partners - 1;
                        for (perspective, list) in lists.iter_mut().enumerate() {
                            list.push_pair(pawn_pair_feature_index(
                                first_color,
                                first,
                                second_color,
                                second,
                                king_squares[perspective],
                                perspective,
                            ));
                        }
                    }
                }
            }
        }

        for list in &mut lists {
            list.pieces[..list.piece_count].sort_unstable();
            list.threats[..list.threat_count].sort_unstable();
            list.pairs[..list.pair_count].sort_unstable();
        }
        lists
    }

    fn push_piece(&mut self, feature: usize) {
        assert!(
            self.piece_count < MAX_PIECE_FEATURES,
            "too many piece features"
        );
        self.pieces[self.piece_count] = feature as u16;
        self.piece_count += 1;
    }

    fn push_threat(&mut self, feature: usize) {
        assert!(
            self.threat_count < MAX_THREAT_FEATURES,
            "too many threat features"
        );
        self.threats[self.threat_count] = feature as u16;
        self.threat_count += 1;
    }

    fn push_pair(&mut self, feature: usize) {
        assert!(
            self.pair_count < MAX_PAIR_FEATURES,
            "too many pawn-pair features"
        );
        self.pairs[self.pair_count] = feature as u16;
        self.pair_count += 1;
    }
}

#[derive(Clone)]
struct StockfishFrame {
    accumulators: [[i16; L1]; 2],
    psqt: [[i32; PSQT_BUCKETS]; 2],
    threat_deltas: [ThreatDelta; MAX_DIRTY_THREAT_DELTAS],
    threat_delta_count: usize,
    threat_overflowed: bool,
    pending_threats: Option<ThreatSnapshot>,
    pending_move: Option<Move>,
    pending_mover: u8,
    pending_captured: u8,
    pawns_before: [u64; 2],
    king_squares: [u8; 2],
    position_hash: u64,
    accurate: bool,
    pending_null: bool,
}

impl Default for StockfishFrame {
    fn default() -> Self {
        Self {
            accumulators: [[0; L1]; 2],
            psqt: [[0; PSQT_BUCKETS]; 2],
            threat_deltas: [ThreatDelta::default(); MAX_DIRTY_THREAT_DELTAS],
            threat_delta_count: 0,
            threat_overflowed: false,
            pending_threats: None,
            pending_move: None,
            pending_mover: u8::MAX,
            pending_captured: u8::MAX,
            pawns_before: [0; 2],
            king_squares: [u8::MAX; 2],
            position_hash: 0,
            accurate: false,
            pending_null: false,
        }
    }
}

impl ThreatDeltaSink for StockfishFrame {
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

pub(crate) struct StockfishAccumulatorState {
    frames: Box<[StockfishFrame]>,
    index: usize,
}

impl StockfishAccumulatorState {
    pub(crate) fn new() -> Self {
        Self {
            frames: vec![StockfishFrame::default(); STOCKFISH_STATE_FRAMES].into_boxed_slice(),
            index: 0,
        }
    }

    #[inline]
    pub(crate) fn push_move(&mut self, board: &Board, mv: Move) {
        assert!(
            self.index + 1 < self.frames.len(),
            "Stockfish NNUE stack exhausted"
        );
        self.index += 1;
        let frame = &mut self.frames[self.index];
        frame.accurate = false;
        frame.pending_null = false;
        frame.pending_move = Some(mv);
        frame.pending_mover = board.piece_ids()[mv.from.index()];
        frame.pending_captured = board.piece_ids()[mv.to.index()];
        frame.pawns_before = [
            board.pieces[0][Piece::Pawn.index()],
            board.pieces[1][Piece::Pawn.index()],
        ];
        frame.threat_delta_count = 0;
        frame.threat_overflowed = false;
        frame.pending_threats = Some(ThreatSnapshot::from_board(board));
    }

    #[inline]
    pub(crate) fn push_null(&mut self) {
        assert!(
            self.index + 1 < self.frames.len(),
            "Stockfish NNUE stack exhausted"
        );
        let next = self.index + 1;
        let (before, after) = self.frames.split_at_mut(next);
        after[0].clone_from(&before[self.index]);
        after[0].pending_move = None;
        after[0].pending_threats = None;
        after[0].pending_null = true;
        self.index = next;
    }

    #[inline]
    pub(crate) fn pop(&mut self) {
        assert!(self.index != 0, "cannot pop the root Stockfish NNUE frame");
        self.index -= 1;
    }

    pub(crate) fn clear(&mut self) {
        self.index = 0;
        self.frames[0].accurate = false;
        self.frames[0].pending_null = false;
    }

    pub(crate) fn evaluate(&mut self, board: &Board, network: &StockfishNetwork) -> i32 {
        if self.frames[self.index].accurate && self.frames[self.index].pending_null {
            self.frames[self.index].position_hash = board.hash;
            self.frames[self.index].pending_null = false;
        }
        if !self.frames[self.index].accurate || self.frames[self.index].position_hash != board.hash
        {
            if self.index != 0 && self.frames[self.index - 1].accurate {
                self.update_from_parent(board, network);
            } else {
                self.refresh(board, network);
            }
        }
        let frame = &self.frames[self.index];
        network.evaluate_accumulators(board, &frame.accumulators, &frame.psqt)
    }

    fn refresh(&mut self, board: &Board, network: &StockfishNetwork) {
        let frame = &mut self.frames[self.index];
        frame.psqt = [[0; PSQT_BUCKETS]; 2];
        let features = FeatureLists::collect_both(board);
        for (perspective, perspective_features) in features.iter().enumerate() {
            frame.accumulators[perspective].copy_from_slice(&network.feature_biases);
            apply_all_features(
                &mut frame.accumulators[perspective],
                &mut frame.psqt[perspective],
                network,
                perspective_features,
            );
        }
        frame.king_squares = current_king_squares(board);
        frame.position_hash = board.hash;
        frame.accurate = true;
        frame.pending_threats = None;
        frame.pending_null = false;
    }

    fn update_from_parent(&mut self, board: &Board, network: &StockfishNetwork) {
        let current = self.index;
        let king_squares = current_king_squares(board);
        let pawns_after = [
            board.pieces[0][Piece::Pawn.index()],
            board.pieces[1][Piece::Pawn.index()],
        ];
        let (before, after) = self.frames.split_at_mut(current);
        let parent = &before[current - 1];
        let frame = &mut after[0];
        if let (Some(snapshot), Some(mv)) = (frame.pending_threats.take(), frame.pending_move) {
            collect_snapshot_move_deltas(frame, snapshot, mv);
        }
        let needs_refresh = [
            frame.threat_overflowed || parent.king_squares[0] != king_squares[0],
            frame.threat_overflowed || parent.king_squares[1] != king_squares[1],
        ];
        let refresh_features = needs_refresh
            .iter()
            .any(|&refresh| refresh)
            .then(|| FeatureLists::collect_both(board));

        for perspective in 0..2 {
            if needs_refresh[perspective] {
                frame.accumulators[perspective].copy_from_slice(&network.feature_biases);
                frame.psqt[perspective] = [0; PSQT_BUCKETS];
                apply_all_features(
                    &mut frame.accumulators[perspective],
                    &mut frame.psqt[perspective],
                    network,
                    &refresh_features.as_ref().expect("refresh features exist")[perspective],
                );
                continue;
            }

            frame.accumulators[perspective] = parent.accumulators[perspective];
            frame.psqt[perspective] = parent.psqt[perspective];
            apply_piece_move_delta(
                &mut frame.accumulators[perspective],
                &mut frame.psqt[perspective],
                network,
                frame.pending_move,
                frame.pending_mover,
                frame.pending_captured,
                usize::from(king_squares[perspective]),
                perspective,
            );
            apply_threat_deltas(
                &mut frame.accumulators[perspective],
                &mut frame.psqt[perspective],
                network,
                &frame.threat_deltas[..frame.threat_delta_count],
                usize::from(king_squares[perspective]),
                perspective,
            );
            if frame.pawns_before != pawns_after {
                apply_pawn_pair_delta(
                    &mut frame.accumulators[perspective],
                    &mut frame.psqt[perspective],
                    network,
                    frame.pawns_before,
                    pawns_after,
                    usize::from(king_squares[perspective]),
                    perspective,
                );
            }
        }
        frame.king_squares = king_squares;
        frame.position_hash = board.hash;
        frame.accurate = true;
        frame.pending_null = false;
    }
}

#[inline(always)]
fn current_king_squares(board: &Board) -> [u8; 2] {
    [
        board.king_square(Color::White).index() as u8,
        board.king_square(Color::Black).index() as u8,
    ]
}

#[allow(clippy::too_many_arguments)]
fn apply_piece_move_delta(
    accumulator: &mut [i16; L1],
    psqt: &mut [i32; PSQT_BUCKETS],
    network: &StockfishNetwork,
    mv: Option<Move>,
    mover: u8,
    captured: u8,
    king_square: usize,
    perspective: usize,
) {
    let Some(mv) = mv else {
        return;
    };
    debug_assert_ne!(mover, u8::MAX);
    let mover_piece = usize::from(mover) / 2;
    let mover_color = usize::from(mover) & 1;
    let resulting_piece = mv.promotion.map_or(mover_piece, Piece::index);

    let mut adds = [0usize; 2];
    let mut subs = [0usize; 2];
    let mut add_count = 1;
    let mut sub_count = 1;
    adds[0] = piece_feature_index(
        mover_color,
        resulting_piece,
        mv.to.index(),
        king_square,
        perspective,
    );
    subs[0] = piece_feature_index(
        mover_color,
        mover_piece,
        mv.from.index(),
        king_square,
        perspective,
    );

    if mv.is_capture() && mv.flag != MoveFlag::EnPassant {
        debug_assert_ne!(captured, u8::MAX);
        subs[sub_count] = piece_feature_index(
            usize::from(captured) & 1,
            usize::from(captured) / 2,
            mv.to.index(),
            king_square,
            perspective,
        );
        sub_count += 1;
    } else if mv.flag == MoveFlag::EnPassant {
        let captured_square = Square::from_file_rank(mv.to.file(), mv.from.rank()).index();
        subs[sub_count] = piece_feature_index(
            mover_color ^ 1,
            Piece::Pawn.index(),
            captured_square,
            king_square,
            perspective,
        );
        sub_count += 1;
    } else if mv.is_castling() {
        let (rook_from, rook_to) = match (mover_color, mv.flag) {
            (0, MoveFlag::KingCastle) => (Square::H1.index(), Square::F1.index()),
            (0, MoveFlag::QueenCastle) => (Square::A1.index(), Square::D1.index()),
            (1, MoveFlag::KingCastle) => (Square::H8.index(), Square::F8.index()),
            (1, MoveFlag::QueenCastle) => (Square::A8.index(), Square::D8.index()),
            _ => unreachable!(),
        };
        adds[add_count] = piece_feature_index(
            mover_color,
            Piece::Rook.index(),
            rook_to,
            king_square,
            perspective,
        );
        subs[sub_count] = piece_feature_index(
            mover_color,
            Piece::Rook.index(),
            rook_from,
            king_square,
            perspective,
        );
        add_count += 1;
        sub_count += 1;
    }

    for &feature in &subs[..sub_count] {
        apply_i16_feature(accumulator, &network.piece_weights, feature, -1);
        apply_psqt_feature(psqt, &network.piece_psqt, feature, -1);
    }
    for &feature in &adds[..add_count] {
        apply_i16_feature(accumulator, &network.piece_weights, feature, 1);
        apply_psqt_feature(psqt, &network.piece_psqt, feature, 1);
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_threat_deltas(
    accumulator: &mut [i16; L1],
    psqt: &mut [i32; PSQT_BUCKETS],
    network: &StockfishNetwork,
    deltas: &[ThreatDelta],
    king_square: usize,
    perspective: usize,
) {
    for &delta in deltas {
        let attacker = delta.attacker();
        let attacked = delta.attacked();
        if let Some(feature) = threat_feature_index(
            attacker / 2,
            attacker & 1,
            delta.source(),
            attacked / 2,
            attacked & 1,
            delta.target(),
            king_square,
            perspective,
        ) {
            let sign = if delta.add() { 1 } else { -1 };
            apply_i8_feature(
                accumulator,
                network.threat_weights.as_slice(),
                feature,
                sign,
            );
            apply_psqt_feature(psqt, &network.threat_and_pair_psqt, feature, sign);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_pawn_pair_delta(
    accumulator: &mut [i16; L1],
    psqt: &mut [i32; PSQT_BUCKETS],
    network: &StockfishNetwork,
    before: [u64; 2],
    after: [u64; 2],
    king_square: usize,
    perspective: usize,
) {
    let old = collect_pawn_pair_features(before, king_square, perspective);
    let new = collect_pawn_pair_features(after, king_square, perspective);
    apply_diff(
        &old.pairs[..old.pair_count],
        &new.pairs[..new.pair_count],
        |feature, sign| {
            apply_i8_feature(
                accumulator,
                network.pair_weights.as_slice(),
                feature - THREAT_FEATURES,
                sign,
            );
            apply_psqt_feature(psqt, &network.threat_and_pair_psqt, feature, sign);
        },
    );
}

fn collect_pawn_pair_features(
    pawns: [u64; 2],
    king_square: usize,
    perspective: usize,
) -> FeatureLists {
    let mut list = FeatureLists::default();
    for first_color in 0..2 {
        let mut first_pawns = pawns[first_color];
        while first_pawns != 0 {
            let first = first_pawns.trailing_zeros() as usize;
            first_pawns &= first_pawns - 1;
            for (second_color, &second_pawns) in pawns.iter().enumerate().skip(first_color) {
                let mut partners = second_pawns & pawn_pair_mask(first);
                if second_color == first_color {
                    partners &= !((1_u64 << (first + 1)) - 1);
                }
                while partners != 0 {
                    let second = partners.trailing_zeros() as usize;
                    partners &= partners - 1;
                    list.push_pair(pawn_pair_feature_index(
                        first_color,
                        first,
                        second_color,
                        second,
                        king_square,
                        perspective,
                    ));
                }
            }
        }
    }
    list.pairs[..list.pair_count].sort_unstable();
    list
}

fn apply_all_features(
    accumulator: &mut [i16; L1],
    psqt: &mut [i32; PSQT_BUCKETS],
    network: &StockfishNetwork,
    features: &FeatureLists,
) {
    for &feature in &features.pieces[..features.piece_count] {
        apply_i16_feature(accumulator, &network.piece_weights, usize::from(feature), 1);
        apply_psqt_feature(psqt, &network.piece_psqt, usize::from(feature), 1);
    }
    for &feature in &features.threats[..features.threat_count] {
        apply_i8_feature(
            accumulator,
            network.threat_weights.as_slice(),
            usize::from(feature),
            1,
        );
        apply_psqt_feature(psqt, &network.threat_and_pair_psqt, usize::from(feature), 1);
    }
    for &feature in &features.pairs[..features.pair_count] {
        apply_i8_feature(
            accumulator,
            network.pair_weights.as_slice(),
            usize::from(feature) - THREAT_FEATURES,
            1,
        );
        apply_psqt_feature(psqt, &network.threat_and_pair_psqt, usize::from(feature), 1);
    }
}

fn apply_diff(old: &[u16], new: &[u16], mut apply: impl FnMut(usize, i16)) {
    let (mut old_index, mut new_index) = (0, 0);
    while old_index < old.len() && new_index < new.len() {
        match old[old_index].cmp(&new[new_index]) {
            std::cmp::Ordering::Less => {
                apply(usize::from(old[old_index]), -1);
                old_index += 1;
            }
            std::cmp::Ordering::Greater => {
                apply(usize::from(new[new_index]), 1);
                new_index += 1;
            }
            std::cmp::Ordering::Equal => {
                old_index += 1;
                new_index += 1;
            }
        }
    }
    for &feature in &old[old_index..] {
        apply(usize::from(feature), -1);
    }
    for &feature in &new[new_index..] {
        apply(usize::from(feature), 1);
    }
}

fn apply_i16_feature(accumulator: &mut [i16; L1], weights: &[i16], feature: usize, sign: i16) {
    super::stockfish_simd::apply_i16_feature(accumulator, weights, feature, sign);
}

fn apply_i8_feature(accumulator: &mut [i16; L1], weights: &[i8], feature: usize, sign: i16) {
    super::stockfish_simd::apply_i8_feature(accumulator, weights, feature, sign);
}

fn apply_psqt_feature(
    accumulator: &mut [i32; PSQT_BUCKETS],
    weights: &[i32],
    feature: usize,
    sign: i16,
) {
    let row = &weights[feature * PSQT_BUCKETS..(feature + 1) * PSQT_BUCKETS];
    for (target, &weight) in accumulator.iter_mut().zip(row) {
        *target = target.wrapping_add(weight.wrapping_mul(i32::from(sign)));
    }
}

impl LayerStack {
    fn read(reader: &mut ParameterReader<'_>, bucket: usize) -> Result<Self, String> {
        let fc0_biases = reader.read_i32_array::<L2>(&format!("bucket {bucket} fc0 biases"))?;
        let fc0_weights = reader.read_i8(L1 * L2, &format!("bucket {bucket} fc0 weights"))?;
        let fc1_biases = reader.read_i32_array::<L3>(&format!("bucket {bucket} fc1 biases"))?;
        let fc1_weights = reader.read_i8(64 * L3, &format!("bucket {bucket} fc1 weights"))?;
        let [fc2_bias] = reader.read_i32_array::<1>(&format!("bucket {bucket} fc2 bias"))?;
        let fc2_weights = reader
            .read_i8(128, &format!("bucket {bucket} fc2 weights"))?
            .into_vec()
            .try_into()
            .map_err(|_| format!("bucket {bucket} fc2 weight count mismatch"))?;
        Ok(Self {
            fc0_biases,
            fc0_weights,
            fc1_biases,
            fc1_weights,
            fc2_bias,
            fc2_weights,
        })
    }

    fn parameter_bytes(&self) -> usize {
        self.fc0_biases.len() * size_of::<i32>()
            + self.fc0_weights.len()
            + self.fc1_biases.len() * size_of::<i32>()
            + self.fc1_weights.len()
            + size_of::<i32>()
            + self.fc2_weights.len()
    }

    fn forward(&self, input: &[u8; L1]) -> i32 {
        let mut fc0 = self.fc0_biases;
        affine(input, &self.fc0_weights, &mut fc0);

        let mut l1_activations = [0_u8; 64];
        activate(&fc0, &mut l1_activations, 7);

        let mut fc1 = self.fc1_biases;
        affine(&l1_activations, &self.fc1_weights, &mut fc1);

        let mut output_activations = [0_u8; 128];
        activate(&fc1, &mut output_activations[64..], 6);
        output_activations[..64].copy_from_slice(&l1_activations);

        let mut output = self.fc2_bias;
        for (&activation, &weight) in output_activations.iter().zip(&self.fc2_weights) {
            output += i32::from(activation) * i32::from(weight);
        }
        output += fc0[L2 - 2] - fc0[L2 - 1];
        ((i64::from(output) * 9_600) / 16_384) as i32
    }
}

fn affine<const INPUTS: usize, const OUTPUTS: usize>(
    input: &[u8; INPUTS],
    weights: &[i8],
    output: &mut [i32; OUTPUTS],
) {
    debug_assert_eq!(weights.len(), INPUTS * OUTPUTS);
    super::stockfish_simd::affine(input, weights, output);
}

fn activate<const N: usize>(input: &[i32; N], output: &mut [u8], scale_bits: u32) {
    debug_assert_eq!(output.len(), N * 2);
    for (index, &value) in input.iter().enumerate() {
        let narrowed = value.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
        let squared = i64::from(narrowed) * i64::from(narrowed);
        output[index] = ((squared >> (2 * scale_bits + 7)).min(127)) as u8;
        output[index + N] = (narrowed >> scale_bits).clamp(0, 127) as u8;
    }
}

#[inline(always)]
fn accumulate_i16_row(accumulator: &mut [i32; L1], weights: &[i16], feature: usize) {
    let row = &weights[feature * L1..(feature + 1) * L1];
    for (target, &weight) in accumulator.iter_mut().zip(row) {
        *target += i32::from(weight);
    }
}

#[inline(always)]
fn accumulate_i8_row(accumulator: &mut [i32; L1], weights: &[i8], feature: usize) {
    let row = &weights[feature * L1..(feature + 1) * L1];
    for (target, &weight) in accumulator.iter_mut().zip(row) {
        *target += i32::from(weight);
    }
}

#[inline(always)]
fn accumulate_i32_row(accumulator: &mut [i32; PSQT_BUCKETS], weights: &[i32], feature: usize) {
    let row = &weights[feature * PSQT_BUCKETS..(feature + 1) * PSQT_BUCKETS];
    for (target, &weight) in accumulator.iter_mut().zip(row) {
        *target += weight;
    }
}

#[inline(always)]
fn color(index: usize) -> Color {
    if index == 0 {
        Color::White
    } else {
        Color::Black
    }
}

#[inline(always)]
fn pieces_of(board: &Board, piece: Piece) -> u64 {
    board.pieces[0][piece.index()] | board.pieces[1][piece.index()]
}

#[inline(always)]
fn piece_feature_index(
    piece_color: usize,
    piece: usize,
    square: usize,
    king_square: usize,
    perspective: usize,
) -> usize {
    const KING_BUCKETS: [usize; 64] = [
        28, 29, 30, 31, 31, 30, 29, 28, 24, 25, 26, 27, 27, 26, 25, 24, 20, 21, 22, 23, 23, 22, 21,
        20, 16, 17, 18, 19, 19, 18, 17, 16, 12, 13, 14, 15, 15, 14, 13, 12, 8, 9, 10, 11, 11, 10,
        9, 8, 4, 5, 6, 7, 7, 6, 5, 4, 0, 1, 2, 3, 3, 2, 1, 0,
    ];
    let vertical_flip = 56 * perspective;
    // HalfKAv2_hm OrientTBL flips files a-d. FullThreats / PP_3Wide flip e-h instead —
    // keep these conventions distinct or the network diverges from Stockfish.
    let orient = (7 * usize::from(king_square & 7 < 4)) ^ vertical_flip;
    let piece_offset = if piece == Piece::King.index() {
        640
    } else {
        piece * 128 + 64 * (piece_color ^ perspective)
    };
    (square ^ orient) + piece_offset + KING_BUCKETS[king_square ^ vertical_flip] * 704
}

#[inline(always)]
fn pawn_pair_mask(square: usize) -> u64 {
    const FILE_A: u64 = 0x0101_0101_0101_0101;
    const FILE_H: u64 = 0x8080_8080_8080_8080;
    const PAWN_RANKS: u64 = 0x00ff_ffff_ffff_ff00;
    let file = FILE_A << (square & 7);
    let files = file | ((file & !FILE_H) << 1) | ((file & !FILE_A) >> 1);
    files & PAWN_RANKS & !(1_u64 << square)
}

fn pawn_pair_feature_index(
    first_color: usize,
    first: usize,
    second_color: usize,
    second: usize,
    king_square: usize,
    perspective: usize,
) -> usize {
    let orient = (7 * usize::from(king_square & 7 >= 4)) ^ (56 * perspective);
    let first_id = (first_color ^ perspective) * 48 + (first ^ orient) - 8;
    let second_id = (second_color ^ perspective) * 48 + (second ^ orient) - 8;
    let hi = first_id.max(second_id);
    let lo = first_id.min(second_id);
    THREAT_FEATURES + hi * (hi - 1) / 2 + lo
}

#[inline(always)]
fn piece_attacks(piece: usize, piece_color: usize, square: usize, occupied: u64) -> u64 {
    match Piece::from_index(piece).expect("piece index is valid") {
        Piece::Pawn => pawn_attacks(piece_color, square),
        Piece::Knight => knight_attacks(square),
        Piece::Bishop => bishop_attacks(square, occupied),
        Piece::Rook => rook_attacks(square, occupied),
        Piece::Queen => queen_attacks(square, occupied),
        Piece::King => king_attacks(square),
    }
}

struct ThreatIndexTables {
    pair_base: [[i32; 12]; 12],
    excluded: [[bool; 12]; 12],
    semi_excluded: [[bool; 12]; 12],
    square_offsets: [[u32; 64]; 12],
    empty_attacks: [[u64; 64]; 12],
}

fn threat_tables() -> &'static ThreatIndexTables {
    static TABLES: OnceLock<ThreatIndexTables> = OnceLock::new();
    TABLES.get_or_init(ThreatIndexTables::new)
}

impl ThreatIndexTables {
    fn new() -> Self {
        #[rustfmt::skip]
        const INTERACTIONS: [[i32; 6]; 6] = [
            [-1, 0, -1, 1, -1, -1],
            [ 0, 1,  2, 3,  4, -1],
            [ 0, 1,  2, 3, -1, -1],
            [ 0, 1,  2, 3, -1, -1],
            [ 0, 1,  2, 3,  4, -1],
            [-1,-1, -1,-1, -1, -1],
        ];
        const TARGET_COUNTS: [i32; 6] = [4, 10, 8, 8, 10, 0];
        let mut square_offsets = [[0_u32; 64]; 12];
        let mut empty_attacks = [[0_u64; 64]; 12];
        let mut piece_counts = [0_i32; 12];
        let mut piece_bases = [0_i32; 12];
        let mut total = 0_i32;

        for piece_color in 0..2 {
            for (piece, &target_count) in TARGET_COUNTS.iter().enumerate() {
                let id = piece * 2 + piece_color;
                let mut count = 0_u32;
                for square in 0..64 {
                    square_offsets[id][square] = count;
                    let attacks = piece_attacks(piece, piece_color, square, 0);
                    empty_attacks[id][square] = attacks;
                    if piece != Piece::Pawn.index() || (8..56).contains(&square) {
                        count += attacks.count_ones();
                    }
                }
                piece_counts[id] = count as i32;
                piece_bases[id] = total;
                total += target_count * count as i32;
            }
        }
        debug_assert_eq!(total as usize, THREAT_FEATURES);

        let mut pair_base = [[0_i32; 12]; 12];
        let mut excluded = [[false; 12]; 12];
        let mut semi_excluded = [[false; 12]; 12];
        for attacker in 0..12 {
            for attacked in 0..12 {
                let attacker_piece = attacker / 2;
                let attacked_piece = attacked / 2;
                let interaction = INTERACTIONS[attacker_piece][attacked_piece];
                pair_base[attacker][attacked] = piece_bases[attacker]
                    + ((attacked & 1) as i32 * (TARGET_COUNTS[attacker_piece] / 2) + interaction)
                        * piece_counts[attacker];
                excluded[attacker][attacked] = interaction < 0;
                semi_excluded[attacker][attacked] = attacker_piece == attacked_piece
                    && ((attacker & 1) != (attacked & 1) || attacker_piece != Piece::Pawn.index());
            }
        }

        Self {
            pair_base,
            excluded,
            semi_excluded,
            square_offsets,
            empty_attacks,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn threat_feature_index(
    attacker_piece: usize,
    attacker_color: usize,
    mut from: usize,
    attacked_piece: usize,
    attacked_color: usize,
    mut to: usize,
    king_square: usize,
    perspective: usize,
) -> Option<usize> {
    let orient = (7 * usize::from(king_square & 7 >= 4)) ^ (56 * perspective);
    from ^= orient;
    to ^= orient;
    let attacker = attacker_piece * 2 + (attacker_color ^ perspective);
    let attacked = attacked_piece * 2 + (attacked_color ^ perspective);
    let tables = threat_tables();
    if tables.excluded[attacker][attacked]
        || (tables.semi_excluded[attacker][attacked] && from < to)
    {
        return None;
    }
    let lower_squares = (1_u64 << to).wrapping_sub(1);
    let attack_number = (tables.empty_attacks[attacker][from] & lower_squares).count_ones();
    let feature = tables.pair_base[attacker][attacked] as usize
        + tables.square_offsets[attacker][from] as usize
        + attack_number as usize;
    debug_assert!(feature < THREAT_FEATURES);
    Some(feature)
}

pub(crate) fn visit_threat_features(
    board: &Board,
    perspective: usize,
    mut visit: impl FnMut(usize),
) {
    let king_square = board.king_square(color(perspective)).index();
    let occupied = board.all_occupancy();
    let pawn_targets = pieces_of(board, Piece::Knight) | pieces_of(board, Piece::Rook);
    let minor_slider_targets = pieces_of(board, Piece::Pawn)
        | pieces_of(board, Piece::Knight)
        | pieces_of(board, Piece::Bishop)
        | pieces_of(board, Piece::Rook);
    let queen_targets = minor_slider_targets | pieces_of(board, Piece::Queen);

    for attacker_color in 0..2 {
        let mut pawns = board.pieces[attacker_color][Piece::Pawn.index()];
        while pawns != 0 {
            let from = pawns.trailing_zeros() as usize;
            pawns &= pawns - 1;
            let mut targets = pawn_attacks(attacker_color, from) & pawn_targets;
            while targets != 0 {
                let to = targets.trailing_zeros() as usize;
                targets &= targets - 1;
                visit_one_threat(
                    board,
                    perspective,
                    king_square,
                    Piece::Pawn.index(),
                    attacker_color,
                    from,
                    to,
                    &mut visit,
                );
            }
        }

        for attacker_piece in [Piece::Knight, Piece::Bishop, Piece::Rook, Piece::Queen] {
            let targets_mask = if matches!(attacker_piece, Piece::Knight | Piece::Queen) {
                queen_targets
            } else {
                minor_slider_targets
            };
            let mut attackers = board.pieces[attacker_color][attacker_piece.index()];
            while attackers != 0 {
                let from = attackers.trailing_zeros() as usize;
                attackers &= attackers - 1;
                let mut targets =
                    piece_attacks(attacker_piece.index(), attacker_color, from, occupied)
                        & targets_mask;
                while targets != 0 {
                    let to = targets.trailing_zeros() as usize;
                    targets &= targets - 1;
                    visit_one_threat(
                        board,
                        perspective,
                        king_square,
                        attacker_piece.index(),
                        attacker_color,
                        from,
                        to,
                        &mut visit,
                    );
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn visit_one_threat(
    board: &Board,
    perspective: usize,
    king_square: usize,
    attacker_piece: usize,
    attacker_color: usize,
    from: usize,
    to: usize,
    visit: &mut impl FnMut(usize),
) {
    let Some((attacked_piece, attacked_color)) = board.piece_on(types::Square::from_index(to))
    else {
        return;
    };
    if let Some(feature) = threat_feature_index(
        attacker_piece,
        attacker_color,
        from,
        attacked_piece.index(),
        attacked_color.index(),
        to,
        king_square,
        perspective,
    ) {
        visit(feature);
    }
}

pub(crate) fn visit_pawn_pair_features(
    board: &Board,
    perspective: usize,
    mut visit: impl FnMut(usize),
) {
    let king_square = board.king_square(color(perspective)).index();
    let pawns = [
        board.pieces[0][Piece::Pawn.index()],
        board.pieces[1][Piece::Pawn.index()],
    ];
    for first_color in 0..2 {
        let mut first_pawns = pawns[first_color];
        while first_pawns != 0 {
            let first = first_pawns.trailing_zeros() as usize;
            first_pawns &= first_pawns - 1;
            for (second_color, &second_pawns) in pawns.iter().enumerate().skip(first_color) {
                let mut partners = second_pawns & pawn_pair_mask(first);
                if second_color == first_color {
                    partners &= !((1_u64 << (first + 1)) - 1);
                }
                while partners != 0 {
                    let second = partners.trailing_zeros() as usize;
                    partners &= partners - 1;
                    visit(pawn_pair_feature_index(
                        first_color,
                        first,
                        second_color,
                        second,
                        king_square,
                        perspective,
                    ));
                }
            }
        }
    }
}

struct ParameterReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> ParameterReader<'a> {
    fn new(bytes: &'a [u8], offset: usize) -> Self {
        Self { bytes, offset }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn read_exact(&mut self, len: usize, section: &str) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| format!("Stockfish NNUE {section} length overflow"))?;
        let bytes = self.bytes.get(self.offset..end).ok_or_else(|| {
            format!(
                "Stockfish NNUE {section} is truncated at byte {}: needs {len} bytes, {} remain",
                self.offset,
                self.remaining()
            )
        })?;
        self.offset = end;
        Ok(bytes)
    }

    fn read_u32(&mut self, section: &str) -> Result<u32, String> {
        let bytes = self.read_exact(4, section)?;
        Ok(u32::from_le_bytes(
            bytes
                .try_into()
                .expect("a four-byte slice always converts to [u8; 4]"),
        ))
    }

    fn read_i8(&mut self, count: usize, section: &str) -> Result<Box<[i8]>, String> {
        let mut values = vec![0_i8; count];
        self.read_i8_into(&mut values, section)?;
        Ok(values.into_boxed_slice())
    }

    fn read_i8_weights(
        &mut self,
        count: usize,
        section: &str,
        static_bytes: Option<&'static [u8]>,
    ) -> Result<I8Weights, String> {
        let start = self.offset;
        let bytes = self.read_exact(count, section)?;
        if let Some(source) = static_bytes {
            let borrowed = source
                .get(start..start + count)
                .ok_or_else(|| format!("Stockfish NNUE {section} exceeds the static payload"))?;
            // SAFETY: i8 and u8 have identical size/alignment, every bit pattern is valid for
            // both, and the embedded payload has static lifetime.
            let borrowed = unsafe {
                std::slice::from_raw_parts(borrowed.as_ptr().cast::<i8>(), borrowed.len())
            };
            Ok(I8Weights::Borrowed(borrowed))
        } else {
            Ok(I8Weights::Owned(
                bytes
                    .iter()
                    .map(|&value| value as i8)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ))
        }
    }

    fn read_i8_into(&mut self, output: &mut [i8], section: &str) -> Result<(), String> {
        let bytes = self.read_exact(output.len(), section)?;
        for (target, &source) in output.iter_mut().zip(bytes) {
            *target = source as i8;
        }
        Ok(())
    }

    fn read_i32_array<const N: usize>(&mut self, section: &str) -> Result<[i32; N], String> {
        let bytes = self.read_exact(N * size_of::<i32>(), section)?;
        let mut values = [0_i32; N];
        for (target, chunk) in values.iter_mut().zip(bytes.chunks_exact(4)) {
            *target = i32::from_le_bytes(
                chunk
                    .try_into()
                    .expect("a four-byte chunk always converts to [u8; 4]"),
            );
        }
        Ok(values)
    }

    fn read_sleb_i16(&mut self, count: usize, section: &str) -> Result<Box<[i16]>, String> {
        let mut values = Vec::with_capacity(count);
        self.read_sleb_into(&mut values, count, section, |value| {
            i16::try_from(value).map_err(|_| format!("{section} value {value} exceeds i16"))
        })?;
        Ok(values.into_boxed_slice())
    }

    fn read_sleb_i32(&mut self, count: usize, section: &str) -> Result<Box<[i32]>, String> {
        let mut values = Vec::with_capacity(count);
        self.read_sleb_i32_into(&mut values, count, section)?;
        Ok(values.into_boxed_slice())
    }

    fn read_sleb_i32_into(
        &mut self,
        output: &mut Vec<i32>,
        count: usize,
        section: &str,
    ) -> Result<(), String> {
        self.read_sleb_into(output, count, section, Ok)
    }

    fn read_sleb_into<T>(
        &mut self,
        output: &mut Vec<T>,
        count: usize,
        section: &str,
        convert: impl Fn(i32) -> Result<T, String>,
    ) -> Result<(), String> {
        let magic = self.read_exact(LEB128_MAGIC.len(), section)?;
        if magic != LEB128_MAGIC {
            return Err(format!(
                "Stockfish NNUE {section} is missing the COMPRESSED_LEB128 marker"
            ));
        }
        let encoded_len = self.read_u32(section)? as usize;
        let encoded = self.read_exact(encoded_len, section)?;
        let initial_len = output.len();
        output.reserve(count);

        let mut cursor = 0;
        while output.len() - initial_len < count {
            let mut value = 0_u32;
            let mut shift = 0_u32;
            let terminal;
            loop {
                let byte = *encoded.get(cursor).ok_or_else(|| {
                    format!("Stockfish NNUE {section} LEB128 payload ends mid-value")
                })?;
                cursor += 1;
                value |= u32::from(byte & 0x7f) << shift;
                shift += 7;
                if byte & 0x80 == 0 {
                    terminal = byte;
                    break;
                }
                if shift >= 35 {
                    return Err(format!(
                        "Stockfish NNUE {section} contains oversized LEB128"
                    ));
                }
            }

            let signed = if shift < 32 && terminal & 0x40 != 0 {
                (value | (!0_u32 << shift)) as i32
            } else {
                value as i32
            };
            output.push(convert(signed)?);
        }

        if cursor != encoded.len() {
            return Err(format!(
                "Stockfish NNUE {section} decoded {count} values with {} unused payload bytes",
                encoded.len() - cursor
            ));
        }
        Ok(())
    }
}

pub fn unsupported_message(path: &Path) -> String {
    let mut header_bytes = [0_u8; 4_096];
    match File::open(path).and_then(|mut file| file.read(&mut header_bytes)) {
        Ok(read) => match parse_header(&header_bytes[..read]) {
            Ok(_) => format!(
                "Stockfish NNUE '{}' is valid for {NETWORK_FILENAME}, but its native evaluator is not enabled yet",
                path.display()
            ),
            Err(error) => format!("invalid Stockfish NNUE '{}': {error}", path.display()),
        },
        Err(error) => format!(
            "failed to read Stockfish NNUE '{}': {error}",
            path.display()
        ),
    }
}

#[inline]
fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| "Stockfish NNUE offset overflow".to_string())?;
    let raw = bytes
        .get(offset..end)
        .ok_or_else(|| format!("Stockfish NNUE is truncated at byte {offset}"))?;
    Ok(u32::from_le_bytes(
        raw.try_into()
            .expect("a four-byte slice always converts to [u8; 4]"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_network_has_the_pinned_current_header() {
        let header = validate_embedded().expect("embedded Stockfish network must be valid");
        assert_eq!(embedded_bytes().len(), FILE_SIZE);
        assert_eq!(header.version, FORMAT_VERSION);
        assert_eq!(header.network_hash, NETWORK_HASH);
        assert_eq!(header.feature_transformer_hash, FEATURE_TRANSFORMER_HASH);
        assert_eq!(header.parameters_offset, 100);
        assert_eq!(
            header.description,
            "Network trained with the https://github.com/official-stockfish/nnue-pytorch trainer."
        );
    }

    #[test]
    fn rejects_wrong_version_before_reading_parameters() {
        let mut bytes = embedded_bytes()[..128].to_vec();
        bytes[..4].copy_from_slice(&0_u32.to_le_bytes());
        let error = parse_header(&bytes).unwrap_err();
        assert!(error.contains("version"));
    }

    #[test]
    fn rejects_wrong_architecture_hash() {
        let mut bytes = embedded_bytes()[..128].to_vec();
        bytes[4..8].copy_from_slice(&0_u32.to_le_bytes());
        let error = parse_header(&bytes).unwrap_err();
        assert!(error.contains("architecture hash"));
    }

    #[test]
    fn rejects_truncated_description() {
        let error = parse_header(&embedded_bytes()[..32]).unwrap_err();
        assert!(error.contains("truncated"));
    }

    #[test]
    fn decodes_every_current_network_parameter_without_trailing_data() {
        let network = load_embedded().expect("current embedded Stockfish network must decode");
        assert_eq!(network.parameter_bytes(), 115_114_528);
        assert_eq!(network.feature_biases.len(), L1);
        assert_eq!(
            network.threat_weights.len() + network.pair_weights.len(),
            (THREAT_FEATURES + PAIR_FEATURES) * L1
        );
        assert!(matches!(network.threat_weights, I8Weights::Borrowed(_)));
        assert!(matches!(network.pair_weights, I8Weights::Borrowed(_)));
        assert_eq!(network.layers.len(), LAYER_STACKS);
    }

    #[test]
    fn current_network_matches_stockfish_start_position() {
        types::init();
        let network = load_embedded().expect("current embedded Stockfish network must decode");
        assert_eq!(network.evaluate(&Board::new()), 9);
    }

    #[test]
    fn current_network_matches_stockfish_feature_rich_positions() {
        types::init();
        let network = load_embedded().expect("current embedded Stockfish network must decode");
        let positions = [
            (
                "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2",
                28,
            ),
            (
                "r1bqkbnr/1ppp1ppp/p1n5/1B2p3/4P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 0 4",
                37,
            ),
            (
                "8/2p2pk1/1p1p2p1/pP1Pp2p/P3P2P/5PP1/2P3K1/8 b - - 0 40",
                -12,
            ),
        ];
        for (fen, expected) in positions {
            let board = Board::from_fen(fen).expect("test FEN is valid");
            assert_eq!(network.evaluate(&board), expected, "{fen}");
        }
    }

    #[test]
    fn incremental_state_matches_reference_after_moves_and_pop() {
        types::init();
        let network = load_embedded().expect("current embedded Stockfish network must decode");
        let mut state = StockfishAccumulatorState::new();
        let mut board = Board::new();
        let mut last_move = None;
        assert_eq!(state.evaluate(&board, &network), network.evaluate(&board));

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
            assert_eq!(
                state.evaluate(&board, &network),
                network.evaluate(&board),
                "{uci}"
            );
        }

        state.pop();
        board.unmake_move(last_move.expect("at least one move was made"));
        assert_eq!(state.evaluate(&board, &network), network.evaluate(&board));
    }

    #[test]
    fn threat_deltas_are_collected_only_when_the_child_is_evaluated() {
        types::init();
        let network = load_embedded().expect("current embedded Stockfish network must decode");
        let mut state = StockfishAccumulatorState::new();
        let mut board = Board::new();
        assert_eq!(state.evaluate(&board, &network), network.evaluate(&board));

        let mv = board
            .generate_legal_moves()
            .iter()
            .find(|mv| mv.to_uci() == "e2e4")
            .copied()
            .expect("e2e4 is legal");
        state.push_move(&board, mv);
        assert_eq!(state.frames[state.index].threat_delta_count, 0);
        assert!(state.frames[state.index].pending_threats.is_some());

        board.make_move(mv);
        assert_eq!(state.evaluate(&board, &network), network.evaluate(&board));
        assert!(state.frames[state.index].pending_threats.is_none());
    }

    #[test]
    fn dirty_state_matches_reference_for_special_moves() {
        types::init();
        let network = load_embedded().expect("current embedded Stockfish network must decode");
        let cases = [
            ("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1", "e1g1"),
            ("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1", "e5d6"),
            ("4k3/P7/8/8/8/8/8/4K3 w - - 0 1", "a7a8q"),
            ("r3k3/1P6/8/8/8/8/8/4K3 w - - 0 1", "b7a8n"),
        ];

        for (fen, uci) in cases {
            let mut state = StockfishAccumulatorState::new();
            let mut board = Board::from_fen(fen).expect("test FEN is valid");
            assert_eq!(state.evaluate(&board, &network), network.evaluate(&board));
            let mv = board
                .generate_legal_moves()
                .iter()
                .find(|mv| mv.to_uci() == uci)
                .copied()
                .expect("test move is legal");
            state.push_move(&board, mv);
            board.make_move(mv);
            assert_eq!(
                state.evaluate(&board, &network),
                network.evaluate(&board),
                "{fen}: {uci}"
            );
        }
    }

    #[test]
    fn null_move_reuses_the_exact_stockfish_accumulator() {
        types::init();
        let network = load_embedded().expect("current embedded Stockfish network must decode");
        let mut state = StockfishAccumulatorState::new();
        let mut board =
            Board::from_fen("r1bq1rk1/ppp2ppp/2n2n2/2bp4/4P3/2P2N2/PP1N1PPP/R1BQ1RK1 w - - 2 9")
                .expect("test FEN is valid");
        assert_eq!(state.evaluate(&board, &network), network.evaluate(&board));

        state.push_null();
        board.make_null_move();
        assert_eq!(state.evaluate(&board, &network), network.evaluate(&board));

        state.pop();
        board.unmake_null_move();
        assert_eq!(state.evaluate(&board, &network), network.evaluate(&board));
    }

    #[test]
    fn root_state_refreshes_when_the_position_changes() {
        types::init();
        let network = load_embedded().expect("current embedded Stockfish network must decode");
        let mut state = StockfishAccumulatorState::new();
        for fen in [
            "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2",
            "8/2p2pk1/1p1p2p1/pP1Pp2p/P3P2P/5PP1/2P3K1/8 b - - 0 40",
        ] {
            let board = Board::from_fen(fen).expect("test FEN is valid");
            assert_eq!(state.evaluate(&board, &network), network.evaluate(&board));
        }
    }
}
