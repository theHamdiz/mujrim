//! Loader and evaluator for Reckless v0.9 threat-aware raw networks.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::OnceLock;

use types::board::BoardObserver;
use types::board::attack_tables::{
    bishop_attacks, king_attacks, knight_attacks, pawn_attacks, queen_attacks, rook_attacks,
};
use types::chess_move::MoveFlag;
use types::{Board, Color, Move, Piece, Square};

use super::dirty_threats::{
    ThreatDelta, ThreatDeltaSink, collect_move_deltas, push_threats_on_change,
    push_threats_on_move, push_threats_on_mutate,
};

pub const FILE_SIZE: u64 = 63_266_880;
pub const HIDDEN_SIZE: usize = 768;
pub const INPUT_BUCKETS: usize = 10;
pub const OUTPUT_BUCKETS: usize = 8;

const THREAT_FEATURES: usize = 66_864;
pub(super) const L2_SIZE: usize = 16;
pub(super) const L3_SIZE: usize = 32;
const NETWORK_SCALE: f32 = 380.0;
const PIECE_CACHE_ENTRIES: usize = 2 * 2 * INPUT_BUCKETS;
const THREAT_STACK_SIZE: usize = 256;
const MAX_THREAT_DELTAS: usize = 80;

#[rustfmt::skip]
const INPUT_BUCKET_LAYOUT: [usize; 64] = [
    0, 1, 2, 3, 3, 2, 1, 0,
    4, 5, 6, 7, 7, 6, 5, 4,
    8, 8, 8, 8, 8, 8, 8, 8,
    9, 9, 9, 9, 9, 9, 9, 9,
    9, 9, 9, 9, 9, 9, 9, 9,
    9, 9, 9, 9, 9, 9, 9, 9,
    9, 9, 9, 9, 9, 9, 9, 9,
    9, 9, 9, 9, 9, 9, 9, 9,
];

#[rustfmt::skip]
const OUTPUT_BUCKET_LAYOUT: [usize; 33] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 1, 1, 1,
    2, 2, 2, 2,
    3, 3, 3,
    4, 4, 4,
    5, 5, 5,
    6, 6, 6,
    7, 7, 7, 7,
];

struct OwnedRecklessParameters {
    threat_weights: Box<[u8]>,
    piece_weights: Box<[i16]>,
    feature_biases: Box<[i16]>,
    l1_weights: Box<[u8]>,
    l1_biases: Box<[f32]>,
    l2_weights: Box<[f32]>,
    l2_biases: Box<[f32]>,
    l3_weights: Box<[f32]>,
    l3_biases: Box<[f32]>,
}

#[repr(C, align(64))]
#[cfg(any(feature = "embedded-networks", test))]
struct Aligned<T>(T);

#[repr(C)]
#[cfg(any(feature = "embedded-networks", test))]
struct EmbeddedRecklessParameters {
    threat_weights: Aligned<[[u8; HIDDEN_SIZE]; THREAT_FEATURES]>,
    piece_weights: Aligned<[[i16; HIDDEN_SIZE]; INPUT_BUCKETS * 768]>,
    feature_biases: Aligned<[i16; HIDDEN_SIZE]>,
    l1_weights: Aligned<[[u8; L2_SIZE * HIDDEN_SIZE]; OUTPUT_BUCKETS]>,
    l1_biases: Aligned<[[f32; L2_SIZE]; OUTPUT_BUCKETS]>,
    l2_weights: Aligned<[[[f32; L3_SIZE]; L2_SIZE]; OUTPUT_BUCKETS]>,
    l2_biases: Aligned<[[f32; L3_SIZE]; OUTPUT_BUCKETS]>,
    l3_weights: Aligned<[[f32; L3_SIZE]; OUTPUT_BUCKETS]>,
    l3_biases: Aligned<[f32; OUTPUT_BUCKETS]>,
}

enum RecklessStorage {
    #[cfg(feature = "embedded-networks")]
    Embedded(&'static EmbeddedRecklessParameters),
    Owned(OwnedRecklessParameters),
}

pub struct RecklessNetwork {
    storage: RecklessStorage,
}

#[cfg(feature = "embedded-networks")]
static EMBEDDED_PARAMETERS: EmbeddedRecklessParameters =
    unsafe { std::mem::transmute(*include_bytes!("../../resources/reckless_v60.nnue")) };

#[cfg(feature = "embedded-networks")]
static EMBEDDED_NETWORK: RecklessNetwork = RecklessNetwork {
    storage: RecklessStorage::Embedded(&EMBEDDED_PARAMETERS),
};

pub fn embedded() -> &'static RecklessNetwork {
    #[cfg(feature = "embedded-networks")]
    {
        &EMBEDDED_NETWORK
    }
    #[cfg(not(feature = "embedded-networks"))]
    {
        static NETWORK: OnceLock<Box<RecklessNetwork>> = OnceLock::new();
        NETWORK
            .get_or_init(|| {
                const SHA256: &str =
                    "7f587dfb1fe5d74d53909328afa6fd51650c8c7f45907602db7fbb1e52948c61";
                let path = super::adapter::discover_network_file(FILE_SIZE, SHA256)
                    .unwrap_or_else(|error| panic!("Mujrim v60 NNUE discovery failed: {error}"));
                load(&path).unwrap_or_else(|error| panic!("Mujrim v60 NNUE load failed: {error}"))
            })
            .as_ref()
    }
}

impl RecklessNetwork {
    #[inline(always)]
    fn threat_weights(&self) -> &[u8] {
        match &self.storage {
            #[cfg(feature = "embedded-networks")]
            RecklessStorage::Embedded(parameters) => flatten(&parameters.threat_weights.0),
            RecklessStorage::Owned(parameters) => &parameters.threat_weights,
        }
    }

    #[inline(always)]
    fn piece_weights(&self) -> &[i16] {
        match &self.storage {
            #[cfg(feature = "embedded-networks")]
            RecklessStorage::Embedded(parameters) => flatten(&parameters.piece_weights.0),
            RecklessStorage::Owned(parameters) => &parameters.piece_weights,
        }
    }

    #[inline(always)]
    fn feature_biases(&self) -> &[i16] {
        match &self.storage {
            #[cfg(feature = "embedded-networks")]
            RecklessStorage::Embedded(parameters) => &parameters.feature_biases.0,
            RecklessStorage::Owned(parameters) => &parameters.feature_biases,
        }
    }

    #[inline(always)]
    fn l1_weights(&self) -> &[u8] {
        match &self.storage {
            #[cfg(feature = "embedded-networks")]
            RecklessStorage::Embedded(parameters) => flatten(&parameters.l1_weights.0),
            RecklessStorage::Owned(parameters) => &parameters.l1_weights,
        }
    }

    #[inline(always)]
    fn l1_biases(&self) -> &[f32] {
        match &self.storage {
            #[cfg(feature = "embedded-networks")]
            RecklessStorage::Embedded(parameters) => flatten(&parameters.l1_biases.0),
            RecklessStorage::Owned(parameters) => &parameters.l1_biases,
        }
    }

    #[inline(always)]
    fn l2_weights(&self) -> &[f32] {
        match &self.storage {
            #[cfg(feature = "embedded-networks")]
            RecklessStorage::Embedded(parameters) => flatten_three(&parameters.l2_weights.0),
            RecklessStorage::Owned(parameters) => &parameters.l2_weights,
        }
    }

    #[inline(always)]
    fn l2_biases(&self) -> &[f32] {
        match &self.storage {
            #[cfg(feature = "embedded-networks")]
            RecklessStorage::Embedded(parameters) => flatten(&parameters.l2_biases.0),
            RecklessStorage::Owned(parameters) => &parameters.l2_biases,
        }
    }

    #[inline(always)]
    fn l3_weights(&self) -> &[f32] {
        match &self.storage {
            #[cfg(feature = "embedded-networks")]
            RecklessStorage::Embedded(parameters) => flatten(&parameters.l3_weights.0),
            RecklessStorage::Owned(parameters) => &parameters.l3_weights,
        }
    }

    #[inline(always)]
    fn l3_biases(&self) -> &[f32] {
        match &self.storage {
            #[cfg(feature = "embedded-networks")]
            RecklessStorage::Embedded(parameters) => &parameters.l3_biases.0,
            RecklessStorage::Owned(parameters) => &parameters.l3_biases,
        }
    }
}

#[inline(always)]
#[cfg(feature = "embedded-networks")]
fn flatten<T, const ROWS: usize, const COLUMNS: usize>(values: &[[T; COLUMNS]; ROWS]) -> &[T] {
    // SAFETY: nested arrays are contiguous, contain exactly `ROWS * COLUMNS`
    // elements, and the returned slice cannot outlive the borrowed array.
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast(), ROWS * COLUMNS) }
}

#[inline(always)]
#[cfg(feature = "embedded-networks")]
fn flatten_three<T, const A: usize, const B: usize, const C: usize>(
    values: &[[[T; C]; B]; A],
) -> &[T] {
    // SAFETY: nested arrays are contiguous, contain exactly `A * B * C`
    // elements, and the returned slice cannot outlive the borrowed array.
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast(), A * B * C) }
}

pub fn load(path: &Path) -> Result<Box<RecklessNetwork>, String> {
    let size = std::fs::metadata(path)
        .map_err(|error| {
            format!(
                "failed to inspect Reckless NNUE '{}': {error}",
                path.display()
            )
        })?
        .len();
    if size != FILE_SIZE {
        return Err(format!(
            "incompatible Reckless NNUE size for '{}': expected {FILE_SIZE} bytes, found {size}",
            path.display()
        ));
    }

    let file = File::open(path)
        .map_err(|error| format!("failed to open Reckless NNUE '{}': {error}", path.display()))?;
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let network = OwnedRecklessParameters {
        threat_weights: read_bytes(&mut reader, THREAT_FEATURES * HIDDEN_SIZE)?,
        piece_weights: read_i16s(&mut reader, INPUT_BUCKETS * 768 * HIDDEN_SIZE)?,
        feature_biases: read_i16s(&mut reader, HIDDEN_SIZE)?,
        l1_weights: read_bytes(&mut reader, OUTPUT_BUCKETS * L2_SIZE * HIDDEN_SIZE)?,
        l1_biases: read_f32s(&mut reader, OUTPUT_BUCKETS * L2_SIZE)?,
        l2_weights: read_f32s(&mut reader, OUTPUT_BUCKETS * L2_SIZE * L3_SIZE)?,
        l2_biases: read_f32s(&mut reader, OUTPUT_BUCKETS * L3_SIZE)?,
        l3_weights: read_f32s(&mut reader, OUTPUT_BUCKETS * L3_SIZE)?,
        l3_biases: read_f32s(&mut reader, OUTPUT_BUCKETS)?,
    };

    let consumed = parameter_bytes();
    let mut padding = vec![0u8; FILE_SIZE as usize - consumed];
    reader
        .read_exact(&mut padding)
        .map_err(|error| format!("failed to read Reckless NNUE padding: {error}"))?;
    let mut extra = [0u8; 1];
    if reader
        .read(&mut extra)
        .map_err(|error| format!("failed to finish Reckless NNUE read: {error}"))?
        != 0
    {
        return Err("Reckless NNUE contains trailing data".to_string());
    }

    Ok(Box::new(RecklessNetwork {
        storage: RecklessStorage::Owned(network),
    }))
}

fn parameter_bytes() -> usize {
    THREAT_FEATURES * HIDDEN_SIZE
        + INPUT_BUCKETS * 768 * HIDDEN_SIZE * size_of::<i16>()
        + HIDDEN_SIZE * size_of::<i16>()
        + OUTPUT_BUCKETS * L2_SIZE * HIDDEN_SIZE
        + OUTPUT_BUCKETS * L2_SIZE * size_of::<f32>()
        + OUTPUT_BUCKETS * L2_SIZE * L3_SIZE * size_of::<f32>()
        + OUTPUT_BUCKETS * L3_SIZE * size_of::<f32>()
        + OUTPUT_BUCKETS * L3_SIZE * size_of::<f32>()
        + OUTPUT_BUCKETS * size_of::<f32>()
}

fn read_bytes(reader: &mut impl Read, count: usize) -> Result<Box<[u8]>, String> {
    let mut values = vec![0u8; count];
    reader
        .read_exact(&mut values)
        .map_err(|error| format!("failed to read Reckless NNUE bytes: {error}"))?;
    Ok(values.into_boxed_slice())
}

fn read_i16s(reader: &mut impl Read, count: usize) -> Result<Box<[i16]>, String> {
    const VALUES_PER_CHUNK: usize = 32 * 1024;
    let mut values = Vec::with_capacity(count);
    let mut bytes = vec![0u8; VALUES_PER_CHUNK * size_of::<i16>()];
    while values.len() < count {
        let chunk = (count - values.len()).min(VALUES_PER_CHUNK);
        reader
            .read_exact(&mut bytes[..chunk * size_of::<i16>()])
            .map_err(|error| format!("failed to read Reckless NNUE i16 values: {error}"))?;
        values.extend(
            bytes[..chunk * size_of::<i16>()]
                .chunks_exact(2)
                .map(|value| i16::from_le_bytes([value[0], value[1]])),
        );
    }
    Ok(values.into_boxed_slice())
}

fn read_f32s(reader: &mut impl Read, count: usize) -> Result<Box<[f32]>, String> {
    const VALUES_PER_CHUNK: usize = 8 * 1024;
    let mut values = Vec::with_capacity(count);
    let mut bytes = vec![0u8; VALUES_PER_CHUNK * size_of::<f32>()];
    while values.len() < count {
        let chunk = (count - values.len()).min(VALUES_PER_CHUNK);
        reader
            .read_exact(&mut bytes[..chunk * size_of::<f32>()])
            .map_err(|error| format!("failed to read Reckless NNUE f32 values: {error}"))?;
        values.extend(
            bytes[..chunk * size_of::<f32>()]
                .chunks_exact(4)
                .map(|value| f32::from_le_bytes([value[0], value[1], value[2], value[3]])),
        );
    }
    Ok(values.into_boxed_slice())
}

struct PieceCache {
    values: [i16; HIDDEN_SIZE],
    pieces: [u64; 12],
    initialized: bool,
}

impl PieceCache {
    fn new() -> Self {
        Self {
            values: [0; HIDDEN_SIZE],
            pieces: [0; 12],
            initialized: false,
        }
    }
}

#[derive(Clone)]
struct PieceFrame {
    values: [[i16; HIDDEN_SIZE]; 2],
    accurate: [bool; 2],
    hash: u64,
}

impl PieceFrame {
    fn new() -> Self {
        Self {
            values: [[0; HIDDEN_SIZE]; 2],
            accurate: [false; 2],
            hash: 0,
        }
    }
}

struct ThreatCache {
    values: [i16; HIDDEN_SIZE],
    features: Vec<usize>,
}

#[derive(Clone)]
struct ThreatFrame {
    values: [[i16; HIDDEN_SIZE]; 2],
    accurate: [bool; 2],
    mirrored: [bool; 2],
    /// Retained as explicit frame-layout padding; removing it degrades cache stride.
    hash: u64,
    deltas: [ThreatDelta; MAX_THREAT_DELTAS],
    delta_count: usize,
    overflowed: bool,
    pending_move: Option<Move>,
    pending_mover: u8,
    pending_captured: u8,
}

impl ThreatFrame {
    fn new() -> Self {
        Self {
            values: [[0; HIDDEN_SIZE]; 2],
            accurate: [false; 2],
            mirrored: [false; 2],
            hash: 0,
            deltas: [ThreatDelta::default(); MAX_THREAT_DELTAS],
            delta_count: 0,
            overflowed: false,
            pending_move: None,
            pending_mover: u8::MAX,
            pending_captured: u8::MAX,
        }
    }
}

impl ThreatDeltaSink for ThreatFrame {
    #[inline(always)]
    fn push_threat_delta(&mut self, delta: ThreatDelta) {
        if self.delta_count < MAX_THREAT_DELTAS {
            self.deltas[self.delta_count] = delta;
            self.delta_count += 1;
        } else {
            self.overflowed = true;
        }
    }
}

impl ThreatCache {
    fn new() -> Self {
        Self {
            values: [0; HIDDEN_SIZE],
            features: Vec::with_capacity(192),
        }
    }
}

pub(crate) struct RecklessAccumulatorState {
    piece_cache: Box<[PieceCache]>,
    piece_stack: Box<[PieceFrame]>,
    threat_cache: [ThreatCache; 4],
    feature_scratch: [Vec<usize>; 2],
    add_scratch: Vec<usize>,
    sub_scratch: Vec<usize>,
    threat_stack: Box<[ThreatFrame]>,
    stack_index: usize,
}

impl RecklessAccumulatorState {
    pub(crate) fn new() -> Self {
        Self {
            piece_cache: (0..PIECE_CACHE_ENTRIES)
                .map(|_| PieceCache::new())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            piece_stack: vec![PieceFrame::new(); THREAT_STACK_SIZE].into_boxed_slice(),
            threat_cache: std::array::from_fn(|_| ThreatCache::new()),
            feature_scratch: std::array::from_fn(|_| Vec::with_capacity(192)),
            add_scratch: Vec::with_capacity(192),
            sub_scratch: Vec::with_capacity(192),
            threat_stack: vec![ThreatFrame::new(); THREAT_STACK_SIZE].into_boxed_slice(),
            stack_index: 0,
        }
    }

    pub(crate) fn clear(&mut self) {
        for cache in &mut self.piece_cache {
            cache.initialized = false;
            cache.pieces = [0; 12];
        }
        for frame in &mut self.piece_stack {
            frame.accurate = [false; 2];
            frame.hash = 0;
        }
        for cache in &mut self.threat_cache {
            cache.values.fill(0);
            cache.features.clear();
        }
        self.stack_index = 0;
        for frame in &mut self.threat_stack {
            frame.accurate = [false; 2];
            frame.hash = 0;
            frame.delta_count = 0;
            frame.overflowed = false;
            frame.pending_move = None;
            frame.pending_mover = u8::MAX;
            frame.pending_captured = u8::MAX;
        }
    }

    pub(crate) fn push_move(&mut self, board: &Board, mv: Move) {
        self.push_move_observed(board, mv);
        collect_move_deltas(&mut self.threat_stack[self.stack_index], board, mv);
    }

    pub(crate) fn push_move_observed(&mut self, board: &Board, mv: Move) {
        if self.stack_index + 1 >= self.threat_stack.len() {
            return;
        }
        self.stack_index += 1;
        let frame = &mut self.threat_stack[self.stack_index];
        self.piece_stack[self.stack_index].accurate = [false; 2];
        frame.accurate = [false; 2];
        frame.hash = 0;
        frame.delta_count = 0;
        frame.overflowed = false;
        frame.pending_move = Some(mv);
        frame.pending_mover = board.piece_ids()[mv.from.index()];
        frame.pending_captured = board.piece_ids()[mv.to.index()];
    }

    pub(crate) fn push_null(&mut self) {
        if self.stack_index + 1 >= self.threat_stack.len() {
            return;
        }
        self.stack_index += 1;
        let frame = &mut self.threat_stack[self.stack_index];
        self.piece_stack[self.stack_index].accurate = [false; 2];
        frame.accurate = [false; 2];
        frame.hash = 0;
        frame.delta_count = 0;
        frame.overflowed = false;
        frame.pending_move = None;
        frame.pending_mover = u8::MAX;
        frame.pending_captured = u8::MAX;
    }

    pub(crate) fn pop(&mut self) {
        self.stack_index = self.stack_index.saturating_sub(1);
    }

    pub(crate) fn evaluate(&mut self, board: &Board, network: &RecklessNetwork) -> i32 {
        let king_squares = [
            board.king_square(Color::White).index(),
            board.king_square(Color::Black).index(),
        ];

        let mut piece_cache_indexes = [0usize; 2];
        let mut threat_cache_indexes = [0usize; 2];
        for (pov, &king_square) in king_squares.iter().enumerate() {
            let mirrored = king_square & 7 >= 4;
            let bucket = INPUT_BUCKET_LAYOUT[king_square ^ (56 * pov)];
            let cache_index =
                pov * 2 * INPUT_BUCKETS + usize::from(mirrored) * INPUT_BUCKETS + bucket;
            piece_cache_indexes[pov] = cache_index;
            threat_cache_indexes[pov] = pov * 2 + usize::from(mirrored);
        }
        self.update_stacked_pieces(board, network, king_squares, piece_cache_indexes);

        let use_stack = self.stack_index > 0;
        if use_stack {
            self.update_stacked_threats(board, network, king_squares);
        } else {
            Self::collect_threat_features(board, king_squares, &mut self.feature_scratch);
            for (pov, &cache_index) in threat_cache_indexes.iter().enumerate() {
                let current = &self.feature_scratch[pov];
                diff_sorted(
                    &self.threat_cache[cache_index].features,
                    current,
                    &mut self.add_scratch,
                    &mut self.sub_scratch,
                );
                apply_i8_rows(
                    &mut self.threat_cache[cache_index].values,
                    network.threat_weights(),
                    &self.add_scratch,
                    &self.sub_scratch,
                );
                self.threat_cache[cache_index].features.clear();
                self.threat_cache[cache_index]
                    .features
                    .extend_from_slice(current);
            }

            // Seed the search stack from the exact root caches. Child nodes can
            // then apply their move deltas instead of rebuilding all threats.
            let root = &mut self.threat_stack[0];
            for (pov, &cache_index) in threat_cache_indexes.iter().enumerate() {
                root.values[pov] = self.threat_cache[cache_index].values;
                root.accurate[pov] = true;
                root.mirrored[pov] = king_squares[pov] & 7 >= 4;
            }
            root.hash = board.hash;
        }

        let stm = board.side_to_move.index();
        let piece_values = [
            &self.piece_stack[self.stack_index].values[0],
            &self.piece_stack[self.stack_index].values[1],
        ];
        if use_stack {
            let frame = &self.threat_stack[self.stack_index];
            forward(
                network,
                piece_values,
                [&frame.values[0], &frame.values[1]],
                stm,
                board.all_occupancy().count_ones() as usize,
            )
        } else {
            forward(
                network,
                piece_values,
                [
                    &self.threat_cache[threat_cache_indexes[0]].values,
                    &self.threat_cache[threat_cache_indexes[1]].values,
                ],
                stm,
                board.all_occupancy().count_ones() as usize,
            )
        }
    }

    fn update_stacked_pieces(
        &mut self,
        board: &Board,
        network: &RecklessNetwork,
        king_squares: [usize; 2],
        cache_indexes: [usize; 2],
    ) {
        let index = self.stack_index;
        if self.piece_stack[index].hash != board.hash {
            self.piece_stack[index].accurate = [false; 2];
        }
        let sources = std::array::from_fn::<_, 2, _>(|pov| {
            piece_update_source(&self.piece_stack, &self.threat_stack, index, pov)
        });
        let needs_refresh = std::array::from_fn::<_, 2, _>(|pov| {
            !self.piece_stack[index].accurate[pov] && sources[pov].is_none()
        });
        let pieces = needs_refresh
            .iter()
            .any(|&refresh| refresh)
            .then(|| snapshot_pieces(board));

        for pov in 0..2 {
            if self.piece_stack[index].accurate[pov] {
                continue;
            }
            if let Some(source) = sources[pov] {
                for child in source + 1..=index {
                    let (parents, children) = self.piece_stack.split_at_mut(child);
                    let parent = &parents[child - 1];
                    let frame = &mut children[0];
                    apply_piece_delta_from(
                        &mut frame.values[pov],
                        &parent.values[pov],
                        network.piece_weights(),
                        &self.threat_stack[child],
                        king_squares[pov],
                        pov,
                    );
                    frame.accurate[pov] = true;
                }
            } else {
                let cache_index = cache_indexes[pov];
                update_piece_cache(
                    &mut self.piece_cache[cache_index],
                    network,
                    pieces
                        .as_ref()
                        .expect("a piece refresh has a position snapshot"),
                    king_squares[pov],
                    pov,
                );
                self.piece_stack[index].values[pov] = self.piece_cache[cache_index].values;
                self.piece_stack[index].accurate[pov] = true;
            }
        }
        self.piece_stack[index].hash = board.hash;
    }

    fn update_stacked_threats(
        &mut self,
        board: &Board,
        network: &RecklessNetwork,
        king_squares: [usize; 2],
    ) {
        let index = self.stack_index;
        let mirrored = [king_squares[0] & 7 >= 4, king_squares[1] & 7 >= 4];
        let sources = std::array::from_fn::<_, 2, _>(|pov| {
            threat_update_source(&self.threat_stack, index, pov)
        });

        let mut can_update = [false; 2];
        for pov in 0..2 {
            let Some(source) = sources[pov] else {
                continue;
            };
            can_update[pov] = true;
            for child in source + 1..=index {
                if self.threat_stack[child].overflowed {
                    can_update[pov] = false;
                    break;
                }
            }
        }

        let needs_refresh = std::array::from_fn::<_, 2, _>(|pov| {
            !self.threat_stack[index].accurate[pov] && !can_update[pov]
        });

        if needs_refresh.iter().any(|&refresh| refresh) {
            Self::collect_threat_features(board, king_squares, &mut self.feature_scratch);
        }

        for pov in 0..2 {
            if self.threat_stack[index].accurate[pov] {
                continue;
            }
            if needs_refresh[pov] {
                let frame = &mut self.threat_stack[index];
                frame.values[pov].fill(0);
                apply_i8_rows(
                    &mut frame.values[pov],
                    network.threat_weights(),
                    &self.feature_scratch[pov],
                    &[],
                );
            } else {
                let source = sources[pov].expect("an incremental threat path has a source");
                for child in source + 1..=index {
                    let (parents, children) = self.threat_stack.split_at_mut(child);
                    let parent = &parents[child - 1];
                    let frame = &mut children[0];
                    self.add_scratch.clear();
                    self.sub_scratch.clear();
                    for &delta in &frame.deltas[..frame.delta_count] {
                        let attacker = delta.attacker();
                        let attacked = delta.attacked();
                        let Some(feature) = threat_index(
                            attacker / 2,
                            attacker & 1,
                            delta.source(),
                            attacked / 2,
                            attacked & 1,
                            delta.target(),
                            mirrored[pov],
                            pov,
                        ) else {
                            continue;
                        };
                        if delta.add() {
                            self.add_scratch.push(feature);
                        } else {
                            self.sub_scratch.push(feature);
                        }
                    }
                    apply_i8_rows_from(
                        &mut frame.values[pov],
                        &parent.values[pov],
                        network.threat_weights(),
                        &self.add_scratch,
                        &self.sub_scratch,
                    );
                    frame.accurate[pov] = true;
                    frame.mirrored[pov] = mirrored[pov];
                }
            }
            let frame = &mut self.threat_stack[index];
            frame.accurate[pov] = true;
            frame.mirrored[pov] = mirrored[pov];
        }
        self.threat_stack[index].hash = board.hash;
    }

    fn collect_threat_features(
        board: &Board,
        king_squares: [usize; 2],
        feature_scratch: &mut [Vec<usize>; 2],
    ) {
        feature_scratch[0].clear();
        feature_scratch[1].clear();
        let occupancy = board.all_occupancy();
        let mut square_piece = [u8::MAX; 64];
        for color in 0..2 {
            for piece in 0..Piece::COUNT {
                let mut bb = board.pieces[color][piece];
                while bb != 0 {
                    let square = bb.trailing_zeros() as usize;
                    bb &= bb - 1;
                    square_piece[square] = (piece * 2 + color) as u8;
                }
            }
        }

        for color in 0..2 {
            for piece in 0..Piece::COUNT {
                let mut sources = board.pieces[color][piece];
                while sources != 0 {
                    let source = sources.trailing_zeros() as usize;
                    sources &= sources - 1;
                    let mut targets = piece_attacks(piece, color, source, occupancy) & occupancy;
                    while targets != 0 {
                        let target = targets.trailing_zeros() as usize;
                        targets &= targets - 1;
                        let attacked_id = square_piece[target] as usize;
                        debug_assert!(attacked_id < 12);
                        for (pov, &king_square) in king_squares.iter().enumerate() {
                            let mirrored = king_square & 7 >= 4;
                            if let Some(index) = threat_index(
                                piece,
                                color,
                                source,
                                attacked_id / 2,
                                attacked_id & 1,
                                target,
                                mirrored,
                                pov,
                            ) {
                                feature_scratch[pov].push(index);
                            }
                        }
                    }
                }
            }
        }

        feature_scratch[0].sort_unstable();
        feature_scratch[1].sort_unstable();
    }
}

impl BoardObserver for RecklessAccumulatorState {
    #[inline(always)]
    fn on_piece_change(
        &mut self,
        board: &Board,
        piece: Piece,
        color: Color,
        square: Square,
        add: bool,
    ) {
        let piece = (piece.index() * 2 + color.index()) as u8;
        push_threats_on_change(
            &mut self.threat_stack[self.stack_index],
            board,
            piece,
            square.index(),
            add,
        );
    }

    #[inline(always)]
    fn on_piece_move(
        &mut self,
        board: &Board,
        piece: Piece,
        color: Color,
        from: Square,
        to: Square,
    ) {
        let piece = (piece.index() * 2 + color.index()) as u8;
        push_threats_on_move(
            &mut self.threat_stack[self.stack_index],
            board,
            piece,
            from.index(),
            to.index(),
        );
    }

    #[inline(always)]
    fn on_piece_mutate(
        &mut self,
        board: &Board,
        old_piece: Piece,
        old_color: Color,
        new_piece: Piece,
        new_color: Color,
        square: Square,
    ) {
        let old_piece = (old_piece.index() * 2 + old_color.index()) as u8;
        let new_piece = (new_piece.index() * 2 + new_color.index()) as u8;
        push_threats_on_mutate(
            &mut self.threat_stack[self.stack_index],
            board,
            old_piece,
            new_piece,
            square.index(),
        );
    }
}

fn piece_update_source(
    piece_stack: &[PieceFrame],
    move_stack: &[ThreatFrame],
    index: usize,
    pov: usize,
) -> Option<usize> {
    if piece_stack[index].accurate[pov] {
        return Some(index);
    }
    for child in (1..=index).rev() {
        let move_frame = &move_stack[child];
        if let Some(mv) = move_frame.pending_move {
            let mover = move_frame.pending_mover;
            debug_assert_ne!(mover, u8::MAX);
            let mover_piece = usize::from(mover) / 2;
            let mover_color = usize::from(mover) & 1;
            if mover_piece == Piece::King.index() && mover_color == pov {
                let from = mv.from.index() ^ (56 * pov);
                let to = mv.to.index() ^ (56 * pov);
                let mirror_changed = (from & 7 >= 4) != (to & 7 >= 4);
                if mirror_changed || INPUT_BUCKET_LAYOUT[from] != INPUT_BUCKET_LAYOUT[to] {
                    return None;
                }
            }
        }
        if piece_stack[child - 1].accurate[pov] {
            return Some(child - 1);
        }
    }
    None
}

fn apply_piece_delta_from(
    dst: &mut [i16; HIDDEN_SIZE],
    src: &[i16; HIDDEN_SIZE],
    weights: &[i16],
    frame: &ThreatFrame,
    king_square: usize,
    pov: usize,
) {
    let Some(mv) = frame.pending_move else {
        *dst = *src;
        return;
    };
    let mover = frame.pending_mover;
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
        pov,
    );
    subs[0] = piece_feature_index(mover_color, mover_piece, mv.from.index(), king_square, pov);

    if mv.is_capture() && mv.flag != MoveFlag::EnPassant {
        let captured = frame.pending_captured;
        debug_assert_ne!(captured, u8::MAX);
        subs[sub_count] = piece_feature_index(
            usize::from(captured) & 1,
            usize::from(captured) / 2,
            mv.to.index(),
            king_square,
            pov,
        );
        sub_count += 1;
    } else if mv.flag == MoveFlag::EnPassant {
        let captured_square = Square::from_file_rank(mv.to.file(), mv.from.rank()).index();
        subs[sub_count] = piece_feature_index(
            mover_color ^ 1,
            Piece::Pawn.index(),
            captured_square,
            king_square,
            pov,
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
        adds[add_count] =
            piece_feature_index(mover_color, Piece::Rook.index(), rook_to, king_square, pov);
        subs[sub_count] = piece_feature_index(
            mover_color,
            Piece::Rook.index(),
            rook_from,
            king_square,
            pov,
        );
        add_count += 1;
        sub_count += 1;
    }

    apply_i16_rows_from(dst, src, weights, &adds[..add_count], &subs[..sub_count]);
}

fn snapshot_pieces(board: &Board) -> [u64; 12] {
    let mut pieces = [0u64; 12];
    for color in 0..2 {
        for piece in 0..Piece::COUNT {
            pieces[color * Piece::COUNT + piece] = board.pieces[color][piece];
        }
    }
    pieces
}

#[inline(always)]
fn threat_mirror_changes(frame: &ThreatFrame, pov: usize) -> bool {
    let Some(mv) = frame.pending_move else {
        return false;
    };
    let mover = frame.pending_mover;
    if mover == u8::MAX || usize::from(mover) & 1 != pov {
        return false;
    }
    usize::from(mover) / 2 == Piece::King.index() && (mv.from.file() >= 4) != (mv.to.file() >= 4)
}

fn threat_update_source(stack: &[ThreatFrame], index: usize, pov: usize) -> Option<usize> {
    if stack[index].accurate[pov] {
        return Some(index);
    }
    for child in (1..=index).rev() {
        if threat_mirror_changes(&stack[child], pov) {
            return None;
        }
        if stack[child - 1].accurate[pov] {
            return Some(child - 1);
        }
    }
    None
}

fn update_piece_cache(
    cache: &mut PieceCache,
    network: &RecklessNetwork,
    pieces: &[u64; 12],
    king_square: usize,
    pov: usize,
) {
    if !cache.initialized {
        cache.values.copy_from_slice(network.feature_biases());
        let mut adds = [0usize; 32];
        let mut add_count = 0;
        for color in 0..2 {
            for piece in 0..Piece::COUNT {
                let mut bb = pieces[color * Piece::COUNT + piece];
                while bb != 0 {
                    let square = bb.trailing_zeros() as usize;
                    bb &= bb - 1;
                    adds[add_count] = piece_feature_index(color, piece, square, king_square, pov);
                    add_count += 1;
                }
            }
        }
        apply_i16_rows(
            &mut cache.values,
            network.piece_weights(),
            &adds[..add_count],
            &[],
        );
        cache.pieces = *pieces;
        cache.initialized = true;
        return;
    }

    if cache.pieces == *pieces {
        return;
    }

    let mut adds = [0usize; 32];
    let mut subs = [0usize; 32];
    let mut add_count = 0;
    let mut sub_count = 0;
    for color in 0..2 {
        for piece in 0..Piece::COUNT {
            let index = color * Piece::COUNT + piece;
            let mut added = pieces[index] & !cache.pieces[index];
            while added != 0 {
                let square = added.trailing_zeros() as usize;
                added &= added - 1;
                adds[add_count] = piece_feature_index(color, piece, square, king_square, pov);
                add_count += 1;
            }
            let mut removed = cache.pieces[index] & !pieces[index];
            while removed != 0 {
                let square = removed.trailing_zeros() as usize;
                removed &= removed - 1;
                subs[sub_count] = piece_feature_index(color, piece, square, king_square, pov);
                sub_count += 1;
            }
        }
    }
    apply_i16_rows(
        &mut cache.values,
        network.piece_weights(),
        &adds[..add_count],
        &subs[..sub_count],
    );
    cache.pieces = *pieces;
}

#[inline(always)]
fn piece_feature_index(
    color: usize,
    piece: usize,
    square: usize,
    king_square: usize,
    pov: usize,
) -> usize {
    let flip = (7 * usize::from(king_square & 7 >= 4)) ^ (56 * pov);
    INPUT_BUCKET_LAYOUT[king_square ^ flip] * 768
        + 384 * usize::from(color != pov)
        + 64 * piece
        + (square ^ flip)
}

fn apply_i16_rows(
    accumulator: &mut [i16; HIDDEN_SIZE],
    weights: &[i16],
    adds: &[usize],
    subs: &[usize],
) {
    super::reckless_simd::apply_i16_rows(accumulator, weights, adds, subs);
}

fn apply_i16_rows_from(
    dst: &mut [i16; HIDDEN_SIZE],
    src: &[i16; HIDDEN_SIZE],
    weights: &[i16],
    adds: &[usize],
    subs: &[usize],
) {
    super::reckless_simd::apply_i16_rows_from(dst, src, weights, adds, subs);
}

fn apply_i8_rows(
    accumulator: &mut [i16; HIDDEN_SIZE],
    weights: &[u8],
    adds: &[usize],
    subs: &[usize],
) {
    super::reckless_simd::apply_i8_rows(accumulator, weights, adds, subs);
}

fn apply_i8_rows_from(
    dst: &mut [i16; HIDDEN_SIZE],
    src: &[i16; HIDDEN_SIZE],
    weights: &[u8],
    adds: &[usize],
    subs: &[usize],
) {
    super::reckless_simd::apply_i8_rows_from(dst, src, weights, adds, subs);
}

fn diff_sorted(old: &[usize], new: &[usize], adds: &mut Vec<usize>, subs: &mut Vec<usize>) {
    adds.clear();
    subs.clear();
    let (mut old_index, mut new_index) = (0usize, 0usize);
    while old_index < old.len() && new_index < new.len() {
        match old[old_index].cmp(&new[new_index]) {
            std::cmp::Ordering::Less => {
                subs.push(old[old_index]);
                old_index += 1;
            }
            std::cmp::Ordering::Greater => {
                adds.push(new[new_index]);
                new_index += 1;
            }
            std::cmp::Ordering::Equal => {
                old_index += 1;
                new_index += 1;
            }
        }
    }
    subs.extend_from_slice(&old[old_index..]);
    adds.extend_from_slice(&new[new_index..]);
}

fn forward(
    network: &RecklessNetwork,
    piece: [&[i16; HIDDEN_SIZE]; 2],
    threat: [&[i16; HIDDEN_SIZE]; 2],
    stm: usize,
    piece_count: usize,
) -> i32 {
    let bucket = OUTPUT_BUCKET_LAYOUT[piece_count.min(32)];
    let l1_base = bucket * L2_SIZE * HIDDEN_SIZE;
    let l2_base = bucket * L2_SIZE * L3_SIZE;
    let l3_base = bucket * L3_SIZE;
    let output = super::reckless_simd::forward(
        piece,
        threat,
        stm,
        super::reckless_simd::ForwardWeights {
            l1: &network.l1_weights()[l1_base..l1_base + L2_SIZE * HIDDEN_SIZE],
            l1_biases: &network.l1_biases()[bucket * L2_SIZE..(bucket + 1) * L2_SIZE],
            l2: &network.l2_weights()[l2_base..l2_base + L2_SIZE * L3_SIZE],
            l2_biases: &network.l2_biases()[l3_base..l3_base + L3_SIZE],
            l3: &network.l3_weights()[l3_base..l3_base + L3_SIZE],
            l3_bias: network.l3_biases()[bucket],
        },
    );
    (output * NETWORK_SCALE) as i32
}

#[inline(always)]
fn piece_attacks(piece: usize, color: usize, square: usize, occupancy: u64) -> u64 {
    match Piece::from_index(piece).expect("piece index is in range") {
        Piece::Pawn => pawn_attacks(color, square),
        Piece::Knight => knight_attacks(square),
        Piece::Bishop => bishop_attacks(square, occupancy),
        Piece::Rook => rook_attacks(square, occupancy),
        Piece::Queen => queen_attacks(square, occupancy),
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
            [0, 1, -1, 2, -1, -1],
            [0, 1, 2, 3, 4, -1],
            [0, 1, 2, 3, -1, -1],
            [0, 1, 2, 3, -1, -1],
            [0, 1, 2, 3, 4, -1],
            [0, 1, 2, 3, -1, -1],
        ];
        const TARGET_COUNTS: [i32; 6] = [6, 10, 8, 8, 10, 8];

        let mut square_offsets = [[0u32; 64]; 12];
        let mut empty_attacks = [[0u64; 64]; 12];
        let mut piece_feature_count = [0i32; 12];
        let mut piece_base = [0i32; 12];
        let mut offset = 0i32;

        for color in 0..2 {
            for (piece, &target_count) in TARGET_COUNTS.iter().enumerate() {
                let id = piece * 2 + color;
                let mut count = 0u32;
                for square in 0..64 {
                    square_offsets[id][square] = count;
                    let attacks = piece_attacks(piece, color, square, 0);
                    empty_attacks[id][square] = attacks;
                    if piece != Piece::Pawn.index() || (8..56).contains(&square) {
                        count += attacks.count_ones();
                    }
                }
                piece_feature_count[id] = count as i32;
                piece_base[id] = offset;
                offset += target_count * count as i32;
            }
        }
        debug_assert_eq!(offset as usize, THREAT_FEATURES);

        let mut pair_base = [[0i32; 12]; 12];
        let mut excluded = [[false; 12]; 12];
        let mut semi_excluded = [[false; 12]; 12];
        for attacker in 0..12 {
            for attacked in 0..12 {
                let attacker_piece = attacker / 2;
                let attacker_color = attacker & 1;
                let attacked_piece = attacked / 2;
                let attacked_color = attacked & 1;
                let interaction = INTERACTIONS[attacker_piece][attacked_piece];
                pair_base[attacker][attacked] = piece_base[attacker]
                    + (attacked_color as i32 * (TARGET_COUNTS[attacker_piece] / 2) + interaction)
                        * piece_feature_count[attacker];
                excluded[attacker][attacked] = interaction < 0;
                semi_excluded[attacker][attacked] = attacker_piece == attacked_piece
                    && (attacker_color != attacked_color || attacker_piece != Piece::Pawn.index());
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
fn threat_index(
    attacker_piece: usize,
    attacker_color: usize,
    mut source: usize,
    attacked_piece: usize,
    attacked_color: usize,
    mut target: usize,
    mirrored: bool,
    pov: usize,
) -> Option<usize> {
    let flip = (7 * usize::from(mirrored)) ^ (56 * pov);
    source ^= flip;
    target ^= flip;
    let attacker = attacker_piece * 2 + (attacker_color ^ pov);
    let attacked = attacked_piece * 2 + (attacked_color ^ pov);
    let tables = threat_tables();
    if tables.excluded[attacker][attacked]
        || (tables.semi_excluded[attacker][attacked] && source < target)
    {
        return None;
    }
    let below_target = if target == 0 { 0 } else { (1u64 << target) - 1 };
    let attack_rank = (tables.empty_attacks[attacker][source] & below_target).count_ones() as usize;
    let index = tables.pair_base[attacker][attacked] as usize
        + tables.square_offsets[attacker][source] as usize
        + attack_rank;
    debug_assert!(index < THREAT_FEATURES);
    Some(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_layout_size_matches_reckless_v09() {
        assert_eq!(parameter_bytes(), FILE_SIZE as usize - 32);
        assert_eq!(size_of::<EmbeddedRecklessParameters>(), FILE_SIZE as usize);
    }

    #[test]
    fn embedded_v60_matches_pinned_evaluations() {
        types::init();
        let mut state = RecklessAccumulatorState::new();
        let positions = [
            (Board::new(), 80),
            (
                Board::from_fen("rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2")
                    .unwrap(),
                94,
            ),
            (
                Board::from_fen("4k3/8/8/3q4/8/8/4Q3/4K3 w - - 0 1").unwrap(),
                17,
            ),
            (
                Board::from_fen(
                    "r1bqkbnr/1ppp1ppp/p1n5/1B2p3/4P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 0 4",
                )
                .unwrap(),
                44,
            ),
        ];
        for (board, expected) in positions {
            assert_eq!(state.evaluate(&board, embedded()), expected);
        }
    }

    #[test]
    fn embedded_v60_incremental_moves_match_fresh_evaluation() {
        types::init();
        let mut board = Board::new();
        let mut warm = RecklessAccumulatorState::new();
        for uci in ["e2e4", "e7e5", "g1f3", "b8c6", "f1b5", "a7a6"] {
            let mv = board
                .generate_legal_moves()
                .iter()
                .find(|mv| mv.to_uci() == uci)
                .copied()
                .unwrap();
            warm.push_move(&board, mv);
            board.make_move(mv);

            let incremental = warm.evaluate(&board, embedded());
            let fresh = RecklessAccumulatorState::new().evaluate(&board, embedded());
            assert_eq!(incremental, fresh, "cache mismatch after {uci}");
        }
    }

    #[test]
    fn threat_deltas_are_compactly_recorded_when_the_move_is_pushed() {
        types::init();
        let mut board = Board::new();
        let mut state = RecklessAccumulatorState::new();
        let _ = state.evaluate(&board, embedded());

        for uci in ["e2e4", "e7e5"] {
            let mv = board
                .generate_legal_moves()
                .iter()
                .find(|mv| mv.to_uci() == uci)
                .copied()
                .unwrap();
            state.push_move(&board, mv);
            let frame = &state.threat_stack[state.stack_index];
            assert_eq!(frame.pending_move, Some(mv));
            assert_ne!(frame.pending_mover, u8::MAX);
            board.make_move(mv);
            let _ = state.evaluate(&board, embedded());
        }
    }

    #[test]
    fn threat_updates_cross_an_unevaluated_parent() {
        types::init();
        let mut board = Board::new();
        let mut state = RecklessAccumulatorState::new();
        let _ = state.evaluate(&board, embedded());

        for (ply, uci) in ["e2e4", "e7e5", "g1f3"].into_iter().enumerate() {
            let mv = board
                .generate_legal_moves()
                .iter()
                .find(|mv| mv.to_uci() == uci)
                .copied()
                .unwrap();
            state.push_move(&board, mv);
            board.make_move(mv);

            if ply == 0 || ply == 2 {
                let incremental = state.evaluate(&board, embedded());
                let fresh = RecklessAccumulatorState::new().evaluate(&board, embedded());
                assert_eq!(incremental, fresh, "cache mismatch after {uci}");
            }
        }

        assert!(
            state.threat_stack[2]
                .accurate
                .into_iter()
                .all(|value| value)
        );
        assert!(
            state.threat_stack[3]
                .accurate
                .into_iter()
                .all(|value| value)
        );
    }

    #[test]
    fn piece_updates_cross_an_unevaluated_parent() {
        types::init();
        let mut board = Board::new();
        let mut state = RecklessAccumulatorState::new();
        let _ = state.evaluate(&board, embedded());

        for (ply, uci) in ["e2e4", "e7e5", "g1f3"].into_iter().enumerate() {
            let mv = board
                .generate_legal_moves()
                .iter()
                .find(|mv| mv.to_uci() == uci)
                .copied()
                .unwrap();
            state.push_move(&board, mv);
            board.make_move(mv);

            if ply == 2 {
                let incremental = state.evaluate(&board, embedded());
                let fresh = RecklessAccumulatorState::new().evaluate(&board, embedded());
                assert_eq!(incremental, fresh, "piece-stack mismatch after {uci}");
            }
        }

        assert!(
            state.piece_stack[1..=3]
                .iter()
                .all(|frame| frame.accurate.into_iter().all(|value| value))
        );
    }

    #[test]
    fn embedded_v60_incremental_handles_special_moves() {
        types::init();
        for (fen, uci) in [
            ("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1", "e5d6"),
            ("4k2r/8/8/8/8/8/8/4K2R w Kk - 0 1", "e1g1"),
            ("4k3/P7/8/8/8/8/8/4K3 w - - 0 1", "a7a8q"),
            ("1r2k3/P7/8/8/8/8/8/4K3 w - - 0 1", "a7b8q"),
        ] {
            let mut board = Board::from_fen(fen).unwrap();
            let mut state = RecklessAccumulatorState::new();
            let parent_score = state.evaluate(&board, embedded());
            let mv = board
                .generate_legal_moves()
                .iter()
                .find(|mv| mv.to_uci() == uci)
                .copied()
                .unwrap_or_else(|| panic!("missing {uci} in {fen}"));
            state.push_move(&board, mv);
            board.make_move(mv);
            let incremental = state.evaluate(&board, embedded());
            let refreshed = RecklessAccumulatorState::new().evaluate(&board, embedded());
            assert_eq!(incremental, refreshed, "incremental mismatch after {uci}");

            board.unmake_move(mv);
            state.pop();
            assert_eq!(state.evaluate(&board, embedded()), parent_score);
        }
    }

    #[test]
    fn observed_move_updates_match_fallback_and_fresh_evaluation() {
        types::init();
        for (fen, uci) in [
            (
                "rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 0 2",
                "f3e5",
            ),
            ("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1", "e5d6"),
            ("4k2r/8/8/8/8/8/8/4K2R w Kk - 0 1", "e1g1"),
            ("4k3/P7/8/8/8/8/8/4K3 w - - 0 1", "a7a8q"),
            ("1r2k3/P7/8/8/8/8/8/4K3 w - - 0 1", "a7b8q"),
        ] {
            let mut board = Board::from_fen(fen).unwrap();
            let mv = board
                .generate_legal_moves()
                .iter()
                .find(|mv| mv.to_uci() == uci)
                .copied()
                .unwrap_or_else(|| panic!("missing {uci} in {fen}"));

            let mut observed_board = board.clone();
            let mut observed = RecklessAccumulatorState::new();
            let _ = observed.evaluate(&observed_board, embedded());
            observed.push_move_observed(&observed_board, mv);
            observed_board.make_move_observed(mv, &mut observed);
            let observed_score = observed.evaluate(&observed_board, embedded());

            let mut fallback_board = board;
            let mut fallback = RecklessAccumulatorState::new();
            let _ = fallback.evaluate(&fallback_board, embedded());
            fallback.push_move(&fallback_board, mv);
            fallback_board.make_move(mv);
            let fallback_score = fallback.evaluate(&fallback_board, embedded());

            let fresh = RecklessAccumulatorState::new().evaluate(&observed_board, embedded());
            assert_eq!(
                observed_score, fallback_score,
                "observer mismatch after {uci}"
            );
            assert_eq!(observed_score, fresh, "fresh-state mismatch after {uci}");
        }
    }

    #[test]
    fn threat_layout_has_expected_feature_count() {
        types::init();
        let tables = threat_tables();
        assert_eq!(tables.pair_base[0][0], 0);
        assert!(tables.excluded[0][4]);
    }

    #[test]
    fn sorted_diff_preserves_duplicate_multiplicity() {
        let mut adds = Vec::new();
        let mut subs = Vec::new();
        diff_sorted(&[1, 2, 2, 4], &[2, 3, 4, 4], &mut adds, &mut subs);
        assert_eq!(adds, [3, 4]);
        assert_eq!(subs, [1, 2]);
    }

    #[test]
    fn rejects_incorrect_size_without_allocating_network() {
        let path = std::env::temp_dir().join("mujrim-invalid-reckless.nnue");
        std::fs::write(&path, b"too short").unwrap();
        let error = load(&path).err().unwrap();
        let _ = std::fs::remove_file(path);
        assert!(error.contains("expected"));
    }

    #[test]
    fn external_network_matches_pinned_evaluations_when_available() {
        let Ok(path) = std::env::var("MUJRIM_RECKLESS_TEST_NET") else {
            return;
        };
        types::init();
        let network = load(Path::new(&path)).unwrap();
        let mut state = RecklessAccumulatorState::new();
        let positions = [
            (Board::new(), 84),
            (
                Board::from_fen("rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2")
                    .unwrap(),
                83,
            ),
            (
                Board::from_fen("4k3/8/8/3q4/8/8/4Q3/4K3 w - - 0 1").unwrap(),
                6,
            ),
        ];
        for (board, expected) in positions {
            assert_eq!(state.evaluate(&board, &network), expected);
        }
    }

    #[test]
    fn external_network_incremental_cache_matches_fresh_state_when_available() {
        let Ok(path) = std::env::var("MUJRIM_RECKLESS_TEST_NET") else {
            return;
        };
        types::init();
        let network = load(Path::new(&path)).unwrap();
        let mut board = Board::new();
        let mut warm = RecklessAccumulatorState::new();
        let mut played = Vec::new();

        for uci in [
            "e2e4", "e7e5", "g1f3", "b8c6", "f1b5", "a7a6", "b5c6", "d7c6",
        ] {
            let mv = board
                .generate_legal_moves()
                .iter()
                .find(|mv| mv.to_uci() == uci)
                .copied()
                .unwrap();
            warm.push_move(&board, mv);
            board.make_move(mv);
            played.push(mv);

            let cached = warm.evaluate(&board, &network);
            let fresh = RecklessAccumulatorState::new().evaluate(&board, &network);
            assert_eq!(cached, fresh, "cache mismatch after {uci}");
        }

        while let Some(mv) = played.pop() {
            board.unmake_move(mv);
            warm.pop();
            let cached = warm.evaluate(&board, &network);
            let fresh = RecklessAccumulatorState::new().evaluate(&board, &network);
            assert_eq!(cached, fresh, "cache mismatch after unmaking {mv}");
        }
    }

    #[test]
    fn external_network_move_observer_handles_special_moves_when_available() {
        let Ok(path) = std::env::var("MUJRIM_RECKLESS_TEST_NET") else {
            return;
        };
        types::init();
        let network = load(Path::new(&path)).unwrap();
        for (fen, uci) in [
            ("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1", "e5d6"),
            ("4k2r/8/8/8/8/8/8/4K2R w Kk - 0 1", "e1g1"),
            ("4k3/P7/8/8/8/8/8/4K3 w - - 0 1", "a7a8q"),
            ("1r2k3/P7/8/8/8/8/8/4K3 w - - 0 1", "a7b8q"),
        ] {
            let mut board = Board::from_fen(fen).unwrap();
            let mut state = RecklessAccumulatorState::new();
            let parent_score = state.evaluate(&board, &network);
            let mv = board
                .generate_legal_moves()
                .iter()
                .find(|mv| mv.to_uci() == uci)
                .copied()
                .unwrap_or_else(|| panic!("missing {uci} in {fen}"));
            state.push_move(&board, mv);
            board.make_move(mv);
            let incremental = state.evaluate(&board, &network);
            let refreshed = RecklessAccumulatorState::new().evaluate(&board, &network);
            assert_eq!(incremental, refreshed, "incremental mismatch after {uci}");

            board.unmake_move(mv);
            state.pop();
            assert_eq!(state.evaluate(&board, &network), parent_score);
        }
    }
}
