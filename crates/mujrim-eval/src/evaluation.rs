//! Advanced tapered evaluation combining middlegame and endgame scores.
//! Targeting Stockfish 11-class classical (HCE) strength for `UseNNUE=false`.
//!
//! Features:
//! - Material + PeSTO piece-square tables (combined lookup for speed)
//! - Tempo
//! - Mobility per piece type (safe squares only)
//! - King safety (attack zone, attacker weights, pawn shield/storm)
//! - Pawn structure (doubled, isolated, backward, connected, passed)
//! - Threats (pieces attacked by lower-value pieces, hanging pieces)
//! - Space evaluation
//! - Knight outposts, rook on 7th, rook on open file
//! - Bishop pair, trapped bishop
//! - Passed pawn king proximity
//! - Connectivity (defended pieces)

use crate::psqt;
use types::bitboard::{Bitboard, count_bits, iter_bits};
use types::board::attack_tables::*;
use types::{Board, Color, Piece, Square};

// ── Game phase ──────────────────────────────────────────────────────────────
const PHASE_VALUES: [i32; 6] = [0, 1, 1, 2, 4, 0];
const TOTAL_PHASE: i32 = 24;

// ── Mobility bonuses per piece [num_moves] ──────────────────────────────────
// Knight: max 8 moves
const KNIGHT_MOB_MG: [i32; 9] = [-62, -53, -12, -4, 3, 13, 22, 28, 33];
const KNIGHT_MOB_EG: [i32; 9] = [-81, -56, -31, -16, 5, 11, 17, 20, 25];
// Bishop: max 13 moves
const BISHOP_MOB_MG: [i32; 14] = [-48, -20, 16, 26, 38, 51, 55, 63, 63, 68, 81, 81, 91, 98];
const BISHOP_MOB_EG: [i32; 14] = [-59, -23, -3, 13, 24, 42, 54, 57, 65, 73, 78, 86, 88, 97];
// Rook: max 14 moves
const ROOK_MOB_MG: [i32; 15] = [-60, -20, 2, 3, 3, 11, 22, 31, 40, 40, 41, 48, 57, 57, 62];
const ROOK_MOB_EG: [i32; 15] = [
    -78, -17, 23, 39, 70, 99, 103, 121, 134, 139, 158, 164, 168, 169, 172,
];
// Queen: max 27 moves
const QUEEN_MOB_MG: [i32; 28] = [
    -30, -12, -8, -9, 20, 23, 23, 35, 38, 53, 64, 65, 65, 66, 67, 67, 72, 72, 77, 79, 93, 108, 108,
    108, 110, 114, 114, 116,
];
const QUEEN_MOB_EG: [i32; 28] = [
    -48, -30, -7, 19, 40, 55, 59, 75, 78, 96, 96, 100, 121, 127, 131, 133, 136, 141, 147, 150, 151,
    168, 168, 171, 182, 182, 192, 219,
];

// ── King safety ─────────────────────────────────────────────────────────────
/// Attacker weights indexed by piece type [N, B, R, Q] (skip pawn/king)
const ATTACKER_WEIGHTS: [i32; 4] = [50, 50, 75, 125]; // N, B, R, Q
/// Safety table: maps attacker weight sum → king danger score (quadratic)
const KING_DANGER_TABLE_SIZE: usize = 128;

/// Pawn shield bonuses (how many pawns are in front of king)
const PAWN_SHIELD_BONUS_MG: [i32; 4] = [-30, 0, 15, 25]; // 0,1,2,3 pawns
const PAWN_SHIELD_BONUS_EG: [i32; 4] = [-10, 0, 5, 8];

/// Pawn storm penalty (enemy pawns advanced toward our king)
const PAWN_STORM_PENALTY: [i32; 8] = [0, 0, 0, -10, -30, -60, -80, 0]; // by rank

// ── Pawn structure ──────────────────────────────────────────────────────────
const DOUBLED_PENALTY_MG: i32 = -11;
const DOUBLED_PENALTY_EG: i32 = -56;
const ISOLATED_PENALTY_MG: i32 = -5;
const ISOLATED_PENALTY_EG: i32 = -15;
const BACKWARD_PENALTY_MG: i32 = -9;
const BACKWARD_PENALTY_EG: i32 = -24;
const CONNECTED_BONUS_MG: [i32; 8] = [0, 7, 8, 12, 29, 48, 86, 0]; // by rank
const CONNECTED_BONUS_EG: [i32; 8] = [0, 7, 8, 12, 29, 48, 86, 0];

const PASSED_BONUS_MG: [i32; 8] = [0, 5, 10, 20, 40, 70, 120, 0];
const PASSED_BONUS_EG: [i32; 8] = [0, 10, 20, 40, 70, 120, 200, 0];
const CANDIDATE_PASSED_MG: [i32; 8] = [0, 2, 4, 8, 16, 28, 0, 0];
const CANDIDATE_PASSED_EG: [i32; 8] = [0, 4, 8, 16, 32, 48, 0, 0];
const PAWN_LEVER_MG: i32 = 14;
const PAWN_LEVER_EG: i32 = 10;
const PIN_PENALTY_MG: [i32; 6] = [8, 22, 22, 16, 12, 0];
const PIN_PENALTY_EG: [i32; 6] = [4, 14, 14, 18, 16, 0];

// ── Threats ─────────────────────────────────────────────────────────────────
const THREAT_BY_PAWN_MG: [i32; 6] = [0, 80, 80, 120, 200, 0]; // P attacks [P,N,B,R,Q,K]
const THREAT_BY_PAWN_EG: [i32; 6] = [0, 40, 40, 60, 100, 0];
const THREAT_BY_MINOR_MG: [i32; 6] = [0, 0, 0, 50, 80, 0]; // minor attacks [P,N,B,R,Q,K]
const THREAT_BY_MINOR_EG: [i32; 6] = [0, 0, 0, 30, 50, 0];
const HANGING_PENALTY_MG: i32 = 48;
const HANGING_PENALTY_EG: i32 = 27;

// ── Piece bonuses ───────────────────────────────────────────────────────────
const BISHOP_PAIR_MG: i32 = 30;
const BISHOP_PAIR_EG: i32 = 52;
const ROOK_ON_SEVENTH_MG: i32 = 2;
const ROOK_ON_SEVENTH_EG: i32 = 28;
const ROOK_ON_OPEN_FILE_MG: i32 = 47;
const ROOK_ON_OPEN_FILE_EG: i32 = 25;
const ROOK_ON_SEMI_OPEN_MG: i32 = 19;
const ROOK_ON_SEMI_OPEN_EG: i32 = 13;
const KNIGHT_OUTPOST_MG: i32 = 54;
const KNIGHT_OUTPOST_EG: i32 = 34;
const BISHOP_OUTPOST_MG: i32 = 31;
const BISHOP_OUTPOST_EG: i32 = 25;
const CONNECTED_ROOKS_MG: i32 = 10;
const CONNECTED_ROOKS_EG: i32 = 5;

// ── Space ───────────────────────────────────────────────────────────────────
const SPACE_BONUS: i32 = 4;

// ── Tempo (Stockfish 11-era classical side-to-move bonus) ────────────────────
const TEMPO_MG: i32 = 28;
const TEMPO_EG: i32 = 24;

// ── File & rank masks ───────────────────────────────────────────────────────
const FILE_MASKS: [Bitboard; 8] = [
    0x0101010101010101,
    0x0202020202020202,
    0x0404040404040404,
    0x0808080808080808,
    0x1010101010101010,
    0x2020202020202020,
    0x4040404040404040,
    0x8080808080808080,
];

const ADJACENT_FILES: [Bitboard; 8] = [
    0x0202020202020202,
    0x0101010101010101 | 0x0404040404040404,
    0x0202020202020202 | 0x0808080808080808,
    0x0404040404040404 | 0x1010101010101010,
    0x0808080808080808 | 0x2020202020202020,
    0x1010101010101010 | 0x4040404040404040,
    0x2020202020202020 | 0x8080808080808080,
    0x4040404040404040,
];

const RANK_MASKS: [Bitboard; 8] = [
    0x00000000000000FF,
    0x000000000000FF00,
    0x0000000000FF0000,
    0x00000000FF000000,
    0x000000FF00000000,
    0x0000FF0000000000,
    0x00FF000000000000,
    0xFF00000000000000,
];

const CENTER_FILES: Bitboard =
    0x0404040404040404 | 0x0808080808080808 | 0x1010101010101010 | 0x2020202020202020; // C-F files

// ── King zone: 3x3 area around king + two squares forward ───────────────────
#[inline(always)]
fn king_zone(sq: usize) -> Bitboard {
    let ka = king_attacks(sq);
    let forward = match sq / 8 {
        0..=5 => ka << 8, // one rank forward
        _ => 0,
    };
    ka | (1u64 << sq) | forward
}

/// Evaluates from the side to move's perspective.
#[inline(always)]
pub fn evaluate(board: &Board) -> i32 {
    let (mg, eg, phase) = evaluate_full(board);
    let phase = phase.clamp(0, TOTAL_PHASE);
    let score = (mg * phase + eg * (TOTAL_PHASE - phase)) / TOTAL_PHASE;
    if board.side_to_move == Color::White {
        score
    } else {
        -score
    }
}

/// Full evaluation returning (mg, eg, phase) from White's perspective.
#[inline]
fn evaluate_full(board: &Board) -> (i32, i32, i32) {
    let mut mg = 0i32;
    let mut eg = 0i32;
    let mut phase = 0i32;

    let occ = board.all_occupancy();
    let w_occ = board.color_occupancy(Color::White);
    let b_occ = board.color_occupancy(Color::Black);
    let w_pawns = board.piece_bb(Piece::Pawn, Color::White);
    let b_pawns = board.piece_bb(Piece::Pawn, Color::Black);
    let all_pawns = w_pawns | b_pawns;

    // Pawn attacks for safe mobility calculation
    let w_pawn_attacks = pawn_attacks_bb(w_pawns, Color::White);
    let b_pawn_attacks = pawn_attacks_bb(b_pawns, Color::Black);

    // King squares
    let wk_sq = board.king_square(Color::White).index();
    let bk_sq = board.king_square(Color::Black).index();
    let wk_zone = king_zone(wk_sq);
    let bk_zone = king_zone(bk_sq);

    // Attack accumulators for king safety
    let mut w_attackers_count = 0i32;
    let mut w_attacker_weight = 0i32;
    let mut b_attackers_count = 0i32;
    let mut b_attacker_weight = 0i32;

    // ── Material + PSQT (combined tables keep the hot path cache-friendly) ─
    for &color in &[Color::White, Color::Black] {
        let sign = if color == Color::White { 1 } else { -1 };
        for &piece in &Piece::ALL {
            let bb = board.piece_bb(piece, color);
            phase += PHASE_VALUES[piece.index()] * bb.count_ones() as i32;

            for sq_idx in iter_bits(bb) {
                let idx = if color == Color::White {
                    sq_idx
                } else {
                    sq_idx ^ 56
                };
                let (piece_mg, piece_eg) = psqt::combined_value(piece, idx);
                mg += sign * piece_mg;
                eg += sign * piece_eg;
            }
        }
    }

    // Tempo for the side to move (applied from White's perspective below).
    match board.side_to_move {
        Color::White => {
            mg += TEMPO_MG;
            eg += TEMPO_EG;
        }
        Color::Black => {
            mg -= TEMPO_MG;
            eg -= TEMPO_EG;
        }
    }

    // ── Mobility + Attack accumulation ───────────────────────────────────
    // Knights
    for sq_idx in iter_bits(board.piece_bb(Piece::Knight, Color::White)) {
        let atk = knight_attacks(sq_idx) & !w_occ & !b_pawn_attacks;
        let mob = count_bits(atk) as usize;
        mg += KNIGHT_MOB_MG[mob.min(8)];
        eg += KNIGHT_MOB_EG[mob.min(8)];
        if atk & bk_zone != 0 {
            w_attackers_count += 1;
            w_attacker_weight += ATTACKER_WEIGHTS[0];
        }
    }
    for sq_idx in iter_bits(board.piece_bb(Piece::Knight, Color::Black)) {
        let atk = knight_attacks(sq_idx) & !b_occ & !w_pawn_attacks;
        let mob = count_bits(atk) as usize;
        mg -= KNIGHT_MOB_MG[mob.min(8)];
        eg -= KNIGHT_MOB_EG[mob.min(8)];
        if atk & wk_zone != 0 {
            b_attackers_count += 1;
            b_attacker_weight += ATTACKER_WEIGHTS[0];
        }
    }

    // Bishops
    for sq_idx in iter_bits(board.piece_bb(Piece::Bishop, Color::White)) {
        let atk = bishop_attacks(sq_idx, occ) & !w_occ & !b_pawn_attacks;
        let mob = count_bits(atk) as usize;
        mg += BISHOP_MOB_MG[mob.min(13)];
        eg += BISHOP_MOB_EG[mob.min(13)];
        if atk & bk_zone != 0 {
            w_attackers_count += 1;
            w_attacker_weight += ATTACKER_WEIGHTS[1];
        }
    }
    for sq_idx in iter_bits(board.piece_bb(Piece::Bishop, Color::Black)) {
        let atk = bishop_attacks(sq_idx, occ) & !b_occ & !w_pawn_attacks;
        let mob = count_bits(atk) as usize;
        mg -= BISHOP_MOB_MG[mob.min(13)];
        eg -= BISHOP_MOB_EG[mob.min(13)];
        if atk & wk_zone != 0 {
            b_attackers_count += 1;
            b_attacker_weight += ATTACKER_WEIGHTS[1];
        }
    }

    // Rooks
    for sq_idx in iter_bits(board.piece_bb(Piece::Rook, Color::White)) {
        let atk = rook_attacks(sq_idx, occ) & !w_occ & !b_pawn_attacks;
        let mob = count_bits(atk) as usize;
        mg += ROOK_MOB_MG[mob.min(14)];
        eg += ROOK_MOB_EG[mob.min(14)];
        if atk & bk_zone != 0 {
            w_attackers_count += 1;
            w_attacker_weight += ATTACKER_WEIGHTS[2];
        }

        // Rook on open/semi-open file
        let file = sq_idx % 8;
        if FILE_MASKS[file] & all_pawns == 0 {
            mg += ROOK_ON_OPEN_FILE_MG;
            eg += ROOK_ON_OPEN_FILE_EG;
        } else if FILE_MASKS[file] & w_pawns == 0 {
            mg += ROOK_ON_SEMI_OPEN_MG;
            eg += ROOK_ON_SEMI_OPEN_EG;
        }

        // Rook on 7th rank
        if sq_idx / 8 == 6 {
            mg += ROOK_ON_SEVENTH_MG;
            eg += ROOK_ON_SEVENTH_EG;
        }
    }
    for sq_idx in iter_bits(board.piece_bb(Piece::Rook, Color::Black)) {
        let atk = rook_attacks(sq_idx, occ) & !b_occ & !w_pawn_attacks;
        let mob = count_bits(atk) as usize;
        mg -= ROOK_MOB_MG[mob.min(14)];
        eg -= ROOK_MOB_EG[mob.min(14)];
        if atk & wk_zone != 0 {
            b_attackers_count += 1;
            b_attacker_weight += ATTACKER_WEIGHTS[2];
        }

        let file = sq_idx % 8;
        if FILE_MASKS[file] & all_pawns == 0 {
            mg -= ROOK_ON_OPEN_FILE_MG;
            eg -= ROOK_ON_OPEN_FILE_EG;
        } else if FILE_MASKS[file] & b_pawns == 0 {
            mg -= ROOK_ON_SEMI_OPEN_MG;
            eg -= ROOK_ON_SEMI_OPEN_EG;
        }

        if sq_idx / 8 == 1 {
            mg -= ROOK_ON_SEVENTH_MG;
            eg -= ROOK_ON_SEVENTH_EG;
        }
    }

    // Queens
    for sq_idx in iter_bits(board.piece_bb(Piece::Queen, Color::White)) {
        let atk = queen_attacks(sq_idx, occ) & !w_occ & !b_pawn_attacks;
        let mob = count_bits(atk) as usize;
        mg += QUEEN_MOB_MG[mob.min(27)];
        eg += QUEEN_MOB_EG[mob.min(27)];
        if atk & bk_zone != 0 {
            w_attackers_count += 1;
            w_attacker_weight += ATTACKER_WEIGHTS[3];
        }
    }
    for sq_idx in iter_bits(board.piece_bb(Piece::Queen, Color::Black)) {
        let atk = queen_attacks(sq_idx, occ) & !b_occ & !w_pawn_attacks;
        let mob = count_bits(atk) as usize;
        mg -= QUEEN_MOB_MG[mob.min(27)];
        eg -= QUEEN_MOB_EG[mob.min(27)];
        if atk & wk_zone != 0 {
            b_attackers_count += 1;
            b_attacker_weight += ATTACKER_WEIGHTS[3];
        }
    }

    // ── King safety ─────────────────────────────────────────────────────
    // White king safety (attacked by black)
    if b_attackers_count >= 2 {
        let danger = king_danger(b_attacker_weight);
        mg -= danger;
    }
    // Black king safety (attacked by white)
    if w_attackers_count >= 2 {
        let danger = king_danger(w_attacker_weight);
        mg += danger;
    }

    // Pawn shield
    {
        let (ws_mg, ws_eg) = eval_pawn_shield(board, Color::White, wk_sq);
        mg += ws_mg;
        eg += ws_eg;
        let (bs_mg, bs_eg) = eval_pawn_shield(board, Color::Black, bk_sq);
        mg -= bs_mg;
        eg -= bs_eg;
    }

    // Pawn storm
    {
        let (ws_mg, _) = eval_pawn_storm(b_pawns, Color::White, wk_sq);
        mg += ws_mg;
        let (bs_mg, _) = eval_pawn_storm(w_pawns, Color::Black, bk_sq);
        mg -= bs_mg;
    }

    // ── Pawn structure ──────────────────────────────────────────────────
    {
        let (w_mg, w_eg) = eval_pawn_structure(w_pawns, b_pawns, Color::White, bk_sq, wk_sq);
        mg += w_mg;
        eg += w_eg;
        let (b_mg, b_eg) = eval_pawn_structure(b_pawns, w_pawns, Color::Black, wk_sq, bk_sq);
        mg -= b_mg;
        eg -= b_eg;
    }

    // ── Bishop pair ─────────────────────────────────────────────────────
    if board.piece_count(Piece::Bishop, Color::White) >= 2 {
        mg += BISHOP_PAIR_MG;
        eg += BISHOP_PAIR_EG;
    }
    if board.piece_count(Piece::Bishop, Color::Black) >= 2 {
        mg -= BISHOP_PAIR_MG;
        eg -= BISHOP_PAIR_EG;
    }

    // ── Knight outposts ─────────────────────────────────────────────────
    {
        let (w_mg, w_eg) = eval_outposts(board, Color::White, b_pawns, b_pawn_attacks);
        mg += w_mg;
        eg += w_eg;
        let (b_mg, b_eg) = eval_outposts(board, Color::Black, w_pawns, w_pawn_attacks);
        mg -= b_mg;
        eg -= b_eg;
    }

    // ── Threats ──────────────────────────────────────────────────────────
    {
        let (w_mg, w_eg) = eval_threats(board, Color::White, w_pawn_attacks, b_pawn_attacks);
        mg += w_mg;
        eg += w_eg;
        let (b_mg, b_eg) = eval_threats(board, Color::Black, b_pawn_attacks, w_pawn_attacks);
        mg -= b_mg;
        eg -= b_eg;
    }

    // ── Pawn levers (our pawns that attack at least one enemy pawn) ──────
    {
        let w_levers = count_lever_pawns(w_pawns, b_pawns, Color::White);
        let b_levers = count_lever_pawns(b_pawns, w_pawns, Color::Black);
        mg += (w_levers - b_levers) * PAWN_LEVER_MG;
        eg += (w_levers - b_levers) * PAWN_LEVER_EG;
    }

    // ── Absolute pins toward each king ───────────────────────────────────
    {
        let (w_mg, w_eg) = eval_pins(board, Color::White, occ);
        mg -= w_mg;
        eg -= w_eg;
        let (b_mg, b_eg) = eval_pins(board, Color::Black, occ);
        mg += b_mg;
        eg += b_eg;
    }

    // ── Space ───────────────────────────────────────────────────────────
    if phase > 12 {
        // Only in middlegame with many pieces
        let w_space = eval_space(board, Color::White, w_pawns, w_pawn_attacks);
        let b_space = eval_space(board, Color::Black, b_pawns, b_pawn_attacks);
        mg += (w_space - b_space) * SPACE_BONUS;
    }

    // ── Connected rooks ─────────────────────────────────────────────────
    {
        let w_rooks = board.piece_bb(Piece::Rook, Color::White);
        if count_bits(w_rooks) >= 2 {
            // Check if rooks can see each other
            for sq in iter_bits(w_rooks) {
                let r_atk = rook_attacks(sq, occ);
                if r_atk & w_rooks & !((1u64) << sq) != 0 {
                    mg += CONNECTED_ROOKS_MG;
                    eg += CONNECTED_ROOKS_EG;
                    break;
                }
            }
        }
        let b_rooks = board.piece_bb(Piece::Rook, Color::Black);
        if count_bits(b_rooks) >= 2 {
            for sq in iter_bits(b_rooks) {
                let r_atk = rook_attacks(sq, occ);
                if r_atk & b_rooks & !((1u64) << sq) != 0 {
                    mg -= CONNECTED_ROOKS_MG;
                    eg -= CONNECTED_ROOKS_EG;
                    break;
                }
            }
        }
    }

    (mg, eg, phase)
}

// ── Helper: generate all pawn attacks for a color ────────────────────────────
#[inline]
fn count_lever_pawns(our_pawns: Bitboard, enemy_pawns: Bitboard, color: Color) -> i32 {
    let mut count = 0i32;
    for sq in iter_bits(our_pawns) {
        if pawn_attacks(color.index(), sq) & enemy_pawns != 0 {
            count += 1;
        }
    }
    count
}

#[inline(always)]
fn pawn_attacks_bb(pawns: Bitboard, color: Color) -> Bitboard {
    match color {
        Color::White => ((pawns & !FILE_MASKS[0]) << 7) | ((pawns & !FILE_MASKS[7]) << 9),
        Color::Black => ((pawns & !FILE_MASKS[7]) >> 7) | ((pawns & !FILE_MASKS[0]) >> 9),
    }
}

// ── King danger: quadratic scaling with a soft floor (SF11-style) ───────────
#[inline(always)]
fn king_danger(weight: i32) -> i32 {
    let w = weight.min(KING_DANGER_TABLE_SIZE as i32 - 1).max(0);
    // Quadratic danger plus a linear term so sparse attacks still matter.
    (w * w) / 64 + w / 4
}

// ── Pawn shield evaluation ──────────────────────────────────────────────────
#[inline]
fn eval_pawn_shield(board: &Board, color: Color, king_sq: usize) -> (i32, i32) {
    let king_file = king_sq % 8;
    let pawns = board.piece_bb(Piece::Pawn, color);

    let shield_rank = match color {
        Color::White => (king_sq / 8) + 1,
        Color::Black => (king_sq / 8).wrapping_sub(1),
    };

    if shield_rank > 7 {
        return (0, 0);
    }

    let mut count = 0u32;
    for df in -1i32..=1 {
        let f = king_file as i32 + df;
        if (0..8).contains(&f) {
            let sq = shield_rank * 8 + f as usize;
            if sq < 64 && pawns & (1u64 << sq) != 0 {
                count += 1;
            }
        }
    }

    let idx = count.min(3) as usize;
    (PAWN_SHIELD_BONUS_MG[idx], PAWN_SHIELD_BONUS_EG[idx])
}

// ── Pawn storm evaluation ───────────────────────────────────────────────────
#[inline]
fn eval_pawn_storm(enemy_pawns: Bitboard, our_color: Color, king_sq: usize) -> (i32, i32) {
    let king_file = king_sq % 8;
    let mut mg = 0i32;

    for df in -1i32..=1 {
        let f = king_file as i32 + df;
        if !(0..8).contains(&f) {
            continue;
        }
        let file_pawns = enemy_pawns & FILE_MASKS[f as usize];
        for sq in iter_bits(file_pawns) {
            let rank = sq / 8;
            let effective_rank = match our_color {
                Color::White => rank, // Enemy pawn on high rank = closer to our king
                Color::Black => 7 - rank,
            };
            if effective_rank < 8 {
                mg += PAWN_STORM_PENALTY[effective_rank];
            }
        }
    }

    (mg, 0)
}

// ── Pawn structure ──────────────────────────────────────────────────────────
#[inline]
fn eval_pawn_structure(
    our_pawns: Bitboard,
    enemy_pawns: Bitboard,
    color: Color,
    enemy_king_sq: usize,
    our_king_sq: usize,
) -> (i32, i32) {
    let mut mg = 0i32;
    let mut eg = 0i32;

    for file in 0..8 {
        let file_pawns = our_pawns & FILE_MASKS[file];
        let count = count_bits(file_pawns) as i32;

        // Doubled
        if count > 1 {
            mg += DOUBLED_PENALTY_MG * (count - 1);
            eg += DOUBLED_PENALTY_EG * (count - 1);
        }

        // Isolated
        if count > 0 && (our_pawns & ADJACENT_FILES[file]) == 0 {
            mg += ISOLATED_PENALTY_MG * count;
            eg += ISOLATED_PENALTY_EG * count;
        }
    }

    // Per-pawn evaluation
    for sq_idx in iter_bits(our_pawns) {
        let file = sq_idx % 8;
        let rank = sq_idx / 8;

        // Relative rank from our perspective
        let rel_rank = match color {
            Color::White => rank,
            Color::Black => 7 - rank,
        };

        // Connected pawns (has a friendly pawn on adjacent file, same or adjacent rank)
        let adjacent_pawns = our_pawns & ADJACENT_FILES[file];
        let same_rank = RANK_MASKS[rank];
        let adj_rank = if rank > 0 { RANK_MASKS[rank - 1] } else { 0 }
            | if rank < 7 { RANK_MASKS[rank + 1] } else { 0 };
        if adjacent_pawns & (same_rank | adj_rank) != 0 {
            mg += CONNECTED_BONUS_MG[rel_rank.min(7)];
            eg += CONNECTED_BONUS_EG[rel_rank.min(7)];
        }

        // Backward pawn: no friendly pawn on adjacent files behind it,
        // and the stop square is controlled by enemy pawn
        let behind_mask = match color {
            Color::White => ADJACENT_FILES[file] & ranks_below(rank),
            Color::Black => ADJACENT_FILES[file] & ranks_above(rank),
        };
        if our_pawns & behind_mask == 0 && adjacent_pawns == 0 {
            // Check if stop square is controlled by enemy pawn
            let stop_sq = match color {
                Color::White => {
                    if rank < 7 {
                        Some(sq_idx + 8)
                    } else {
                        None
                    }
                }
                Color::Black => {
                    if rank > 0 {
                        Some(sq_idx - 8)
                    } else {
                        None
                    }
                }
            };
            if let Some(stop) = stop_sq {
                let enemy_pawn_atk = pawn_attacks(color.index(), stop);
                if enemy_pawn_atk & enemy_pawns != 0 {
                    mg += BACKWARD_PENALTY_MG;
                    eg += BACKWARD_PENALTY_EG;
                }
            }
        }

        // Passed pawn
        let blocking_files = FILE_MASKS[file] | ADJACENT_FILES[file];
        let ahead_mask = match color {
            Color::White => blocking_files & ranks_above(rank),
            Color::Black => blocking_files & ranks_below(rank),
        };
        if (enemy_pawns & ahead_mask) == 0 && rel_rank < 8 {
            mg += PASSED_BONUS_MG[rel_rank.min(7)];
            eg += PASSED_BONUS_EG[rel_rank.min(7)];
        } else if rel_rank < 8 {
            let same_file_ahead = FILE_MASKS[file]
                & match color {
                    Color::White => ranks_above(rank),
                    Color::Black => ranks_below(rank),
                };
            if enemy_pawns & same_file_ahead == 0 {
                let adj_ahead = ahead_mask & ADJACENT_FILES[file];
                if count_bits(enemy_pawns & adj_ahead) <= 1 {
                    mg += CANDIDATE_PASSED_MG[rel_rank.min(7)];
                    eg += CANDIDATE_PASSED_EG[rel_rank.min(7)];
                }
            }
        }

        if (enemy_pawns & ahead_mask) == 0 && rel_rank < 8 {
            // King proximity / support for fully passed pawns (existing terms).

            // King proximity bonus for passed pawns (endgame)
            let pawn_dist_to_enemy_king = chebyshev_distance(sq_idx, enemy_king_sq);
            let pawn_dist_to_our_king = chebyshev_distance(sq_idx, our_king_sq);

            // Enemy king far from passed pawn = good for us
            eg += (pawn_dist_to_enemy_king as i32) * 5 * (rel_rank as i32);
            // Our king close to passed pawn = good for us
            eg -= (pawn_dist_to_our_king as i32) * 2 * (rel_rank as i32);

            // Supported passed pawn (another pawn behind it)
            let support_mask = match color {
                Color::White => {
                    if rank > 0 {
                        ADJACENT_FILES[file] & RANK_MASKS[rank - 1]
                    } else {
                        0
                    }
                }
                Color::Black => {
                    if rank < 7 {
                        ADJACENT_FILES[file] & RANK_MASKS[rank + 1]
                    } else {
                        0
                    }
                }
            };
            if our_pawns & support_mask != 0 {
                eg += 15 * (rel_rank as i32);
            }
        }
    }

    (mg, eg)
}

#[inline(always)]
fn chebyshev_distance(sq1: usize, sq2: usize) -> usize {
    let r1 = sq1 / 8;
    let f1 = sq1 % 8;
    let r2 = sq2 / 8;
    let f2 = sq2 % 8;
    let rd = r1.abs_diff(r2);
    let fd = f1.abs_diff(f2);
    rd.max(fd)
}

#[inline(always)]
fn ranks_below(rank: usize) -> Bitboard {
    RANK_MASKS[..rank]
        .iter()
        .copied()
        .fold(0, |mask, rank_mask| mask | rank_mask)
}

#[inline(always)]
fn ranks_above(rank: usize) -> Bitboard {
    RANK_MASKS[rank + 1..]
        .iter()
        .copied()
        .fold(0, |mask, rank_mask| mask | rank_mask)
}

// ── Outposts ────────────────────────────────────────────────────────────────
#[inline]
fn eval_outposts(
    board: &Board,
    color: Color,
    enemy_pawns: Bitboard,
    _enemy_pawn_attacks: Bitboard,
) -> (i32, i32) {
    let mut mg = 0i32;
    let mut eg = 0i32;
    let our_pawns = board.piece_bb(Piece::Pawn, color);

    // Knight outposts
    for sq_idx in iter_bits(board.piece_bb(Piece::Knight, color)) {
        let rank = sq_idx / 8;
        let file = sq_idx % 8;
        let rel_rank = match color {
            Color::White => rank,
            Color::Black => 7 - rank,
        };

        // Must be on ranks 4-6 (from our perspective) and not attackable by enemy pawns
        if (3..=5).contains(&rel_rank) {
            // Check no enemy pawns can attack this square from adjacent files ahead
            let ahead_adj = match color {
                Color::White => ADJACENT_FILES[file] & ranks_above(rank),
                Color::Black => ADJACENT_FILES[file] & ranks_below(rank),
            };
            if enemy_pawns & ahead_adj == 0 {
                // Supported by our pawn
                let support = pawn_attacks(color.opponent().index(), sq_idx) & our_pawns;
                if support != 0 {
                    mg += KNIGHT_OUTPOST_MG;
                    eg += KNIGHT_OUTPOST_EG;
                }
            }
        }
    }

    // Bishop outposts
    for sq_idx in iter_bits(board.piece_bb(Piece::Bishop, color)) {
        let rank = sq_idx / 8;
        let file = sq_idx % 8;
        let rel_rank = match color {
            Color::White => rank,
            Color::Black => 7 - rank,
        };

        if (3..=5).contains(&rel_rank) {
            let ahead_adj = match color {
                Color::White => ADJACENT_FILES[file] & ranks_above(rank),
                Color::Black => ADJACENT_FILES[file] & ranks_below(rank),
            };
            if enemy_pawns & ahead_adj == 0 {
                let support = pawn_attacks(color.opponent().index(), sq_idx) & our_pawns;
                if support != 0 {
                    mg += BISHOP_OUTPOST_MG;
                    eg += BISHOP_OUTPOST_EG;
                }
            }
        }
    }

    (mg, eg)
}

// ── Threats ─────────────────────────────────────────────────────────────────
#[inline]
fn eval_threats(
    board: &Board,
    color: Color,
    our_pawn_attacks: Bitboard,
    enemy_pawn_attacks: Bitboard,
) -> (i32, i32) {
    let mut mg = 0i32;
    let mut eg = 0i32;
    let them = color.opponent();
    let occ = board.all_occupancy();

    // Threats by our pawns on enemy pieces
    for &piece in &Piece::ALL {
        let enemy_bb = board.piece_bb(piece, them);
        let attacked = enemy_bb & our_pawn_attacks;
        let count = count_bits(attacked) as i32;
        if count > 0 {
            mg += THREAT_BY_PAWN_MG[piece.index()] * count;
            eg += THREAT_BY_PAWN_EG[piece.index()] * count;
        }
    }

    // Threats by our minor pieces on enemy rooks/queens
    let our_knight_attacks: Bitboard = {
        let mut atk = 0u64;
        for sq in iter_bits(board.piece_bb(Piece::Knight, color)) {
            atk |= knight_attacks(sq);
        }
        atk
    };
    let our_bishop_attacks: Bitboard = {
        let mut atk = 0u64;
        for sq in iter_bits(board.piece_bb(Piece::Bishop, color)) {
            atk |= bishop_attacks(sq, occ);
        }
        atk
    };
    let minor_attacks = our_knight_attacks | our_bishop_attacks;

    for &piece in &Piece::ALL {
        let enemy_bb = board.piece_bb(piece, them);
        let attacked = enemy_bb & minor_attacks;
        let count = count_bits(attacked) as i32;
        if count > 0 {
            mg += THREAT_BY_MINOR_MG[piece.index()] * count;
            eg += THREAT_BY_MINOR_EG[piece.index()] * count;
        }
    }

    // Hanging pieces (attacked by us, not defended by enemy pawns)
    let our_attacks = our_pawn_attacks | minor_attacks;
    let enemy_pieces = board.color_occupancy(them) & !board.piece_bb(Piece::Pawn, them);
    let hanging = enemy_pieces & our_attacks & !enemy_pawn_attacks;
    let hanging_count = count_bits(hanging) as i32;
    mg += HANGING_PENALTY_MG * hanging_count;
    eg += HANGING_PENALTY_EG * hanging_count;

    (mg, eg)
}

#[inline]
fn eval_pins(board: &Board, color: Color, occ: Bitboard) -> (i32, i32) {
    let king = board.king_square(color).index();
    let us = board.color_occupancy(color);
    let them = color.opponent();
    let enemy_diag = board.piece_bb(Piece::Bishop, them) | board.piece_bb(Piece::Queen, them);
    let enemy_orth = board.piece_bb(Piece::Rook, them) | board.piece_bb(Piece::Queen, them);
    let mut mg = 0i32;
    let mut eg = 0i32;

    let diag_hits = bishop_attacks(king, occ) & us;
    for sq in iter_bits(diag_hits) {
        let without = occ ^ (1u64 << sq);
        let beyond = bishop_attacks(king, without) & !bishop_attacks(king, occ);
        if beyond & enemy_diag != 0
            && let Some((piece, _)) = board.piece_on(Square::from_index(sq))
        {
            mg += PIN_PENALTY_MG[piece.index()];
            eg += PIN_PENALTY_EG[piece.index()];
        }
    }

    let orth_hits = rook_attacks(king, occ) & us;
    for sq in iter_bits(orth_hits) {
        let without = occ ^ (1u64 << sq);
        let beyond = rook_attacks(king, without) & !rook_attacks(king, occ);
        if beyond & enemy_orth != 0
            && let Some((piece, _)) = board.piece_on(Square::from_index(sq))
        {
            mg += PIN_PENALTY_MG[piece.index()];
            eg += PIN_PENALTY_EG[piece.index()];
        }
    }

    (mg, eg)
}

// ── Space ───────────────────────────────────────────────────────────────────
#[inline]
fn eval_space(
    _board: &Board,
    color: Color,
    our_pawns: Bitboard,
    our_pawn_attacks: Bitboard,
) -> i32 {
    let safe_zone = match color {
        Color::White => RANK_MASKS[1] | RANK_MASKS[2] | RANK_MASKS[3], // ranks 2-4
        Color::Black => RANK_MASKS[4] | RANK_MASKS[5] | RANK_MASKS[6], // ranks 5-7
    };
    let space = safe_zone & CENTER_FILES & !our_pawns & !our_pawn_attacks;
    count_bits(space) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() {
        types::init();
    }

    #[test]
    fn test_starting_position_roughly_equal() {
        setup();
        let score = evaluate(&Board::new());
        assert!(score.abs() < 100, "Starting position eval: {score}");
    }

    #[test]
    fn test_material_advantage_missing_queen() {
        setup();
        let base = evaluate(&Board::new());
        let board =
            Board::from_fen("rnb1kbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        assert!(evaluate(&board) > base + 600);
    }

    #[test]
    fn test_material_advantage_missing_rook() {
        setup();
        let base = evaluate(&Board::new());
        let board =
            Board::from_fen("rnbqkbn1/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQq - 0 1").unwrap();
        assert!(evaluate(&board) > base + 300);
    }

    #[test]
    fn test_symmetry() {
        setup();
        let w = evaluate(&Board::new());
        let b = evaluate(
            &Board::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1").unwrap(),
        );
        // Tempo makes both side-to-move scores positive; the mirror should still match.
        assert!((w - b).abs() < 30, "Symmetry broken: w={w}, b={b}");
    }

    #[test]
    fn test_bishop_pair() {
        setup();
        let pair = evaluate(&Board::from_fen("4k3/8/8/8/8/8/8/2B1KB2 w - - 0 1").unwrap());
        let single = evaluate(&Board::from_fen("4k3/8/8/8/8/8/8/4KB2 w - - 0 1").unwrap());
        assert!(pair > single);
    }

    #[test]
    fn test_endgame_king() {
        setup();
        let center = evaluate(&Board::from_fen("8/8/8/4K3/8/8/8/4k3 w - - 0 1").unwrap());
        let corner = evaluate(&Board::from_fen("K7/8/8/8/8/8/8/4k3 w - - 0 1").unwrap());
        assert!(center > corner);
    }

    #[test]
    fn test_no_panic() {
        setup();
        for fen in [
            "8/8/8/8/8/8/8/4K2k w - - 0 1",
            "8/P7/8/8/8/8/8/4K2k w - - 0 1",
            "qqqq1qqq/qqqqqqqq/8/8/8/8/8/4K2k b - - 0 1",
        ] {
            let _ = evaluate(&Board::from_fen(fen).unwrap());
        }
    }

    #[test]
    fn test_sign_convention() {
        setup();
        assert!(evaluate(&Board::from_fen("4k3/8/8/8/8/8/8/4KQ2 w - - 0 1").unwrap()) > 0);
        assert!(evaluate(&Board::from_fen("4k3/8/8/8/8/8/8/4KQ2 b - - 0 1").unwrap()) < 0);
    }

    #[test]
    fn test_passed_pawn() {
        setup();
        let passed = evaluate(&Board::from_fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap());
        let normal = evaluate(&Board::from_fen("4k3/8/8/8/8/8/P7/4K3 w - - 0 1").unwrap());
        assert!(passed > normal);
    }

    #[test]
    fn test_rook_open_file() {
        setup();
        let open = evaluate(&Board::from_fen("4k3/8/8/8/8/8/1P6/R3K3 w - - 0 1").unwrap());
        let closed = evaluate(&Board::from_fen("4k3/8/8/8/8/8/P7/R3K3 w - - 0 1").unwrap());
        assert!(
            open > closed,
            "Rook on open file should score higher: open={open}, closed={closed}"
        );
    }

    #[test]
    fn test_mobility_matters() {
        setup();
        // Knight in center vs corner should have better mobility
        let center = evaluate(&Board::from_fen("4k3/8/8/4N3/8/8/8/4K3 w - - 0 1").unwrap());
        let corner = evaluate(&Board::from_fen("4k3/8/8/8/8/8/8/N3K3 w - - 0 1").unwrap());
        assert!(
            center > corner,
            "Center knight should score higher: center={center}, corner={corner}"
        );
    }

    #[test]
    fn classical_eval_is_deterministic() {
        setup();
        let board =
            Board::from_fen("r1bq1rk1/ppp2ppp/2n2n2/2bp4/4P3/2P2N2/PP1N1PPP/R1BQ1RK1 w - - 2 9")
                .unwrap();
        let first = evaluate(&board);
        for _ in 0..32 {
            assert_eq!(evaluate(&board), first);
        }
    }

    #[test]
    fn tempo_favors_side_to_move_on_symmetric_material() {
        setup();
        let white = evaluate(&Board::new());
        let black = evaluate(
            &Board::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1").unwrap(),
        );
        // With tempo, each side-to-move score should be non-negative on startpos.
        assert!(white >= 0, "white-to-move startpos={white}");
        assert!(black >= 0, "black-to-move startpos={black}");
    }

    #[test]
    fn pawn_lever_counts_our_pawns_that_attack_enemy_pawns() {
        setup();
        let two_white = (1u64 << 26) | (1u64 << 28); // c4 and e4
        let one_black = 1u64 << 35; // d5
        assert_eq!(count_lever_pawns(two_white, one_black, Color::White), 2);
        assert_eq!(count_lever_pawns(one_black, two_white, Color::Black), 1);
    }

    #[test]
    fn absolute_pin_penalizes_the_pinned_side() {
        setup();
        let pinned = evaluate(&Board::from_fen("4k3/8/8/8/7b/8/5N2/4K3 w - - 0 1").unwrap());
        let free = evaluate(&Board::from_fen("4k3/8/8/8/8/8/5N2/4K3 w - - 0 1").unwrap());
        assert!(
            pinned < free,
            "pinned knight should score worse: pinned={pinned}, free={free}"
        );
    }

    #[test]
    fn candidate_passer_outscores_a_same_file_block() {
        setup();
        let our = 1u64 << 28; // e4
        let candidate_enemy = 1u64 << 53; // f7: adjacent file, does not attack e5
        let blocked_enemy = 1u64 << 52; // e7
        let (cand_mg, cand_eg) = eval_pawn_structure(our, candidate_enemy, Color::White, 4, 60);
        let (block_mg, block_eg) = eval_pawn_structure(our, blocked_enemy, Color::White, 4, 60);
        assert!(
            cand_mg > block_mg && cand_eg > block_eg,
            "candidate={cand_mg}/{cand_eg} blocked={block_mg}/{block_eg}"
        );
    }
}
