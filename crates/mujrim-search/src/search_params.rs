//! Search Parameters — tunable constants for the alpha-beta search engine.
//!
//! Each NNUE network has its own optimal search parameters (tuned via SPRT).
//! The `SearchParams` struct encapsulates all of these, with factory methods
//! for each supported network family.
//!
//! # Presets
//! - `SearchParams::akimbo()` — current defaults, tuned for Akimbo-family nets
//! - `SearchParams::stockfish()` — Stockfish's SPRT-tuned values for SF nets

use std::path::{Path, PathBuf};
use std::sync::Arc;

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

    // ── Singular extensions ────────────────────────────────────────
    /// SE margin: `se_margin_mul * depth`.
    pub se_margin_mul: i32,
    /// Minimum depth to try singular extensions.
    pub se_depth_min: i32,
    /// Maximum double extensions per path.
    pub max_dbl_exts: i32,
    /// Second margin (centipawns) below `se_beta` to trigger double singular extension.
    pub se_double_ext_margin: i32,
    /// Maximum depth for low-depth cut-node extensions; zero disables them.
    pub ldse_depth_max: i32,
    /// Static-evaluation deficit below alpha required for a low-depth extension.
    pub ldse_margin: i32,

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

    // ── Evaluation-difference history ───────────────────────────────
    /// Scale applied to the parent/child static-evaluation swing.
    pub eval_history_scale: i32,
    /// Minimum evaluation-difference history update.
    pub eval_history_min: i32,
    /// Maximum evaluation-difference history update.
    pub eval_history_max: i32,
    /// Below this depth, learn even when the current position has a TT entry.
    pub eval_history_depth_limit: i32,

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
            futility_depth_limit: 3,

            // Null-move pruning
            nmp_depth_min: 4,
            nmp_base: 5,
            nmp_depth_div: 5,
            nmp_eval_div: 198,
            nmp_eval_max: 6,

            // LMR — keep quiets alive so BK pawn breaks and sacs survive.
            lmr_base: 0.30,
            lmr_divisor: 2.48,
            lmr_cut_node_bonus: 0,

            // LMP — trim quiet pruning at higher depths.
            lmp_depth_limit: 3,

            // History pruning
            hist_prune_margin: -1682,
            hist_prune_depth_limit: 2,
            hist_lmr_div: 8192,

            // Singular extensions — start earlier for deep tactics.
            se_margin_mul: 1,
            se_depth_min: 4,
            max_dbl_exts: 6,
            se_double_ext_margin: 25,
            ldse_depth_max: 0,
            ldse_margin: 0,

            // NMP verification
            nmp_min_verif_depth: 17,
            nmp_verif_frac: 12,

            // Aspiration
            aspiration_window: 16,

            // Quiescence — a bit deeper for mating/tactical sequences.
            delta_margin: 400,
            max_qs_ply: 14,

            // History bonus / malus — Akimbo tuned values
            history_bonus_mul: 375,
            history_bonus_sub: 141,
            history_bonus_max: 1827,
            history_malus_mul: 396,
            history_malus_sub: 8,
            history_malus_max: 1192,

            // Evaluation-difference history
            eval_history_scale: 812,
            eval_history_min: -144,
            eval_history_max: 324,
            eval_history_depth_limit: 6,

            // Correction history
            lmr_corr_mul: 448,
        }
    }

    /// Parameters tuned for Stockfish NNUE networks.
    ///
    /// Stockfish-inspired baselines with **tighter pruning depth caps** so tactical tests lose
    /// fewer quiet lines to LMP / SEE / futility. NMP uses the same base reduction as Reckless
    /// so shallow searches still leave real child depth (native SF's R=7 collapses our AB tree).
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
            futility_depth_limit: 8,

            // Null-move pruning — share Reckless/Akimbo R base; keep SF depth divisor.
            nmp_depth_min: 3,
            nmp_base: 5,
            nmp_depth_div: 3,
            nmp_eval_div: 200,
            nmp_eval_max: 3,

            // LMR — SF: reductions[i] = 2809/128 * ln(i), but we keep the 2D table
            lmr_base: 0.77,
            lmr_divisor: 2.36,
            lmr_cut_node_bonus: 1,

            lmp_depth_limit: 5,

            hist_prune_margin: -3826,
            hist_prune_depth_limit: 6,
            hist_lmr_div: 2917,

            se_margin_mul: 1,
            se_depth_min: 5,
            max_dbl_exts: 10,
            se_double_ext_margin: 22,
            ldse_depth_max: 0,
            ldse_margin: 0,

            // NMP verification
            nmp_min_verif_depth: 17,
            nmp_verif_frac: 12,

            aspiration_window: 10,

            delta_margin: 400,
            max_qs_ply: 16,

            // History bonus — SF: 121*depth - 75 (capped at 932)
            history_bonus_mul: 121,
            history_bonus_sub: 75,
            history_bonus_max: 932,
            history_malus_mul: 121,
            history_malus_sub: 75,
            history_malus_max: 932,

            // Evaluation-difference history
            eval_history_scale: 812,
            eval_history_min: -144,
            eval_history_max: 324,
            eval_history_depth_limit: 6,

            // Correction history
            lmr_corr_mul: 448,
        }
    }

    /// Parameters used with the embedded Reckless v60 network.
    ///
    /// Pinned independently of [`Self::akimbo`] so tactical Akimbo BK tweaks
    /// cannot leak into the Reckless adapter.
    pub fn reckless() -> Self {
        Self {
            razoring_base: 507,
            razoring_depth_mul: 312,
            rfp_mul: 77,
            rfp_improving_bonus: 46,
            futility_base: 188,
            futility_mul: 77,
            futility_improving_bonus: 46,
            futility_depth_limit: 6,
            nmp_depth_min: 2,
            nmp_base: 5,
            nmp_depth_div: 5,
            nmp_eval_div: 200,
            nmp_eval_max: 3,
            lmr_base: 0.77,
            lmr_divisor: 2.36,
            lmr_cut_node_bonus: 1,
            lmp_depth_limit: 7,
            hist_prune_margin: -1682,
            hist_prune_depth_limit: 8,
            hist_lmr_div: 4096,
            se_margin_mul: 2,
            se_depth_min: 5,
            max_dbl_exts: 6,
            se_double_ext_margin: 25,
            ldse_depth_max: 7,
            ldse_margin: 25,
            nmp_min_verif_depth: 17,
            nmp_verif_frac: 12,
            aspiration_window: 10,
            delta_margin: 400,
            max_qs_ply: 10,
            history_bonus_mul: 375,
            history_bonus_sub: 141,
            history_bonus_max: 1827,
            history_malus_mul: 396,
            history_malus_sub: 8,
            history_malus_max: 1192,
            eval_history_scale: 812,
            eval_history_min: -144,
            eval_history_max: 324,
            eval_history_depth_limit: 6,
            lmr_corr_mul: 448,
        }
    }

    /// Parameters paired with a Viridithas-family network.
    ///
    /// Slightly less LMR than Stockfish so converting lines survive, with the
    /// same NMP base used by the other NNUE stacks (keeps NPS stable).
    pub fn viridithas() -> Self {
        let mut params = Self::stockfish();
        // RecklessFull LMR finds BK#24 at 5s; softer history pruning keeps
        // BK#10 Ne5 from being dropped as a late quiet.
        params.lmr_base = 0.70;
        params.lmr_cut_node_bonus = 1;
        params.se_depth_min = 5;
        params.aspiration_window = 12;
        params
    }

    /// Parameters paired with an Obsidian-family layered network.
    pub fn obsidian() -> Self {
        let mut params = Self::stockfish();
        params.futility_depth_limit = 7;
        params.se_depth_min = 4;
        params.se_margin_mul = 2;
        params.ldse_depth_max = 7;
        params.ldse_margin = 25;
        params.lmr_cut_node_bonus = 1;
        params.aspiration_window = 12;
        params
    }

    /// Parameters paired with the PlentyChess in-process search profile.
    pub fn plentychess() -> Self {
        let mut params = Self::obsidian();
        params.lmr_base = 0.62;
        params.lmp_depth_limit = 5;
        params.se_depth_min = 5;
        params.aspiration_window = 12;
        params
    }

    /// Parameters for the in-process Lc0 fallback (official Lc0 is passthrough).
    ///
    /// Snapshotted from the Reckless-shaped set used when Lc0 last scored 20/24
    /// so Reckless-only BK restores cannot move this adapter.
    pub fn lc0() -> Self {
        let mut params = Self::reckless();
        params.nmp_depth_min = 4;
        params.lmr_cut_node_bonus = 0;
        params.lmp_depth_limit = 3;
        params.max_qs_ply = 14;
        params
    }

    /// Parameters for classical Mujrim HCE (no NNUE).
    ///
    /// Stockfish-shaped search with less LMP/NMP/LMR so pawn breaks and
    /// sacrifices survive. HCE leaves are cheap, so the extra nodes stay
    /// affordable at high NPS.
    pub fn mujrim_hce() -> Self {
        let mut params = Self::stockfish();
        params.lmr_base = 0.24;
        params.lmr_cut_node_bonus = 0;
        params.lmp_depth_limit = 1;
        params.futility_depth_limit = 2;
        params.futility_mul = 40;
        params.nmp_depth_min = 8;
        params.hist_prune_depth_limit = 0;
        params.rfp_mul = 110;
        params.se_depth_min = 4;
        params.se_margin_mul = 2;
        params.ldse_depth_max = 8;
        params.ldse_margin = 20;
        params.aspiration_window = 16;
        params.max_qs_ply = 16;
        params
    }

    /// Select the best preset for a given network type.
    ///
    /// # Arguments
    /// - `preset_name`: One of `"akimbo"`, `"stockfish"`, `"reckless"`, `"mujrim-hce"`, or a custom name.
    pub fn for_preset(preset_name: &str) -> Self {
        match preset_name {
            "stockfish" => Self::stockfish(),
            "reckless" => Self::reckless(),
            "viridithas" => Self::viridithas(),
            "obsidian" => Self::obsidian(),
            "plentychess" | "plenty" => Self::plentychess(),
            "lc0" => Self::lc0(),
            "mujrim-hce" | "hce" => Self::mujrim_hce(),
            _ => Self::akimbo(),
        }
    }

    /// Like [`Self::for_preset`], then apply an explicit runtime tuning file for
    /// non-Stockfish presets when `MUJRIM_TUNING_FILE` is set.
    ///
    /// Built-in presets are independent of the process working directory so an
    /// installed executable searches identically to a repository-local launch.
    #[must_use]
    pub fn for_preset_with_repo_tuning(preset_name: &str) -> Self {
        let base = Self::for_preset(preset_name);
        if matches!(
            preset_name,
            "stockfish"
                | "viridithas"
                | "obsidian"
                | "plentychess"
                | "plenty"
                | "lc0"
                | "mujrim-hce"
                | "hce"
        ) {
            return base;
        }
        if let Some(path) = std::env::var_os("MUJRIM_TUNING_FILE") {
            base.with_tuning_file(&PathBuf::from(path))
        } else {
            base
        }
    }

    /// Load optional overrides from a tuning TOML file.
    ///
    /// This reads `sprt/params.toml`-style values and applies known fields to
    /// the in-memory search parameters. Unknown/missing keys are ignored.
    pub fn with_tuning_file(mut self, path: &Path) -> Self {
        let Ok(raw) = std::fs::read_to_string(path) else {
            return self;
        };
        let Ok(root) = raw.parse::<toml::Value>() else {
            return self;
        };

        let get = |p: &[&str]| -> Option<f64> {
            let mut cur = &root;
            for key in p {
                cur = cur.get(*key)?;
            }
            cur.as_float()
                .or_else(|| cur.as_integer().map(|v| v as f64))
        };

        if let Some(v) = get(&["search", "null_move", "base_r", "value"]) {
            self.nmp_base = v as i32;
        }
        if let Some(v) = get(&["search", "null_move", "depth_divisor", "value"]) {
            self.nmp_depth_div = (v as i32).max(1);
        }
        if let Some(v) = get(&["search", "null_move", "eval_divisor", "value"]) {
            self.nmp_eval_div = (v as i32).max(1);
        }
        if let Some(v) = get(&["search", "null_move", "eval_max", "value"]) {
            self.nmp_eval_max = v as i32;
        }
        if let Some(v) = get(&["search", "lmr", "base", "value"]) {
            self.lmr_base = v / 100.0;
        }
        if let Some(v) = get(&["search", "lmr", "divisor", "value"]) {
            self.lmr_divisor = (v / 100.0).max(0.1);
        }
        if let Some(v) = get(&["search", "lmr", "history_divisor", "value"]) {
            self.hist_lmr_div = (v as i32).max(1);
        }
        if let Some(v) = get(&["search", "lmr", "corr_divisor", "value"]) {
            self.lmr_corr_mul = (v as i32).max(1);
        }
        if let Some(v) = get(&["search", "rfp", "margin_per_depth", "value"]) {
            self.rfp_mul = v as i32;
        }
        if let Some(v) = get(&["search", "rfp", "improving_bonus", "value"]) {
            self.rfp_improving_bonus = v as i32;
        }
        if let Some(v) = get(&["search", "rfp", "max_depth", "value"]) {
            self.hist_prune_depth_limit = v as i32;
        }
        if let Some(v) = get(&["search", "razoring", "base", "value"]) {
            self.razoring_base = v as i32;
        }
        if let Some(v) = get(&["search", "razoring", "quadratic", "value"]) {
            self.razoring_depth_mul = v as i32;
        }
        if let Some(v) = get(&["search", "futility", "margin_per_depth", "value"]) {
            self.futility_mul = v as i32;
        }
        if let Some(v) = get(&["search", "futility", "improving_bonus", "value"]) {
            self.futility_improving_bonus = v as i32;
        }
        if let Some(v) = get(&["search", "futility", "max_depth", "value"]) {
            self.futility_depth_limit = v as i32;
        }
        if let Some(v) = get(&["search", "lmp", "max_depth", "value"]) {
            self.lmp_depth_limit = v as i32;
        }
        if let Some(v) = get(&["search", "aspiration", "initial_delta", "value"]) {
            self.aspiration_window = v as i32;
        }
        if let Some(v) = get(&["search", "singular", "margin_multiplier", "value"]) {
            self.se_margin_mul = v as i32;
        }
        if let Some(v) = get(&["search", "singular", "min_depth", "value"]) {
            self.se_depth_min = v as i32;
        }
        if let Some(v) = get(&["search", "singular", "double_ext_margin", "value"]) {
            self.se_double_ext_margin = v as i32;
        }
        if let Some(v) = get(&["search", "singular", "low_depth_max", "value"]) {
            self.ldse_depth_max = (v as i32).max(0);
        }
        if let Some(v) = get(&["search", "singular", "low_depth_margin", "value"]) {
            self.ldse_margin = (v as i32).max(0);
        }
        self
    }

    /// Path used for optional TOML overrides (`MUJRIM_TUNING_FILE` or `sprt/params.toml`).
    #[must_use]
    pub fn default_tuning_file_path() -> PathBuf {
        std::env::var("MUJRIM_TUNING_FILE")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("sprt/params.toml"))
    }

    /// Apply [`Self::with_tuning_file`] using [`Self::default_tuning_file_path`].
    #[must_use]
    pub fn with_default_tuning_file(self) -> Self {
        self.with_tuning_file(&Self::default_tuning_file_path())
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

    /// History update learned from the static-evaluation swing across a quiet move.
    #[inline(always)]
    pub fn eval_history_bonus(&self, eval: i32, parent_eval: i32) -> i32 {
        (self.eval_history_scale * (-(eval + parent_eval)) / 128)
            .clamp(self.eval_history_min, self.eval_history_max)
    }

    /// Build the LMR reduction table from `lmr_base` and `lmr_divisor`.
    pub fn build_lmr_table(&self) -> Arc<[[i32; 128]; 128]> {
        let rows = vec![[0i32; 128]; 128].into_boxed_slice();
        let mut table: Box<[[i32; 128]; 128]> = rows
            .try_into()
            .expect("LMR table has the requested fixed dimensions");
        for (depth, row) in table.iter_mut().enumerate().skip(1) {
            for (moves, reduction) in row.iter_mut().enumerate().skip(1) {
                *reduction = (self.lmr_base
                    + (depth as f64).ln() * (moves as f64).ln() / self.lmr_divisor)
                    as i32;
            }
        }
        Arc::from(table)
    }
}

impl Default for SearchParams {
    fn default() -> Self {
        Self::for_preset_with_repo_tuning("akimbo")
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
        assert_eq!(p.null_move_r(10, 200, 100), 5 + 2); // (200-100)/200 = 0
        assert_eq!(p.lmp_threshold(3, false), (3 + 9) / 2); // 6
        assert_eq!(p.lmp_threshold(3, true), 3 + 9); // 12
    }

    #[test]
    fn test_stockfish_preset() {
        let p = SearchParams::stockfish();
        assert_eq!(p.nmp_base, 5);
        assert_eq!(p.nmp_depth_div, 3);
        assert_eq!(p.null_move_r(12, 300, 200), 5 + 4); // 9
        assert_eq!(p.futility_depth_limit, 8);
        assert_eq!(p.hist_prune_margin, -3826);
        assert_eq!(p.nmp_depth_min, 3);
        assert_eq!(p.lmp_depth_limit, 5);
        assert_eq!(p.se_depth_min, 5);
        assert_eq!(p.max_qs_ply, 16);
        assert_eq!(p.se_double_ext_margin, 22);
        assert_eq!(p.aspiration_window, 10);
    }

    #[test]
    fn reckless_preset_embeds_the_release_search_parameters() {
        let p = SearchParams::reckless();
        assert_eq!(p.nmp_eval_div, 200);
        assert_eq!(p.nmp_eval_max, 3);
        assert_eq!(p.lmr_base, 0.77);
        assert_eq!(p.lmr_divisor, 2.36);
        assert_eq!(p.hist_lmr_div, 4096);
        assert_eq!(p.hist_prune_depth_limit, 8);
        assert_eq!(p.futility_mul, 77);
        assert_eq!(p.futility_depth_limit, 6);
        assert_eq!(p.se_margin_mul, 2);
        assert_eq!(p.se_depth_min, 5);
        assert_eq!(p.ldse_depth_max, 7);
        assert_eq!(p.ldse_margin, 25);
        assert_eq!(p.nmp_depth_min, 2);
        assert_eq!(p.lmp_depth_limit, 7);
        assert_eq!(p.lmr_cut_node_bonus, 1);
        assert_eq!(p.max_qs_ply, 10);
        assert_eq!(p.aspiration_window, 10);
    }

    #[test]
    fn test_for_preset() {
        let akimbo = SearchParams::for_preset("akimbo");
        let sf = SearchParams::for_preset("stockfish");
        let reckless = SearchParams::for_preset("reckless");
        assert_eq!(akimbo.nmp_base, 5);
        assert_eq!(sf.nmp_base, 5);
        assert_eq!(sf.aspiration_window, 10);
        assert_eq!(reckless.aspiration_window, 10);

        // Unknown preset falls back to akimbo
        let unknown = SearchParams::for_preset("unknown");
        assert_eq!(unknown.nmp_base, 5);

        let viri = SearchParams::for_preset("viridithas");
        let obs = SearchParams::for_preset("obsidian");
        let plenty = SearchParams::for_preset("plentychess");
        let lc0 = SearchParams::for_preset("lc0");
        let hce = SearchParams::for_preset("mujrim-hce");
        assert_eq!(viri.lmr_cut_node_bonus, 1);
        assert_eq!(viri.lmr_base, 0.70);
        assert_eq!(viri.se_depth_min, 5);
        assert_eq!(obs.se_depth_min, 4);
        assert_eq!(obs.ldse_depth_max, 7);
        assert_eq!(akimbo.ldse_depth_max, 0);
        assert_eq!(obs.lmp_depth_limit, 5);
        assert_eq!(akimbo.se_depth_min, 4);
        assert_eq!(akimbo.lmp_depth_limit, 3);
        assert_eq!(akimbo.lmr_base, 0.30);
        assert_eq!(plenty.lmr_base, 0.62);
        assert_eq!(lc0.lmr_base, reckless.lmr_base);
        assert_eq!(lc0.lmr_cut_node_bonus, 0);
        assert_eq!(lc0.nmp_depth_min, 4);
        assert_eq!(lc0.lmp_depth_limit, 3);
        assert_eq!(lc0.max_qs_ply, 14);
        assert_eq!(hce.se_depth_min, 4);
        assert_eq!(hce.lmp_depth_limit, 1);
        assert_eq!(hce.nmp_depth_min, 8);
        assert_eq!(hce.futility_depth_limit, 2);
        assert_eq!(hce.hist_prune_depth_limit, 0);
        assert_eq!(hce.rfp_mul, 110);
    }

    #[test]
    fn for_preset_with_repo_tuning_matches_stockfish_without_overlay() {
        let a = SearchParams::for_preset_with_repo_tuning("stockfish");
        let b = SearchParams::stockfish();
        assert_eq!(a.nmp_base, b.nmp_base);
        assert_eq!(a.lmr_base, b.lmr_base);
    }

    #[test]
    fn test_lmr_table() {
        let p = SearchParams::akimbo();
        let table = p.build_lmr_table();
        // LMR[10][10] = floor(0.40 + ln(10)² / 2.48)
        assert_eq!(table[10][10], 2);
        assert_eq!(table[1][1], 0);
    }

    #[test]
    fn eval_history_bonus_tracks_and_clamps_eval_swings() {
        let p = SearchParams::akimbo();
        assert_eq!(p.eval_history_bonus(-100, 50), 317);
        assert_eq!(p.eval_history_bonus(-1_000, 0), p.eval_history_max);
        assert_eq!(p.eval_history_bonus(1_000, 0), p.eval_history_min);
    }
}
