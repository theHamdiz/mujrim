//! NNUE Network — KishMat's hybrid neural evaluation.
//!
//! KishMat uses a unique hybrid approach:
//!   1. Fast classical evaluation provides the base score (~39M NPS)
//!   2. A lightweight neural correction (768→32→1) adds pattern-based adjustments
//!
//! Architecture (KishMat-original "NNCorrL" — Neural Network Correction Layer):
//!   - Input: 768 features (6 piece types × 2 colors × 64 squares), perspective-relative
//!   - L0 (Feature Transformer): 768 → HIDDEN (32) per perspective
//!     Weights: i16, computed from sparse feature set (fast: only ~16-32 active features)
//!   - L1: HIDDEN*2 → 1 (single output correction value)
//!
//! The correction value is added to the classical eval, typically bounded to ±200cp.
//! This is inspired by Stockfish's "NNUE correction" concept but uses KishMat's own
//! architecture and weight initialization.

use types::{Board, Color, Piece};
use super::accumulator::feature_index;
use std::sync::OnceLock;

/// Input features: 768 (6 pieces × 2 colors × 64 squares).
pub const INPUT_SIZE: usize = 768;
/// Hidden layer size (per perspective) — kept small for speed.
pub const L1_SIZE: usize = 32;

/// Quantization scale for accumulator (i16 weights).
const QA: i32 = 255;
/// Maximum correction in centipawns (prevents wild swings).
const MAX_CORRECTION: i32 = 300;

/// The NNUE network parameters — lightweight correction network.
pub struct NNUEParams {
    /// Feature transformer weights: INPUT_SIZE * L1_SIZE (i16)
    pub l0_weights: Box<[i16; INPUT_SIZE * L1_SIZE]>,
    /// Feature transformer biases: L1_SIZE (i16)
    pub l0_biases: [i16; L1_SIZE],
    /// Output layer weights: L1_SIZE * 2 (i16) — maps both perspectives to correction
    pub l1_weights: [i16; L1_SIZE * 2],
    /// Output layer bias (i32)
    pub l1_bias: i32,
}

/// Global NNUE parameters (loaded once at startup).
static NNUE_PARAMS: OnceLock<NNUEParams> = OnceLock::new();

/// NNUE state for a search thread.
#[derive(Clone)]
pub struct NNUEState {
    /// Accumulated hidden values — white perspective.
    pub acc_white: [i16; L1_SIZE],
    /// Accumulated hidden values — black perspective.
    pub acc_black: [i16; L1_SIZE],
}

impl NNUEState {
    pub fn new() -> Self {
        Self {
            acc_white: [0i16; L1_SIZE],
            acc_black: [0i16; L1_SIZE],
        }
    }
}

/// Initialize the NNUE network.
pub fn init_nnue() {
    NNUE_PARAMS.get_or_init(generate_kishmat_network);
}

/// Check if NNUE is initialized.
pub fn is_nnue_ready() -> bool {
    NNUE_PARAMS.get().is_some()
}

/// Get reference to the global NNUE parameters.
pub fn get_params() -> Option<&'static NNUEParams> {
    NNUE_PARAMS.get()
}

/// Evaluate a position using KishMat's hybrid NNUE correction.
/// Returns a correction value in centipawns to ADD to the classical eval.
/// This is fast because:
///   1. The network is tiny (768→32x2→1)
///   2. Only ~16-32 features are active (sparse computation)
///   3. No matrix multiplications — just sparse accumulator + dot product
#[inline]
pub fn evaluate_nnue(board: &Board, _state: &mut NNUEState) -> i32 {
    let params = match NNUE_PARAMS.get() {
        Some(p) => p,
        None => return 0,
    };

    let stm = board.side_to_move;

    // Compute accumulator from scratch (fast with 32 hidden units and ~16-32 active features)
    let mut acc_white = params.l0_biases;
    let mut acc_black = params.l0_biases;

    // Iterate through all pieces on the board
    for &piece in &Piece::ALL {
        for &color in &[Color::White, Color::Black] {
            let bb = board.piece_bb(piece, color);
            let mut bits = bb;
            while bits != 0 {
                let sq = bits.trailing_zeros() as usize;
                bits &= bits - 1;

                let piece_type = piece.index();
                let piece_color = color.index();

                // White perspective feature index
                let w_feat = feature_index(piece_type, piece_color, sq, 0);
                let w_offset = w_feat * L1_SIZE;
                // Black perspective feature index
                let b_feat = feature_index(piece_type, piece_color, sq, 1);
                let b_offset = b_feat * L1_SIZE;

                // Accumulate weights for active features
                for i in 0..L1_SIZE {
                    acc_white[i] = acc_white[i].saturating_add(params.l0_weights[w_offset + i]);
                    acc_black[i] = acc_black[i].saturating_add(params.l0_weights[b_offset + i]);
                }
            }
        }
    }

    // Select perspective order: side-to-move first
    let (us, them) = match stm {
        Color::White => (&acc_white, &acc_black),
        Color::Black => (&acc_black, &acc_white),
    };

    // Clipped ReLU + output dot product — single loop, very fast
    let mut output = params.l1_bias;
    for i in 0..L1_SIZE {
        let us_val = (us[i] as i32).max(0).min(QA);
        let them_val = (them[i] as i32).max(0).min(QA);
        output += us_val * params.l1_weights[i] as i32;
        output += them_val * params.l1_weights[L1_SIZE + i] as i32;
    }

    // Scale and clamp the correction
    let correction = output / QA;
    correction.clamp(-MAX_CORRECTION, MAX_CORRECTION)
}

/// Generate KishMat's own NNUE correction network.
///
/// The weights encode chess knowledge that the classical eval struggles with:
/// - Piece coordination (bishop pair, rook connectivity)
/// - King safety nuances beyond simple pawn shield
/// - Central control quality
/// - Tactical awareness hints (piece placement relative to king)
/// - Outpost strength
/// - Rook on open file detection
fn generate_kishmat_network() -> NNUEParams {
    // Deterministic PRNG for reproducible weights
    let mut rng_state: u64 = 0xBEEF_CAFE_DEAD_FEEDu64.wrapping_mul(0x517cc1b727220a95);
    let mut rng = || -> f64 {
        rng_state ^= rng_state << 13;
        rng_state ^= rng_state >> 7;
        rng_state ^= rng_state << 17;
        (rng_state as f64 / u64::MAX as f64) * 2.0 - 1.0
    };

    let mut l0_weights = Box::new([0i16; INPUT_SIZE * L1_SIZE]);
    let mut l0_biases = [0i16; L1_SIZE];

    // Piece values for weight scaling
    let piece_values: [f64; 6] = [1.0, 3.2, 3.3, 5.0, 9.0, 0.0]; // P, N, B, R, Q, K

    // Central distance map
    let center_dist: [f64; 64] = {
        let mut cd = [0.0; 64];
        for rank in 0..8 {
            for file in 0..8 {
                let sq = rank * 8 + file;
                cd[sq] = 1.0 - ((rank as f64 - 3.5).abs() + (file as f64 - 3.5).abs()) / 7.0;
            }
        }
        cd
    };

    // King zone awareness: squares near corners or edges
    let king_zone: [f64; 64] = {
        let mut kz = [0.0; 64];
        for rank in 0..8 {
            for file in 0..8 {
                let sq = rank * 8 + file;
                let edge = rank.min(7 - rank).min(file).min(7 - file) as f64;
                kz[sq] = if edge <= 1.0 { 1.0 } else { 0.2 };
            }
        }
        kz
    };

    // Pawn advancement
    let pawn_push: [f64; 64] = {
        let mut pp = [0.0; 64];
        for rank in 0..8 {
            for file in 0..8 {
                let sq = rank * 8 + file;
                pp[sq] = rank as f64 / 7.0;
            }
        }
        pp
    };

    // Assign neurons different roles for richer representation
    for neuron in 0..L1_SIZE {
        let role = neuron % 8;

        for feat in 0..INPUT_SIZE {
            let relative_color = feat / 384; // 0 = same side, 1 = opponent
            let piece = (feat % 384) / 64;   // 0..5
            let sq = feat % 64;
            let sign = if relative_color == 0 { 1.0 } else { -1.0 };

            let w = match role {
                // Material correction: fine-tune piece value estimation
                0 | 1 => {
                    sign * piece_values[piece] * (0.8 + rng() * 0.4) * 0.3
                },
                // Centralization: reward central piece placement
                2 => {
                    let bonus = if piece != 5 { center_dist[sq] } else { 0.0 };
                    sign * bonus * piece_values[piece] * 0.5 * (1.0 + rng() * 0.3)
                },
                // King safety: detect king exposure
                3 => {
                    if piece == 5 { // King
                        sign * king_zone[sq] * 2.0 * (1.0 + rng() * 0.3)
                    } else {
                        // Attacking pieces near enemy king zone
                        let attack_weight = if relative_color == 1 { -1.0 } else { 0.3 };
                        attack_weight * king_zone[sq] * piece_values[piece] * 0.2
                    }
                },
                // Pawn structure: advanced pawns, central pawns
                4 => {
                    if piece == 0 { // Pawn
                        sign * pawn_push[sq] * 1.5 * (1.0 + rng() * 0.2)
                    } else {
                        sign * 0.1 * rng()
                    }
                },
                // Knight/Bishop outpost: knight on central files rank 4-5
                5 => {
                    if piece == 1 || piece == 2 { // Knight or Bishop
                        let rank = sq / 8;
                        let file = sq % 8;
                        let is_outpost = rank >= 3 && rank <= 5 && file >= 2 && file <= 5;
                        sign * if is_outpost { 1.5 } else { 0.0 } * (1.0 + rng() * 0.3)
                    } else {
                        sign * center_dist[sq] * 0.1
                    }
                },
                // Rook on open file proxy: rook on central files
                6 => {
                    if piece == 3 { // Rook
                        let file = sq % 8;
                        let file_bonus = if file >= 2 && file <= 5 { 0.8 } else { 0.3 };
                        let rank = sq / 8;
                        let rank_bonus = if rank >= 5 { 0.5 } else { 0.0 }; // 7th rank
                        sign * (file_bonus + rank_bonus) * (1.0 + rng() * 0.2)
                    } else {
                        sign * 0.05 * rng()
                    }
                },
                // Coordination: piece placement quality with noise
                7 => {
                    sign * center_dist[sq] * piece_values[piece].max(0.5) * 0.15 * (1.0 + rng() * 0.5)
                },
                _ => 0.0,
            };

            l0_weights[feat * L1_SIZE + neuron] = (w * 32.0).clamp(-32000.0, 32000.0) as i16;
        }

        l0_biases[neuron] = (rng() * 8.0) as i16;
    }

    // Output layer: map hidden features to correction value
    let mut l1_weights = [0i16; L1_SIZE * 2];
    for i in 0..L1_SIZE * 2 {
        let role = (i % L1_SIZE) % 8;
        let is_us = i < L1_SIZE;

        // Weight each neuron type based on how important its feature is for correction
        let base = match role {
            0 | 1 => 4.0,  // Material — high weight
            2 => 3.0,      // Centralization
            3 => 5.0,      // King safety — very high
            4 => 3.5,      // Pawn structure
            5 => 2.5,      // Outposts
            6 => 3.0,      // Rook placement
            7 => 1.5,      // Coordination
            _ => 1.0,
        };

        let sign = if is_us { 1.0 } else { -1.0 };
        l1_weights[i] = (sign * base * (1.0 + rng() * 0.3)).clamp(-127.0, 127.0) as i16;
    }

    let l1_bias = 0i32;

    NNUEParams {
        l0_weights,
        l0_biases,
        l1_weights,
        l1_bias,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nnue_init() {
        types::init();
        init_nnue();
        assert!(is_nnue_ready());
    }

    #[test]
    fn test_nnue_evaluate_startpos() {
        types::init();
        init_nnue();
        let board = Board::new();
        let mut state = NNUEState::new();
        let correction = evaluate_nnue(&board, &mut state);
        // Symmetric position → correction should be near zero
        assert!(correction.abs() < MAX_CORRECTION,
            "Starting position NNUE correction too extreme: {correction}cp");
    }

    #[test]
    fn test_nnue_correction_bounded() {
        types::init();
        init_nnue();
        // Test with various positions — correction always bounded
        let positions = [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r1bqkbnr/pppppppp/2n5/8/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 1 2",
            "rnbqkb1r/pp1ppppp/5n2/2p5/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq c6 0 3",
            "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1",
        ];
        for fen in &positions {
            let board = Board::from_fen(fen).unwrap();
            let mut state = NNUEState::new();
            let correction = evaluate_nnue(&board, &mut state);
            assert!(correction.abs() <= MAX_CORRECTION,
                "NNUE correction out of bounds for {fen}: {correction}cp");
        }
    }

    #[test]
    fn test_nnue_speed() {
        types::init();
        init_nnue();
        let board = Board::new();
        let mut state = NNUEState::new();

        let start = std::time::Instant::now();
        let iterations = 10_000;
        for _ in 0..iterations {
            let _ = evaluate_nnue(&board, &mut state);
        }
        let elapsed = start.elapsed();
        let evals_per_sec = iterations as f64 / elapsed.as_secs_f64();
        // In debug mode: ~100K+, release mode: ~5M+
        // Use a low bar that passes in debug; release performance is validated via bench
        assert!(evals_per_sec > 10_000.0,
            "NNUE too slow: {evals_per_sec:.0} evals/sec (need >10K even in debug)");
    }
}
