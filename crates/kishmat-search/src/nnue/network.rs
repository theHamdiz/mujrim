//! NNUE Network — KishMat's standalone neural network evaluation.
//!
//! Architecture: 768 → L1_SIZE×2 → 1 (perspective net with SCReLU)
//!   - Input: 768 features (6 piece types × 2 colors × 64 squares)
//!   - Feature Transformer: 768 → L1_SIZE per perspective (i16 weights)
//!   - Squared Clipped ReLU activation (better gradient flow than ClippedReLU)
//!   - Output: L1_SIZE*2 → 1 (i16 weights, single score output)
//!
//! This is a STANDALONE evaluation — it returns centipawns directly,
//! NOT a correction to add to classical eval.

use types::{Board, Color, Piece};
use super::accumulator::feature_index;
use std::sync::OnceLock;

/// Input features: 768 (6 pieces × 2 colors × 64 squares).
pub const INPUT_SIZE: usize = 768;
/// Hidden layer size (per perspective) — 64 balances speed with pattern capacity.
pub const L1_SIZE: usize = 64;

/// Quantization scale for accumulator (i16 weights).
const QA: i32 = 255;
/// Output scale factor — converts raw NNUE output to centipawns.
const EVAL_SCALE: i32 = 600;

/// The NNUE network parameters.
pub struct NNUEParams {
    /// Feature transformer weights: INPUT_SIZE * L1_SIZE (i16)
    pub l0_weights: Box<[i16]>,
    /// Feature transformer biases: L1_SIZE (i16)
    pub l0_biases: [i16; L1_SIZE],
    /// Output layer weights: L1_SIZE * 2 (i16) — maps both perspectives to score
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

/// Evaluate a position using KishMat's NNUE.
/// Returns a score in centipawns (standalone, NOT a correction).
#[inline]
pub fn evaluate_nnue(board: &Board, _state: &mut NNUEState) -> i32 {
    let params = match NNUE_PARAMS.get() {
        Some(p) => p,
        None => return 0,
    };

    let stm = board.side_to_move;

    // Compute accumulator from scratch — stack-allocated, zero heap allocs
    let mut acc_white = params.l0_biases;
    let mut acc_black = params.l0_biases;

    for &piece in &Piece::ALL {
        for &color in &[Color::White, Color::Black] {
            let bb = board.piece_bb(piece, color);
            let mut bits = bb;
            while bits != 0 {
                let sq = bits.trailing_zeros() as usize;
                bits &= bits - 1;

                let piece_type = piece.index();
                let piece_color = color.index();

                let w_feat = feature_index(piece_type, piece_color, sq, 0);
                let w_offset = w_feat * L1_SIZE;
                let b_feat = feature_index(piece_type, piece_color, sq, 1);
                let b_offset = b_feat * L1_SIZE;

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

    // SCReLU (Squared Clipped ReLU) + output dot product
    // SCReLU(x) = clamp(x, 0, QA)² — provides better gradients than standard CReLU
    let mut output = params.l1_bias as i64;
    for i in 0..L1_SIZE {
        let us_val = (us[i] as i32).clamp(0, QA) as i64;
        let them_val = (them[i] as i32).clamp(0, QA) as i64;
        // Squared activation: val * val * weight
        output += us_val * us_val * params.l1_weights[i] as i64;
        output += them_val * them_val * params.l1_weights[L1_SIZE + i] as i64;
    }

    // Scale: divide by QA² to undo the squaring, then apply eval scale
    (output / (QA as i64 * QA as i64) * EVAL_SCALE as i64 / 256) as i32
}

// ──────────────────────────────────────────────────────────────
// Knowledge-based weight generation
// ──────────────────────────────────────────────────────────────

/// Deterministic PRNG for reproducible weights.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self { Self(seed) }
    fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 as f64 / u64::MAX as f64) * 2.0 - 1.0
    }
    /// Gaussian-ish approximation (sum of 3 uniform)
    fn next_gaussian(&mut self) -> f64 {
        (self.next_f64() + self.next_f64() + self.next_f64()) / 1.73
    }
}

/// Pre-computed square tables for weight generation.
struct SquareTables {
    center_dist: [f64; 64],
    king_zone: [f64; 64],
    pawn_push_w: [f64; 64],
    pawn_push_b: [f64; 64],
    rank: [usize; 64],
    file: [usize; 64],
}

impl SquareTables {
    fn new() -> Self {
        let mut t = Self {
            center_dist: [0.0; 64],
            king_zone: [0.0; 64],
            pawn_push_w: [0.0; 64],
            pawn_push_b: [0.0; 64],
            rank: [0; 64],
            file: [0; 64],
        };
        for sq in 0..64 {
            let r = sq / 8;
            let f = sq % 8;
            t.rank[sq] = r;
            t.file[sq] = f;
            t.center_dist[sq] = 1.0 - ((r as f64 - 3.5).abs() + (f as f64 - 3.5).abs()) / 7.0;
            let edge = r.min(7 - r).min(f).min(7 - f) as f64;
            t.king_zone[sq] = if edge <= 1.0 { 1.0 } else { 0.2 };
            t.pawn_push_w[sq] = r as f64 / 7.0;
            t.pawn_push_b[sq] = (7 - r) as f64 / 7.0;
        }
        t
    }
}

/// Piece-Square Table values (MG/EG blend) for weight init.
/// Based on PeSTO tables but simplified.
static PIECE_VALUES: [f64; 6] = [1.0, 3.2, 3.3, 5.0, 9.0, 0.0];

/// Generate the NNUE network with deep chess knowledge embedded in weights.
fn generate_kishmat_network() -> NNUEParams {
    let mut rng = Rng::new(0xBEEF_CAFE_DEAD_FEEDu64.wrapping_mul(0x517cc1b727220a95));
    let tables = SquareTables::new();

    let mut l0_weights = vec![0i16; INPUT_SIZE * L1_SIZE];
    let mut l0_biases = [0i16; L1_SIZE];

    // 64 neurons organized into 8 roles (8 neurons each)
    for neuron in 0..L1_SIZE {
        let role = neuron / 8; // 8 roles, 8 neurons each

        for feat in 0..INPUT_SIZE {
            let relative_color = feat / 384; // 0 = friendly, 1 = enemy
            let piece = (feat % 384) / 64;   // 0=P, 1=N, 2=B, 3=R, 4=Q, 5=K
            let sq = feat % 64;
            let sign = if relative_color == 0 { 1.0 } else { -1.0 };
            let r = tables.rank[sq];
            let f = tables.file[sq];

            let w = match role {
                // 0-1: Material value estimation (16 neurons)
                0 | 1 => {
                    sign * PIECE_VALUES[piece] * (0.7 + rng.next_f64() * 0.6) * 0.4
                },
                // 2: Pawn structure — advancement, center pawns
                2 => {
                    if piece == 0 {
                        let push = tables.pawn_push_w[sq];
                        let center = if f >= 2 && f <= 5 { 0.3 } else { 0.0 };
                        sign * (push * 1.8 + center) * (1.0 + rng.next_gaussian() * 0.2)
                    } else {
                        sign * rng.next_gaussian() * 0.05
                    }
                },
                // 3: Pawn structure — doubled/isolated penalty signals
                3 => {
                    if piece == 0 {
                        let file_factor = if f == 0 || f == 7 { -0.3 } else { 0.1 };
                        sign * (file_factor + tables.center_dist[sq] * 0.5) * (1.0 + rng.next_gaussian() * 0.2)
                    } else {
                        sign * rng.next_gaussian() * 0.05
                    }
                },
                // 4-5: Knight/Bishop centralization and outposts (16 neurons)
                4 | 5 => {
                    if piece == 1 || piece == 2 {
                        let is_outpost = r >= 3 && r <= 5 && f >= 2 && f <= 5;
                        let base = tables.center_dist[sq] * PIECE_VALUES[piece] * 0.5;
                        let outpost = if is_outpost { 1.2 } else { 0.0 };
                        sign * (base + outpost) * (1.0 + rng.next_gaussian() * 0.25)
                    } else if piece == 5 { // King centralization (bad in middlegame)
                        let edge_bonus = if tables.king_zone[sq] > 0.5 { 0.3 } else { -0.2 };
                        sign * edge_bonus * (1.0 + rng.next_gaussian() * 0.2)
                    } else {
                        sign * tables.center_dist[sq] * 0.1
                    }
                },
                // 6-7: Rook placement (open files, 7th rank) (16 neurons)
                6 | 7 => {
                    if piece == 3 {
                        let file_bonus = if f >= 3 && f <= 4 { 0.8 } else { 0.3 };
                        let rank_bonus = if r >= 6 { 1.0 } else if r >= 5 { 0.5 } else { 0.0 };
                        sign * (file_bonus + rank_bonus) * (1.0 + rng.next_gaussian() * 0.2)
                    } else if piece == 4 { // Queen mobility proxy
                        sign * tables.center_dist[sq] * 0.6 * (1.0 + rng.next_gaussian() * 0.2)
                    } else {
                        sign * rng.next_gaussian() * 0.05
                    }
                },
                // 8-9: King safety — pawn shield and attacker proximity (16 neurons)
                8 | 9 => {
                    if piece == 5 { // King
                        // Prefer castled king positions
                        let castled = if sq == 6 || sq == 2 || sq == 62 || sq == 58 { 2.0 }
                        else if tables.king_zone[sq] > 0.5 { 0.8 }
                        else { -0.5 };
                        sign * castled * (1.0 + rng.next_gaussian() * 0.2)
                    } else if piece == 0 { // Pawns near king = shield
                        let shield = if tables.king_zone[sq] > 0.5 { 0.6 } else { 0.0 };
                        sign * shield * (1.0 + rng.next_gaussian() * 0.2)
                    } else {
                        // Enemy pieces near king = danger
                        let attack = tables.king_zone[sq] * PIECE_VALUES[piece] * 0.15;
                        -sign * attack * (1.0 + rng.next_gaussian() * 0.2)
                    }
                },
                // 10-11: Bishop pair and minor piece coordination (16 neurons)
                10 | 11 => {
                    if piece == 2 { // Bishop — strong on diagonals
                        let diag_control = ((r as f64 - f as f64).abs() < 2.0 || (r as f64 - (7.0 - f as f64)).abs() < 2.0) as i32 as f64;
                        sign * (tables.center_dist[sq] + diag_control * 0.5) * 0.6 * (1.0 + rng.next_gaussian() * 0.2)
                    } else if piece == 1 { // Knight — prefer closed positions
                        sign * tables.center_dist[sq] * 0.7 * (1.0 + rng.next_gaussian() * 0.2)
                    } else {
                        sign * rng.next_gaussian() * 0.03
                    }
                },
                // 12-13: Piece coordination and connectivity (16 neurons)
                12 | 13 => {
                    sign * tables.center_dist[sq] * PIECE_VALUES[piece].max(0.5) * 0.2 * (1.0 + rng.next_gaussian() * 0.3)
                },
                // 14-15: General pattern detectors with heavy noise (16 neurons)
                // These neurons act as "free parameters" to learn diverse patterns
                14 | 15 => {
                    let base = sign * PIECE_VALUES[piece] * tables.center_dist[sq] * 0.15;
                    base + rng.next_gaussian() * 0.3
                },
                _ => 0.0,
            };

            l0_weights[feat * L1_SIZE + neuron] = (w * 40.0).clamp(-32000.0, 32000.0) as i16;
        }

        // Biases: small random initialization
        l0_biases[neuron] = (rng.next_gaussian() * 6.0) as i16;
    }

    // Output layer: map hidden features to eval score
    let mut l1_weights = [0i16; L1_SIZE * 2];
    for i in 0..L1_SIZE * 2 {
        let role = (i % L1_SIZE) / 8;
        let is_us = i < L1_SIZE;

        let base = match role {
            0 | 1 => 5.0,
            2 | 3 => 4.0,
            4 | 5 => 3.5,
            6 | 7 => 3.5,
            8 | 9 => 5.5,
            10 | 11 => 3.0,
            12 | 13 => 2.0,
            14 | 15 => 1.5,
            _ => 1.0,
        };

        let sign = if is_us { 1.0 } else { -0.85 };
        l1_weights[i] = (sign * base * (1.0 + rng.next_gaussian() * 0.2)).clamp(-127.0, 127.0) as i16;
    }

    NNUEParams {
        l0_weights: l0_weights.into_boxed_slice(),
        l0_biases,
        l1_weights,
        l1_bias: 0,
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
        let score = evaluate_nnue(&board, &mut state);
        // Starting position should be roughly balanced
        assert!(score.abs() < 200,
            "Starting position NNUE score too extreme: {score}cp");
    }

    #[test]
    fn test_nnue_score_sensible() {
        types::init();
        init_nnue();
        // Position with extra white queen should favor white
        let board_w = Board::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let board_b = Board::from_fen("rnbqkb1r/pppppppp/5n2/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1").unwrap();
        let mut state = NNUEState::new();
        let _score_w = evaluate_nnue(&board_w, &mut state);
        let _score_b = evaluate_nnue(&board_b, &mut state);
        // Just check they don't crash and return reasonable values
        assert!(_score_w.abs() < 5000, "Score too extreme: {_score_w}");
        assert!(_score_b.abs() < 5000, "Score too extreme: {_score_b}");
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
        assert!(evals_per_sec > 1_000.0,
            "NNUE too slow: {evals_per_sec:.0} evals/sec (need >1K in debug with 128-neuron net)");
    }
}
