//! Search Parameters — tunable constants for the alpha-beta search engine.
//!
//! Each NNUE network has its own optimal search parameters (tuned via SPRT).
//! The `SearchParams` struct encapsulates all of these, with factory methods
//! for each supported network family.
//!
//! # Presets
//! - `SearchParams::akimbo()` — current defaults, tuned for Akimbo-family nets
//! - `SearchParams::stockfish()` — Stockfish's SPRT-tuned values for SF nets

/// All tunable search constants in one place.
///
/// This struct replaces the scattered `const` definitions in `engine.rs` and
/// allows the engine to swap parameter sets when switching NNUE networks.
#[derive(Clone, Debug)]
pub struct SearchParams {
    // ── Razoring ────────────────────────────────────────────────────
    /// Razoring margin base: `base + depth_mul * depth²`.
    pub razoring_base: i32,
    /// Razoring depth-squared multiplier.
    pub razoring_depth_mul: i32,

    // ── Reverse futility pruning (RFP) ─────────────────────────────
    /// RFP margin per depth: `rfp_mul * depth - rfp_improving_bonus`.
    pub rfp_mul: i32,
    /// RFP bonus when the position is improving.
    pub rfp_improving_bonus: i32,

    // ── Futility pruning (move loop) ───────────────────────────────
    /// Futility base constant.
    pub futility_base: i32,
    /// Futility margin per depth²: `futility_base + futility_mul * depth²`.
    pub futility_mul: i32,
    /// Futility bonus when improving (not used in quadratic formula).
    pub futility_improving_bonus: i32,
    /// Maximum depth to apply futility pruning.
    pub futility_depth_limit: i32,

    // ── Null-move pruning ──────────────────────────────────────────
    /// Minimum depth to attempt null-move pruning (Akimbo=2, Stockfish=3).
    pub nmp_depth_min: i32,
    /// NMP base reduction: `nmp_base + depth / nmp_depth_div`.
    pub nmp_base: i32,
    /// NMP depth divisor.
    pub nmp_depth_div: i32,
    /// NMP eval-beta divisor: `+ ((eval - beta) / nmp_eval_div).min(nmp_eval_max)`.
    pub nmp_eval_div: i32,
    /// NMP eval-beta max bonus.
    pub nmp_eval_max: i32,

    // ── Late Move Reductions (LMR) ─────────────────────────────────
    /// LMR table base: `lmr_base + ln(depth) * ln(moves) / lmr_divisor`.
    pub lmr_base: f64,
    /// LMR table divisor.
    pub lmr_divisor: f64,
    /// LMR cut-node bonus (whole plies added for non-PV cut nodes).
    pub lmr_cut_node_bonus: i32,

    // ── Late Move Pruning (LMP) ────────────────────────────────────
    /// Maximum depth to apply LMP.
    pub lmp_depth_limit: i32,

    // ── History pruning ────────────────────────────────────────────
    /// History pruning margin per depth: skip if `stat_score < hist_prune_margin * depth`.
    pub hist_prune_margin: i32,
    /// Maximum depth for history pruning.
    pub hist_prune_depth_limit: i32,
    /// History LMR divisor: `reduction -= stat_score / hist_lmr_div`.
    pub hist_lmr_div: i32,

    // ── SEE pruning ────────────────────────────────────────────────
    /// SEE margin for capture pruning: `see_ge(mv, see_capture_margin * depth)`.
    pub see_capture_margin: i32,
    /// SEE margin for quiet pruning: `see_ge(mv, see_quiet_margin * depth + ...)`.
    pub see_quiet_margin: i32,
    /// Maximum depth for SEE pruning.
    pub see_prune_depth_limit: i32,

    // ── Singular extensions ────────────────────────────────────────
    /// SE margin: `se_margin_mul * depth`.
    pub se_margin_mul: i32,
    /// Minimum depth to try singular extensions.
    pub se_depth_min: i32,
    /// Maximum double extensions per path.
    pub max_dbl_exts: i32,

    // ── Null-move verification ─────────────────────────────────────
    /// Minimum depth to trigger NMP verification search.
    pub nmp_min_verif_depth: i32,
    /// NMP verification fraction: `(depth - R) * frac / 16`.
    pub nmp_verif_frac: i32,

    // ── Aspiration windows ─────────────────────────────────────────
    /// Initial aspiration window half-width (centipawns).
    pub aspiration_window: i32,

    // ── Quiescence ─────────────────────────────────────────────────
    /// Delta pruning margin in QS.
    pub delta_margin: i32,
    /// Maximum QS depth to prevent explosion.
    pub max_qs_ply: i32,

    // ── History bonus / malus ──────────────────────────────────────
    /// History bonus formula: `min(bonus_max, bonus_mul * depth - bonus_sub)`.
    pub history_bonus_mul: i32,
    /// History bonus subtracted constant.
    pub history_bonus_sub: i32,
    /// Maximum history bonus.
    pub history_bonus_max: i32,
    /// History malus formula: `min(malus_max, malus_mul * depth - malus_sub)`.
    pub history_malus_mul: i32,
    /// History malus subtracted constant.
    pub history_malus_sub: i32,
    /// Maximum history malus.
    pub history_malus_max: i32,

    // ── Correction history ─────────────────────────────────────────
    /// LMR correction multiplier.
    pub lmr_corr_mul: i32,
}

impl SearchParams {
    /// Default parameters for Akimbo-family NNUE networks.
    ///
    /// These match the hardcoded constants from the original engine.
    pub fn akimbo() -> Self {
        Self {
            // Razoring
            razoring_base: 507,
            razoring_depth_mul: 312,

            // Reverse futility pruning
            rfp_mul: 77,
            rfp_improving_bonus: 74,

            // Futility pruning (move loop) — quadratic (Akimbo: fp_base + fp_margin * d²)
            futility_base: 188,
            futility_mul: 35,
            futility_improving_bonus: 0,
            futility_depth_limit: 6,

            // Null-move pruning
            nmp_depth_min: 2,
            nmp_base: 5,
            nmp_depth_div: 5,
            nmp_eval_div: 198,
            nmp_eval_max: 6,

            // LMR
            lmr_base: 0.48,
            lmr_divisor: 2.48,
            lmr_cut_node_bonus: 2,

            // LMP
            lmp_depth_limit: 9,

            // History pruning
            hist_prune_margin: -1682,
            hist_prune_depth_limit: 6,
            hist_lmr_div: 8192,

            // SEE pruning
            see_capture_margin: -148,
            see_quiet_margin: -64,
            see_prune_depth_limit: 7,

            // Singular extensions
            se_margin_mul: 1,
            se_depth_min: 8,
            max_dbl_exts: 5,

            // NMP verification
            nmp_min_verif_depth: 17,
            nmp_verif_frac: 12,

            // Aspiration
            aspiration_window: 16,

            // Quiescence
            delta_margin: 400,
            max_qs_ply: 8,

            // History bonus / malus — Akimbo tuned values
            history_bonus_mul: 375,
            history_bonus_sub: 141,
            history_bonus_max: 1827,
            history_malus_mul: 396,
            history_malus_sub: 8,
            history_malus_max: 1192,

            // Correction history
            lmr_corr_mul: 448,
        }
    }

    /// Parameters tuned for Stockfish NNUE networks.
    ///
    /// Values extracted from Stockfish's source (SPRT-tuned over millions
    /// of games). These are optimal for the HalfKAv2_hm architecture.
    pub fn stockfish() -> Self {
        Self {
            // Razoring — identical to Akimbo (both use SF formula)
            razoring_base: 507,
            razoring_depth_mul: 312,

            // Reverse futility pruning — SF: futilityMult = 77, with TT hit adj
            rfp_mul: 77,
            rfp_improving_bonus: 74,

            // Futility — SF applies at depth < 16 (much deeper than Akimbo's 6)
            futility_base: 188,
            futility_mul: 77,
            futility_improving_bonus: 46,
            futility_depth_limit: 13,

            // Null-move pruning — SF: R = 7 + depth/3
            nmp_depth_min: 3,
            nmp_base: 7,
            nmp_depth_div: 3,
            nmp_eval_div: 200,
            nmp_eval_max: 3,

            // LMR — SF: reductions[i] = 2809/128 * ln(i), but we keep the 2D table
            // SF's effective LMR is more aggressive due to the 1D base + per-node adjustments
            lmr_base: 0.77,
            lmr_divisor: 2.36,
            lmr_cut_node_bonus: 3,

            // LMP — SF has no explicit depth limit, applies via moveCount formula
            lmp_depth_limit: 12,

            // History pruning — SF: -3826 * depth (more aggressive)
            hist_prune_margin: -3826,
            hist_prune_depth_limit: 12,
            hist_lmr_div: 2917,

            // SEE pruning — SF: captures margin based on depth + captHist
            see_capture_margin: -185,
            see_quiet_margin: -25,
            see_prune_depth_limit: 12,

            // Singular extensions — SF: singularBeta = ttValue - 58*depth/57
            se_margin_mul: 1,
            se_depth_min: 8,
            max_dbl_exts: 5,

            // NMP verification
            nmp_min_verif_depth: 17,
            nmp_verif_frac: 12,

            // Aspiration
            aspiration_window: 16,

            // Quiescence
            delta_margin: 400,
            max_qs_ply: 8,

            // History bonus — SF: 121*depth - 75 (capped at 932)
            history_bonus_mul: 121,
            history_bonus_sub: 75,
            history_bonus_max: 932,
            history_malus_mul: 121,
            history_malus_sub: 75,
            history_malus_max: 932,

            // Correction history
            lmr_corr_mul: 448,
        }
    }

    /// Select the best preset for a given network type.
    ///
    /// # Arguments
    /// - `preset_name`: One of `"akimbo"`, `"stockfish"`, or any custom name.
    pub fn for_preset(preset_name: &str) -> Self {
        match preset_name {
            "stockfish" => Self::stockfish(),
            _ => Self::akimbo(),
        }
    }

    // ── Computed helper methods ─────────────────────────────────────

    /// Razoring margin: `base + mul * depth²`.
    #[inline(always)]
    pub fn razoring_margin(&self, depth: i32) -> i32 {
        self.razoring_base + self.razoring_depth_mul * depth * depth
    }

    /// Reverse futility pruning margin: `mul * depth - improving_bonus`.
    #[inline(always)]
    pub fn rfp_margin(&self, depth: i32, improving: bool) -> i32 {
        self.rfp_mul * depth
            - if improving {
                self.rfp_improving_bonus
            } else {
                0
            }
    }

    /// Futility margin: `base + mul * depth²`.
    #[inline(always)]
    pub fn futility_margin(&self, depth: i32, _improving: bool) -> i32 {
        self.futility_base + self.futility_mul * depth * depth
    }

    /// Singular extension margin: `mul * depth`.
    #[inline(always)]
    pub fn se_margin(&self, depth: i32) -> i32 {
        self.se_margin_mul * depth
    }

    /// LMP threshold: `(3 + depth²) / (2 - improving)`.
    #[inline(always)]
    pub fn lmp_threshold(&self, depth: i32, improving: bool) -> usize {
        ((3 + depth * depth) / if improving { 1 } else { 2 }) as usize
    }

    /// Null-move reduction: `base + depth/div + ((eval-beta)/eval_div).min(eval_max)`.
    #[inline(always)]
    pub fn null_move_r(&self, depth: i32, eval: i32, beta: i32) -> i32 {
        self.nmp_base
            + depth / self.nmp_depth_div
            + ((eval - beta) / self.nmp_eval_div).min(self.nmp_eval_max)
    }

    /// History bonus: `min(bonus_max, mul * depth - sub)`.
    #[inline(always)]
    pub fn history_bonus(&self, depth: i32) -> i32 {
        self.history_bonus_max
            .min(self.history_bonus_mul * depth - self.history_bonus_sub)
    }

    /// History malus: `min(malus_max, malus_mul * depth - malus_sub)`.
    #[inline(always)]
    pub fn history_malus(&self, depth: i32) -> i32 {
        self.history_malus_max
            .min(self.history_malus_mul * depth - self.history_malus_sub)
    }

    /// Build the LMR reduction table from `lmr_base` and `lmr_divisor`.
    pub fn build_lmr_table(&self) -> [[i32; 128]; 128] {
        let mut table = [[0i32; 128]; 128];
        for depth in 1..128 {
            for moves in 1..128 {
                table[depth][moves] = (self.lmr_base
                    + (depth as f64).ln() * (moves as f64).ln() / self.lmr_divisor)
                    as i32;
            }
        }
        table
    }
}

impl Default for SearchParams {
    fn default() -> Self {
        Self::akimbo()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_akimbo_defaults() {
        let p = SearchParams::akimbo();
        assert_eq!(p.razoring_margin(2), 507 + 312 * 4);
        assert_eq!(p.rfp_margin(5, true), 77 * 5 - 74);
        assert_eq!(p.rfp_margin(5, false), 77 * 5);
        assert_eq!(p.null_move_r(10, 200, 100), 5 + 2 + 0); // (200-100)/200 = 0
        assert_eq!(p.lmp_threshold(3, false), (3 + 9) / 2); // 6
        assert_eq!(p.lmp_threshold(3, true), 3 + 9); // 12
    }

    #[test]
    fn test_stockfish_preset() {
        let p = SearchParams::stockfish();
        assert_eq!(p.nmp_base, 7);
        assert_eq!(p.nmp_depth_div, 3);
        assert_eq!(p.null_move_r(12, 300, 200), 7 + 4 + 0); // 11
        assert_eq!(p.futility_depth_limit, 13);
        assert_eq!(p.hist_prune_margin, -3826);
    }

    #[test]
    fn test_for_preset() {
        let akimbo = SearchParams::for_preset("akimbo");
        let sf = SearchParams::for_preset("stockfish");
        assert_eq!(akimbo.nmp_base, 5);
        assert_eq!(sf.nmp_base, 7);

        // Unknown preset falls back to akimbo
        let unknown = SearchParams::for_preset("unknown");
        assert_eq!(unknown.nmp_base, 5);
    }

    #[test]
    fn test_lmr_table() {
        let p = SearchParams::akimbo();
        let table = p.build_lmr_table();
        // LMR[10][10] = floor(0.48 + ln(10)*ln(10)/2.48) = floor(0.48 + 2.14) = 2
        assert_eq!(table[10][10], 2);
        // LMR[1][1] = floor(0.48 + 0) = 0
        assert_eq!(table[1][1], 0);
    }
}
