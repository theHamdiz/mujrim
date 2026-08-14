//! Root-only draw conversion: contempt and anti-repetition when ahead.
//!
//! Interior search still scores draws with a tiny deterministic jitter so
//! identical repetitions do not look like the same node. Contempt and
//! eval-scaled aversion are applied only to root move scores.

/// Default UCI/search contempt (centipawns). Applied only in HCE mode.
pub const DEFAULT_CONTEMPT: i32 = 32;

/// Static eval (cp) at which a repeating root move is treated as a missed win.
pub const WIN_CONVERSION_CP: i32 = 150;

/// Mate-distance band that must never be shifted by contempt.
const MATE_BAND: i32 = 30_000;

/// Interior draw jitter: ±1 from the node counter. Unchanged from the
/// historical helper so TT collisions do not all look like the same draw.
#[inline(always)]
pub const fn interior_draw_score(nodes: u64) -> i32 {
    -1 + (nodes & 2) as i32
}

/// Root draw score: interior jitter minus contempt so the side to move
/// prefers playing on.
#[inline]
pub fn root_draw_score(nodes: u64, contempt: i32) -> i32 {
    interior_draw_score(nodes).saturating_sub(contempt.clamp(-100, 100))
}

/// Penalty applied to a root move that repeats or is a near-draw while ahead.
#[inline]
pub fn root_conversion_adjustment(repeats: bool, static_eval: i32, contempt: i32) -> i32 {
    let contempt = contempt.clamp(-100, 100);
    if !repeats {
        return 0;
    }
    let mut penalty = contempt;
    if static_eval >= WIN_CONVERSION_CP {
        penalty += 40 + (static_eval - WIN_CONVERSION_CP) / 8;
    } else if static_eval <= -WIN_CONVERSION_CP {
        // When losing, a repetition is acceptable; do not punish it.
        penalty = 0;
    }
    penalty
}

/// Adjust a searched root-move score. Mate scores are left untouched.
#[inline]
pub fn apply_root_conversion(score: i32, repeats: bool, static_eval: i32, contempt: i32) -> i32 {
    if score.abs() >= MATE_BAND {
        return score;
    }
    score.saturating_sub(root_conversion_adjustment(repeats, static_eval, contempt))
}

/// Extra soft-time multiplier when the root score is a clear win.
#[inline]
pub fn winning_time_multiplier(root_score: i32) -> f64 {
    if root_score >= WIN_CONVERSION_CP {
        1.25
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interior_jitter_matches_historical_draw_score() {
        assert_eq!(interior_draw_score(0), -1);
        assert_eq!(interior_draw_score(1), -1);
        assert_eq!(interior_draw_score(2), 1);
        assert_eq!(interior_draw_score(3), 1);
        assert_eq!(interior_draw_score(4), -1);
    }

    #[test]
    fn root_draw_applies_contempt_only_at_root() {
        assert_eq!(root_draw_score(0, 32), -33);
        assert_eq!(root_draw_score(2, 32), -31);
        assert_eq!(interior_draw_score(0), -1);
    }

    #[test]
    fn repeating_move_is_punished_when_ahead() {
        let penalty = root_conversion_adjustment(true, 220, 32);
        assert!(penalty >= 32 + 40);
        assert_eq!(apply_root_conversion(0, true, 220, 32), -penalty);
    }

    #[test]
    fn repeating_move_is_not_punished_when_losing() {
        assert_eq!(root_conversion_adjustment(true, -200, 32), 0);
        assert_eq!(apply_root_conversion(0, true, -200, 32), 0);
    }

    #[test]
    fn non_repeating_move_keeps_its_score() {
        assert_eq!(apply_root_conversion(40, false, 220, 32), 40);
    }

    #[test]
    fn mate_scores_are_never_shifted() {
        assert_eq!(apply_root_conversion(31_000, true, 400, 32), 31_000);
        assert_eq!(apply_root_conversion(-31_000, true, 400, 32), -31_000);
    }

    #[test]
    fn winning_positions_spend_more_clock() {
        assert_eq!(winning_time_multiplier(149), 1.0);
        assert_eq!(winning_time_multiplier(150), 1.25);
    }
}
