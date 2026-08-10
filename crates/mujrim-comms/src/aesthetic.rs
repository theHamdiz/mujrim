//! Root-only aesthetic move selection.
//!
//! This module deliberately has no dependency on search internals. Style is
//! considered only among already-scored root moves that pass the configured
//! centipawn gate.

use types::chess_move::MoveFlag;
use types::{Board, Color, Move, Piece, Square};

/// The largest configurable loss accepted by the aesthetic selector.
pub const MAX_AESTHETIC_DELTA_CP: i32 = 30;
/// Converts style points into centipawns after the hard evaluation gate.
pub const STYLE_WEIGHT: f32 = 0.1;
const FORCED_MATE_SCORE: i32 = 28_900;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AestheticConfig {
    pub enabled: bool,
    pub max_delta_cp: i32,
}

impl Default for AestheticConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_delta_cp: MAX_AESTHETIC_DELTA_CP,
        }
    }
}

impl AestheticConfig {
    #[inline]
    pub fn effective_delta(self, board: &Board, top_eval: i32) -> i32 {
        if !self.enabled || board.is_endgame() || top_eval.abs() >= FORCED_MATE_SCORE {
            0
        } else {
            self.max_delta_cp.clamp(0, MAX_AESTHETIC_DELTA_CP)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RootCandidate {
    pub mv: Move,
    /// Evaluation in centipawns from the root side's perspective.
    pub eval: i32,
}

/// Selects a move from already-scored root Multi-PV candidates.
///
/// Candidate order breaks equal-evaluation ties, so callers should provide
/// engine order. A zero effective delta returns that first absolute best move.
pub fn select_root_move(
    board: &Board,
    candidates: &[RootCandidate],
    config: AestheticConfig,
) -> Option<Move> {
    let (top_index, top) = candidates
        .iter()
        .enumerate()
        .max_by_key(|(index, candidate)| (candidate.eval, std::cmp::Reverse(*index)))?;

    let delta = config.effective_delta(board, top.eval);
    if delta == 0 {
        return Some(top.mv);
    }

    let threshold = top.eval.saturating_sub(delta);
    let mut selected_index = top_index;
    let mut selected_score = top.eval as f32 + STYLE_WEIGHT * evaluate_move_style(board, top.mv);

    for (index, candidate) in candidates.iter().enumerate() {
        if candidate.eval < threshold {
            continue;
        }
        let combined =
            candidate.eval as f32 + STYLE_WEIGHT * evaluate_move_style(board, candidate.mv);
        if combined > selected_score {
            selected_index = index;
            selected_score = combined;
        }
    }

    Some(candidates[selected_index].mv)
}

/// Computes a static style score without allocating or mutating the position.
pub fn evaluate_move_style(pos: &Board, mv: Move) -> f32 {
    let us = pos.side_to_move;
    let them = us.opponent();
    let Some(moving_piece) = pos.piece_of_color_on(mv.from, us) else {
        return 0.0;
    };
    let mut score = 0.0;

    if !matches!(moving_piece, Piece::Pawn | Piece::King)
        && let Some(defender_value) = minimum_attacker_value(pos, mv, mv.to, them)
    {
        let attacker_value = style_piece_value(moving_piece);
        let defended = minimum_attacker_value(pos, mv, mv.to, us).is_some();
        if attacker_value > defender_value && !defended {
            score += 150.0 * (attacker_value - defender_value) as f32;
        }
    }

    if matches!(moving_piece, Piece::Bishop | Piece::Rook | Piece::Queen) {
        let distance =
            mv.from.file().abs_diff(mv.to.file()) + mv.from.rank().abs_diff(mv.to.rank());
        if distance >= 4 {
            score += 10.0 * f32::from(distance);
        }
    }

    if matches!(
        mv.promotion,
        Some(Piece::Knight | Piece::Bishop | Piece::Rook)
    ) {
        score += 300.0;
    }

    if mv.flag == MoveFlag::EnPassant {
        score += 200.0;
    }

    if moving_piece == Piece::Pawn {
        let on_enemy_sixth_or_seventh = match us {
            Color::White => matches!(mv.to.rank(), 5 | 6),
            Color::Black => matches!(mv.to.rank(), 1 | 2),
        };
        if on_enemy_sixth_or_seventh {
            score += 80.0;
        }
    }

    if moving_piece == Piece::King
        && mv.is_quiet()
        && pos.piece_count(Piece::Queen, them) > 0
        && match us {
            Color::White => mv.to.rank() > mv.from.rank(),
            Color::Black => mv.to.rank() < mv.from.rank(),
        }
    {
        score += 120.0;
    }

    score
}

#[inline]
const fn style_piece_value(piece: Piece) -> i32 {
    match piece {
        Piece::Pawn => 1,
        Piece::Knight | Piece::Bishop => 3,
        Piece::Rook => 5,
        Piece::Queen => 9,
        Piece::King => 100,
    }
}

fn minimum_attacker_value(board: &Board, mv: Move, target: Square, attacker: Color) -> Option<i32> {
    let mut minimum = None;
    for from in Square::ALL {
        if from == mv.from {
            continue;
        }
        let Some((piece, color)) = board.piece_on(from) else {
            continue;
        };
        if color == attacker && piece_attacks_after_move(board, mv, piece, color, from, target) {
            let value = style_piece_value(piece);
            minimum = Some(minimum.map_or(value, |current: i32| current.min(value)));
        }
    }
    minimum
}

fn piece_attacks_after_move(
    board: &Board,
    mv: Move,
    piece: Piece,
    color: Color,
    from: Square,
    target: Square,
) -> bool {
    let file_delta = target.file() as i8 - from.file() as i8;
    let rank_delta = target.rank() as i8 - from.rank() as i8;
    let abs_file = file_delta.unsigned_abs();
    let abs_rank = rank_delta.unsigned_abs();

    match piece {
        Piece::Pawn => {
            abs_file == 1
                && rank_delta
                    == match color {
                        Color::White => 1,
                        Color::Black => -1,
                    }
        }
        Piece::Knight => matches!((abs_file, abs_rank), (1, 2) | (2, 1)),
        Piece::King => abs_file <= 1 && abs_rank <= 1 && (abs_file != 0 || abs_rank != 0),
        Piece::Bishop => {
            abs_file == abs_rank
                && abs_file != 0
                && ray_is_clear_after_move(board, mv, from, target)
        }
        Piece::Rook => {
            ((file_delta == 0) != (rank_delta == 0))
                && ray_is_clear_after_move(board, mv, from, target)
        }
        Piece::Queen => {
            ((abs_file == abs_rank && abs_file != 0) || ((file_delta == 0) != (rank_delta == 0)))
                && ray_is_clear_after_move(board, mv, from, target)
        }
    }
}

fn ray_is_clear_after_move(board: &Board, mv: Move, from: Square, target: Square) -> bool {
    let file_step = (target.file() as i8 - from.file() as i8).signum();
    let rank_step = (target.rank() as i8 - from.rank() as i8).signum();
    let mut file = from.file() as i8 + file_step;
    let mut rank = from.rank() as i8 + rank_step;

    while file != target.file() as i8 || rank != target.rank() as i8 {
        let square = Square::from_file_rank(file as u8, rank as u8);
        let en_passant_capture = mv.flag == MoveFlag::EnPassant
            && square.file() == mv.to.file()
            && square.rank() == mv.from.rank();
        if square != mv.from && !en_passant_capture && board.piece_on(square).is_some() {
            return false;
        }
        file += file_step;
        rank += rank_step;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(fen: &str) -> Board {
        types::init();
        Board::from_fen(fen).unwrap()
    }

    fn legal_move(board: &mut Board, uci: &str) -> Move {
        board
            .generate_legal_moves()
            .iter()
            .copied()
            .find(|mv| mv.to_uci() == uci)
            .unwrap_or_else(|| panic!("expected legal move {uci}"))
    }

    #[test]
    fn scores_requested_static_style_features() {
        let promotion = board("3q3k/P7/8/8/8/8/8/R2Q3K w - - 0 1");
        assert_eq!(
            evaluate_move_style(&promotion, legal_move(&mut promotion.clone(), "a7a8n")),
            300.0
        );

        let mut en_passant = board("7k/8/8/3pP3/8/8/8/K7 w - d6 0 1");
        let ep = legal_move(&mut en_passant, "e5d6");
        assert_eq!(evaluate_move_style(&en_passant, ep), 280.0);

        let mut king_walk = board("6k1/7q/8/8/8/8/4K3/8 w - - 0 1");
        let walk = legal_move(&mut king_walk, "e2e3");
        assert_eq!(evaluate_move_style(&king_walk, walk), 120.0);
    }

    #[test]
    fn scores_long_unprotected_sacrifice() {
        let mut position = board("7k/8/1p6/8/8/Q7/8/7K w - - 0 1");
        let mv = legal_move(&mut position, "a3c5");
        assert_eq!(evaluate_move_style(&position, mv), 1_240.0);
    }

    #[test]
    fn strict_delta_gate_discards_fancy_but_too_weak_move() {
        let mut position = board("3q3k/P7/8/8/8/8/8/R2Q3K w - - 0 1");
        let queen = legal_move(&mut position.clone(), "a7a8q");
        let knight = legal_move(&mut position, "a7a8n");
        let candidates = [
            RootCandidate {
                mv: queen,
                eval: 100,
            },
            RootCandidate {
                mv: knight,
                eval: 69,
            },
        ];
        assert_eq!(
            select_root_move(
                &position,
                &candidates,
                AestheticConfig {
                    enabled: true,
                    ..AestheticConfig::default()
                },
            ),
            Some(queen)
        );
    }

    #[test]
    fn aesthetic_score_can_select_a_move_inside_the_gate() {
        let mut position = board("3q3k/P7/8/8/8/8/8/R2Q3K w - - 0 1");
        let queen = legal_move(&mut position.clone(), "a7a8q");
        let knight = legal_move(&mut position, "a7a8n");
        let candidates = [
            RootCandidate {
                mv: queen,
                eval: 100,
            },
            RootCandidate {
                mv: knight,
                eval: 75,
            },
        ];
        assert_eq!(
            select_root_move(
                &position,
                &candidates,
                AestheticConfig {
                    enabled: true,
                    ..AestheticConfig::default()
                },
            ),
            Some(knight)
        );
    }

    #[test]
    fn mate_endgame_and_disabled_modes_force_engine_choice() {
        let mut middle = board("3q3k/P7/8/8/8/8/8/R2Q3K w - - 0 1");
        let queen = legal_move(&mut middle.clone(), "a7a8q");
        let knight = legal_move(&mut middle, "a7a8n");
        let mate = [
            RootCandidate {
                mv: queen,
                eval: FORCED_MATE_SCORE,
            },
            RootCandidate {
                mv: knight,
                eval: FORCED_MATE_SCORE - 1,
            },
        ];
        assert_eq!(
            select_root_move(&middle, &mate, AestheticConfig::default()),
            Some(queen)
        );

        let mut ending = board("7k/P7/8/8/8/8/8/K7 w - - 0 1");
        let queen = legal_move(&mut ending.clone(), "a7a8q");
        let knight = legal_move(&mut ending, "a7a8n");
        let candidates = [
            RootCandidate {
                mv: queen,
                eval: 100,
            },
            RootCandidate {
                mv: knight,
                eval: 100,
            },
        ];
        assert_eq!(
            select_root_move(&ending, &candidates, AestheticConfig::default()),
            Some(queen)
        );
        assert_eq!(
            select_root_move(
                &middle,
                &candidates,
                AestheticConfig {
                    enabled: false,
                    max_delta_cp: MAX_AESTHETIC_DELTA_CP,
                },
            ),
            Some(queen)
        );
    }
}
