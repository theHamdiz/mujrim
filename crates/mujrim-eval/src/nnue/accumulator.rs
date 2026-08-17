//! NNUE Accumulator — manages the hidden layer state during search.
//!
//! Akimbo uses a per-ply stack with per-perspective Finny refresh. Reckless
//! and Stockfish keep their own format-specific stacks.

use super::adapter::{ActiveNetwork, NnueNetworkInfo, NnueNetworkParameters, NnueNetworkSource};
use super::akimbo_state::AkimboAccumulatorState;
use std::sync::Arc;
use types::{AkimboPos, Board, BoardSnapshot, Move};

/// Board bitmask state for accumulator cache validation.
pub use super::akimbo_state::EvalEntry;

/// NNUE state for use in the search.
pub struct NNUEState {
    akimbo: Option<AkimboAccumulatorState>,
    source: Arc<ActiveNetwork>,
    #[cfg(feature = "reckless-nnue")]
    reckless: Option<super::reckless_format::RecklessAccumulatorState>,
    #[cfg(feature = "stockfish-nnue")]
    stockfish: Option<super::stockfish_format::StockfishAccumulatorState>,
    #[cfg(feature = "obsidian-nnue")]
    obsidian: Option<super::obsidian_format::ObsidianAccumulatorState>,
    #[cfg(feature = "plentychess-nnue")]
    plentychess: Option<super::plentychess_format::PlentyChessAccumulatorState>,
    #[cfg(feature = "ateed-nnue")]
    ateed: Option<super::ateed_format::AteedAccumulatorState>,
    #[cfg(feature = "viridithas-nnue")]
    viridithas: Option<super::viridithas_format::ViridithasAccumulatorState>,
}

impl Default for NNUEState {
    fn default() -> Self {
        Self::new()
    }
}

impl NNUEState {
    pub fn new() -> Self {
        Self::with_network(Arc::new(super::adapter::default_embedded_network()))
    }

    pub fn with_network(source: Arc<ActiveNetwork>) -> Self {
        let parameters = source.parameters();
        let akimbo = matches!(parameters, NnueNetworkParameters::Akimbo(_))
            .then(AkimboAccumulatorState::new);
        #[cfg(feature = "reckless-nnue")]
        let reckless = matches!(parameters, NnueNetworkParameters::Reckless(_))
            .then(super::reckless_format::RecklessAccumulatorState::new);
        #[cfg(feature = "stockfish-nnue")]
        let stockfish = matches!(parameters, NnueNetworkParameters::Stockfish(_))
            .then(super::stockfish_format::StockfishAccumulatorState::new);
        #[cfg(feature = "obsidian-nnue")]
        let obsidian = matches!(parameters, NnueNetworkParameters::Obsidian(_))
            .then(super::obsidian_format::ObsidianAccumulatorState::new);
        #[cfg(feature = "plentychess-nnue")]
        let plentychess = matches!(parameters, NnueNetworkParameters::PlentyChess(_))
            .then(super::plentychess_format::PlentyChessAccumulatorState::new);
        #[cfg(feature = "ateed-nnue")]
        let ateed = matches!(parameters, NnueNetworkParameters::Ateed(_))
            .then(super::ateed_format::AteedAccumulatorState::new);
        #[cfg(feature = "viridithas-nnue")]
        let viridithas = match parameters {
            NnueNetworkParameters::Viridithas(net) => {
                Some(super::viridithas_format::ViridithasAccumulatorState::for_network(net))
            }
            _ => None,
        };
        Self {
            akimbo,
            source,
            #[cfg(feature = "reckless-nnue")]
            reckless,
            #[cfg(feature = "stockfish-nnue")]
            stockfish,
            #[cfg(feature = "obsidian-nnue")]
            obsidian,
            #[cfg(feature = "plentychess-nnue")]
            plentychess,
            #[cfg(feature = "ateed-nnue")]
            ateed,
            #[cfg(feature = "viridithas-nnue")]
            viridithas,
        }
    }

    pub fn network_info(&self) -> NnueNetworkInfo {
        self.source.info()
    }

    /// Start an accumulator frame for a real search move.
    #[inline]
    pub fn push_move(&mut self, board: &Board, mv: Move) {
        if let Some(state) = &mut self.akimbo {
            state.push_move(board, mv);
            return;
        }
        #[cfg(feature = "reckless-nnue")]
        if let Some(state) = &mut self.reckless {
            state.push_move(board, mv);
            return;
        }
        #[cfg(feature = "stockfish-nnue")]
        if let Some(state) = &mut self.stockfish {
            state.push_move(board, mv);
            return;
        }
        #[cfg(feature = "obsidian-nnue")]
        if let Some(state) = &mut self.obsidian {
            state.push_move(board, mv);
            #[cfg(any(
                feature = "plentychess-nnue",
                feature = "ateed-nnue",
                feature = "viridithas-nnue"
            ))]
            return;
        }
        #[cfg(feature = "plentychess-nnue")]
        if let Some(state) = &mut self.plentychess {
            state.push_move(board, mv);
            return;
        }
        #[cfg(feature = "ateed-nnue")]
        if let Some(state) = &mut self.ateed {
            state.push_move(board, mv);
            return;
        }
        #[cfg(feature = "viridithas-nnue")]
        if let Some(state) = &mut self.viridithas {
            state.push_move(board, mv);
        }
    }

    /// Applies a search move through the evaluator-specific update path.
    #[inline]
    pub fn make_move(&mut self, board: &mut Board, mv: Move) {
        self.apply_move(board, mv, true);
    }

    pub fn make_move_without_undo(&mut self, board: &mut Board, mv: Move) {
        self.apply_move(board, mv, false);
    }

    #[inline]
    pub fn push_move_pos(&mut self, pos: &AkimboPos, mv: Move) {
        if let Some(state) = &mut self.akimbo {
            state.push_move_pos(pos, mv);
        }
    }

    #[inline]
    pub fn push_move_snap(&mut self, pos: &BoardSnapshot, mv: Move) {
        if let Some(state) = &mut self.akimbo {
            state.push_move_snap(pos, mv);
        }
    }

    fn apply_move(&mut self, board: &mut Board, mv: Move, record_undo: bool) {
        let make = |board: &mut Board, mv: Move| {
            if record_undo {
                board.make_move(mv);
            } else {
                board.make_move_without_undo(mv);
            }
        };
        #[cfg(feature = "reckless-nnue")]
        if let Some(state) = &mut self.reckless {
            state.push_move_observed(board, mv);
            if record_undo {
                board.make_move_observed(mv, state);
            } else {
                board.make_move_without_undo(mv);
            }
            return;
        }
        if let Some(state) = &mut self.akimbo {
            state.push_move(board, mv);
            make(board, mv);
            return;
        }
        #[cfg(feature = "stockfish-nnue")]
        if let Some(state) = &mut self.stockfish {
            state.push_move(board, mv);
            make(board, mv);
            return;
        }
        #[cfg(feature = "obsidian-nnue")]
        if let Some(state) = &mut self.obsidian {
            state.push_move(board, mv);
            make(board, mv);
            return;
        }
        #[cfg(feature = "plentychess-nnue")]
        if let Some(state) = &mut self.plentychess {
            state.push_move(board, mv);
            make(board, mv);
            return;
        }
        #[cfg(feature = "ateed-nnue")]
        if let Some(state) = &mut self.ateed {
            state.push_move(board, mv);
            make(board, mv);
            return;
        }
        #[cfg(feature = "viridithas-nnue")]
        if let Some(state) = &mut self.viridithas {
            state.push_move(board, mv);
        }
        make(board, mv);
    }

    /// Official `hint_common_access`: apply pending FT/aux after a TT miss.
    /// TT-cut children skip this so they do not pay sandhi aux on a cutoff.
    #[inline]
    pub fn hint_common_access(&mut self, board: &Board) {
        #[cfg(feature = "viridithas-nnue")]
        self.ensure_viridithas_after_make(board);
        #[cfg(feature = "ateed-nnue")]
        self.ensure_ateed_after_make(board);
        #[cfg(not(any(feature = "viridithas-nnue", feature = "ateed-nnue")))]
        let _ = board;
    }

    #[cfg(feature = "viridithas-nnue")]
    fn ensure_viridithas_after_make(&mut self, board: &Board) {
        let Some(state) = self.viridithas.as_mut() else {
            return;
        };
        if let super::adapter::NnueNetworkParameters::Viridithas(net) = self.source.parameters() {
            state.ensure_after_make(board, net);
        }
    }

    #[cfg(feature = "ateed-nnue")]
    fn ensure_ateed_after_make(&mut self, board: &Board) {
        let Some(state) = self.ateed.as_mut() else {
            return;
        };
        if let super::adapter::NnueNetworkParameters::Ateed(net) = self.source.parameters() {
            state.ensure_after_make(board, net);
        }
    }

    /// Start an accumulator frame for a null move.
    #[inline]
    pub fn push_null(&mut self) {
        if let Some(state) = &mut self.akimbo {
            state.push_null();
            return;
        }
        #[cfg(feature = "reckless-nnue")]
        if let Some(state) = &mut self.reckless {
            state.push_null();
            return;
        }
        #[cfg(feature = "stockfish-nnue")]
        if let Some(state) = &mut self.stockfish {
            state.push_null();
            return;
        }
        #[cfg(feature = "obsidian-nnue")]
        if let Some(state) = &mut self.obsidian {
            state.push_null();
            #[cfg(any(
                feature = "plentychess-nnue",
                feature = "ateed-nnue",
                feature = "viridithas-nnue"
            ))]
            return;
        }
        #[cfg(feature = "plentychess-nnue")]
        if let Some(state) = &mut self.plentychess {
            state.push_null();
            return;
        }
        #[cfg(feature = "ateed-nnue")]
        if let Some(state) = &mut self.ateed {
            state.push_null();
            return;
        }
        #[cfg(feature = "viridithas-nnue")]
        if let Some(state) = &mut self.viridithas {
            state.push_null();
        }
    }

    /// Restore the parent accumulator frame after unmaking a search move.
    #[inline]
    pub fn pop_move(&mut self) {
        if let Some(state) = &mut self.akimbo {
            state.pop();
            return;
        }
        #[cfg(feature = "reckless-nnue")]
        if let Some(state) = &mut self.reckless {
            state.pop();
            return;
        }
        #[cfg(feature = "stockfish-nnue")]
        if let Some(state) = &mut self.stockfish {
            state.pop();
            return;
        }
        #[cfg(feature = "obsidian-nnue")]
        if let Some(state) = &mut self.obsidian {
            state.pop();
            #[cfg(any(
                feature = "plentychess-nnue",
                feature = "ateed-nnue",
                feature = "viridithas-nnue"
            ))]
            return;
        }
        #[cfg(feature = "plentychess-nnue")]
        if let Some(state) = &mut self.plentychess {
            state.pop();
            return;
        }
        #[cfg(feature = "ateed-nnue")]
        if let Some(state) = &mut self.ateed {
            state.pop();
            return;
        }
        #[cfg(feature = "viridithas-nnue")]
        if let Some(state) = &mut self.viridithas {
            state.pop();
        }
    }

    /// Evaluate the position using NNUE with incremental accumulator updates.
    pub fn evaluate(&mut self, board: &Board) -> i32 {
        match self.source.as_ref() {
            #[cfg(feature = "reckless-nnue")]
            ActiveNetwork::EmbeddedReckless => {
                return self
                    .reckless
                    .as_mut()
                    .expect("Reckless source has matching accumulator state")
                    .evaluate(board, super::reckless_format::embedded());
            }
            #[cfg(feature = "reckless-nnue")]
            ActiveNetwork::ExternalReckless { network, .. } => {
                return self
                    .reckless
                    .as_mut()
                    .expect("Reckless source has matching accumulator state")
                    .evaluate(board, network);
            }
            #[cfg(feature = "stockfish-nnue")]
            ActiveNetwork::EmbeddedStockfish => {
                return self
                    .stockfish
                    .as_mut()
                    .expect("Stockfish source has matching accumulator state")
                    .evaluate(board, super::stockfish_format::embedded());
            }
            #[cfg(feature = "stockfish-nnue")]
            ActiveNetwork::ExternalStockfish { network, .. } => {
                return self
                    .stockfish
                    .as_mut()
                    .expect("Stockfish source has matching accumulator state")
                    .evaluate(board, network);
            }
            ActiveNetwork::Embedded => {}
            ActiveNetwork::ExternalAkimbo { .. } => {}
            #[cfg(feature = "viridithas-nnue")]
            ActiveNetwork::ExternalViridithas { network, .. } => {
                return self
                    .viridithas
                    .as_mut()
                    .expect("Viridithas source has matching accumulator state")
                    .evaluate(board, network);
            }
            #[cfg(feature = "obsidian-nnue")]
            ActiveNetwork::ExternalObsidian { network, .. } => {
                return self
                    .obsidian
                    .as_mut()
                    .expect("Obsidian source has matching accumulator state")
                    .evaluate(board, network);
            }
            #[cfg(feature = "plentychess-nnue")]
            ActiveNetwork::ExternalPlentyChess { network, .. } => {
                return self
                    .plentychess
                    .as_mut()
                    .expect("PlentyChess source has matching accumulator state")
                    .evaluate(board, network);
            }
            #[cfg(feature = "ateed-nnue")]
            ActiveNetwork::ExternalAteed { network, .. } => {
                return self
                    .ateed
                    .as_mut()
                    .expect("Ateed source has matching accumulator state")
                    .evaluate(board, network);
            }
        }

        let net = match self.source.parameters() {
            NnueNetworkParameters::Akimbo(net) => net,
            #[cfg(any(
                feature = "stockfish-nnue",
                feature = "reckless-nnue",
                feature = "viridithas-nnue",
                feature = "obsidian-nnue",
                feature = "plentychess-nnue",
                feature = "ateed-nnue"
            ))]
            _ => unreachable!("non-Akimbo backends are handled above"),
        };
        self.akimbo
            .as_mut()
            .expect("Akimbo source has matching accumulator state")
            .evaluate(board, net)
    }

    /// Search-only evaluate: skip the ply-hash probe when the current frame is
    /// already accurate. Bench / scratch callers must keep using [`Self::evaluate`].
    pub fn evaluate_search(&mut self, board: &Board) -> i32 {
        match self.source.as_ref() {
            #[cfg(feature = "reckless-nnue")]
            ActiveNetwork::EmbeddedReckless => {
                return self
                    .reckless
                    .as_mut()
                    .expect("Reckless source has matching accumulator state")
                    .evaluate_search(board, super::reckless_format::embedded());
            }
            #[cfg(feature = "reckless-nnue")]
            ActiveNetwork::ExternalReckless { network, .. } => {
                return self
                    .reckless
                    .as_mut()
                    .expect("Reckless source has matching accumulator state")
                    .evaluate_search(board, network);
            }
            #[cfg(feature = "stockfish-nnue")]
            ActiveNetwork::EmbeddedStockfish => {
                return self
                    .stockfish
                    .as_mut()
                    .expect("Stockfish source has matching accumulator state")
                    .evaluate_search(board, super::stockfish_format::embedded());
            }
            #[cfg(feature = "stockfish-nnue")]
            ActiveNetwork::ExternalStockfish { network, .. } => {
                return self
                    .stockfish
                    .as_mut()
                    .expect("Stockfish source has matching accumulator state")
                    .evaluate_search(board, network);
            }
            ActiveNetwork::Embedded => {}
            ActiveNetwork::ExternalAkimbo { .. } => {}
            #[cfg(feature = "viridithas-nnue")]
            ActiveNetwork::ExternalViridithas { network, .. } => {
                return self
                    .viridithas
                    .as_mut()
                    .expect("Viridithas source has matching accumulator state")
                    .evaluate_search(board, network);
            }
            #[cfg(feature = "obsidian-nnue")]
            ActiveNetwork::ExternalObsidian { network, .. } => {
                return self
                    .obsidian
                    .as_mut()
                    .expect("Obsidian source has matching accumulator state")
                    .evaluate_search(board, network);
            }
            #[cfg(feature = "plentychess-nnue")]
            ActiveNetwork::ExternalPlentyChess { network, .. } => {
                return self
                    .plentychess
                    .as_mut()
                    .expect("PlentyChess source has matching accumulator state")
                    .evaluate_search(board, network);
            }
            #[cfg(feature = "ateed-nnue")]
            ActiveNetwork::ExternalAteed { network, .. } => {
                return self
                    .ateed
                    .as_mut()
                    .expect("Ateed source has matching accumulator state")
                    .evaluate_search(board, network);
            }
        }

        let net = match self.source.parameters() {
            NnueNetworkParameters::Akimbo(net) => net,
            #[cfg(any(
                feature = "stockfish-nnue",
                feature = "reckless-nnue",
                feature = "viridithas-nnue",
                feature = "obsidian-nnue",
                feature = "plentychess-nnue",
                feature = "ateed-nnue"
            ))]
            _ => unreachable!("non-Akimbo backends are handled above"),
        };
        self.akimbo
            .as_mut()
            .expect("Akimbo source has matching accumulator state")
            .evaluate_search(board, net)
    }

    /// Official Akimbo Finny-from-pos evaluate. Dedicated loop only.
    pub fn evaluate_search_pos(&mut self, pos: &AkimboPos) -> i32 {
        let net = match self.source.parameters() {
            NnueNetworkParameters::Akimbo(net) => net,
            #[cfg(any(
                feature = "stockfish-nnue",
                feature = "reckless-nnue",
                feature = "viridithas-nnue",
                feature = "obsidian-nnue",
                feature = "plentychess-nnue",
                feature = "ateed-nnue"
            ))]
            _ => unreachable!("evaluate_search_pos is Akimbo-only"),
        };
        self.akimbo
            .as_mut()
            .expect("Akimbo source has matching accumulator state")
            .evaluate_search_pos(pos, net)
    }

    /// Ply-stack evaluate from a mailbox snapshot. Dedicated Akimbo loop only.
    pub fn evaluate_search_snap(&mut self, pos: &BoardSnapshot) -> i32 {
        let net = match self.source.parameters() {
            NnueNetworkParameters::Akimbo(net) => net,
            #[cfg(any(
                feature = "stockfish-nnue",
                feature = "reckless-nnue",
                feature = "viridithas-nnue",
                feature = "obsidian-nnue",
                feature = "plentychess-nnue",
                feature = "ateed-nnue"
            ))]
            _ => unreachable!("evaluate_search_snap is Akimbo-only"),
        };
        self.akimbo
            .as_mut()
            .expect("Akimbo source has matching accumulator state")
            .evaluate_search_snap(pos, net)
    }

    /// Score plus a non-negative uncertainty proxy. Only Ateed fills WDL variance;
    /// other evaluators return `0`.
    pub fn evaluate_with_uncertainty(&mut self, board: &Board) -> (i32, i32) {
        #[cfg(feature = "ateed-nnue")]
        if let ActiveNetwork::ExternalAteed { network, .. } = self.source.as_ref() {
            let eval = self
                .ateed
                .as_mut()
                .expect("Ateed source has matching accumulator state")
                .evaluate_full(board, network);
            return (eval.score, super::ateed_format::wdl_variance(eval.wdl));
        }
        (self.evaluate(board), 0)
    }

    pub fn evaluate_with_uncertainty_search(&mut self, board: &Board) -> (i32, i32) {
        #[cfg(feature = "ateed-nnue")]
        if let Some(signal) = self.evaluate_ateed_search_signal(board) {
            return (signal.score, signal.variance);
        }
        (self.evaluate_search(board), 0)
    }

    #[cfg(feature = "ateed-nnue")]
    pub fn evaluate_ateed_search_signal(
        &mut self,
        board: &Board,
    ) -> Option<super::ateed_format::AteedSearchSignal> {
        let ActiveNetwork::ExternalAteed { network, .. } = self.source.as_ref() else {
            return None;
        };
        let eval = self
            .ateed
            .as_mut()
            .expect("Ateed source has matching accumulator state")
            .evaluate_full_search(board, network);
        Some(eval.search_signal())
    }

    #[cfg(feature = "ateed-nnue")]
    pub fn last_ateed_search_signal(&self) -> Option<super::ateed_format::AteedSearchSignal> {
        self.ateed
            .as_ref()
            .map(super::ateed_format::AteedAccumulatorState::last_search_signal)
    }

    #[cfg(feature = "ateed-nnue")]
    pub fn cached_ateed_search_signal(
        &self,
        board: &Board,
    ) -> Option<super::ateed_format::AteedSearchSignal> {
        self.ateed
            .as_ref()
            .and_then(|state| state.cached_search_signal(board.hash))
    }

    /// Fully recompute the accumulators from a board position.
    pub fn reinit_from(&mut self, board: &Board) {
        match self.source.as_ref() {
            #[cfg(feature = "stockfish-nnue")]
            ActiveNetwork::EmbeddedStockfish => {
                let state = self
                    .stockfish
                    .as_mut()
                    .expect("Stockfish source has matching accumulator state");
                state.clear();
                let _ = state.evaluate(board, super::stockfish_format::embedded());
                return;
            }
            #[cfg(feature = "stockfish-nnue")]
            ActiveNetwork::ExternalStockfish { network, .. } => {
                let state = self
                    .stockfish
                    .as_mut()
                    .expect("Stockfish source has matching accumulator state");
                state.clear();
                let _ = state.evaluate(board, network);
                return;
            }
            #[cfg(feature = "reckless-nnue")]
            ActiveNetwork::EmbeddedReckless => {
                let state = self
                    .reckless
                    .as_mut()
                    .expect("Reckless source has matching accumulator state");
                state.clear();
                let _ = state.evaluate(board, super::reckless_format::embedded());
                return;
            }
            #[cfg(feature = "reckless-nnue")]
            ActiveNetwork::ExternalReckless { network, .. } => {
                let state = self
                    .reckless
                    .as_mut()
                    .expect("Reckless source has matching accumulator state");
                state.clear();
                let _ = state.evaluate(board, network);
                return;
            }
            ActiveNetwork::Embedded | ActiveNetwork::ExternalAkimbo { .. } => {}
            #[cfg(feature = "viridithas-nnue")]
            ActiveNetwork::ExternalViridithas { network, .. } => {
                let state = self
                    .viridithas
                    .as_mut()
                    .expect("Viridithas source has matching accumulator state");
                state.clear();
                let _ = state.evaluate(board, network);
                return;
            }
            #[cfg(feature = "obsidian-nnue")]
            ActiveNetwork::ExternalObsidian { network, .. } => {
                let state = self
                    .obsidian
                    .as_mut()
                    .expect("Obsidian source has matching accumulator state");
                state.clear();
                let _ = state.evaluate(board, network);
                return;
            }
            #[cfg(feature = "plentychess-nnue")]
            ActiveNetwork::ExternalPlentyChess { network, .. } => {
                let state = self
                    .plentychess
                    .as_mut()
                    .expect("PlentyChess source has matching accumulator state");
                state.clear();
                let _ = state.evaluate(board, network);
                return;
            }
            #[cfg(feature = "ateed-nnue")]
            ActiveNetwork::ExternalAteed { network, .. } => {
                let state = self
                    .ateed
                    .as_mut()
                    .expect("Ateed source has matching accumulator state");
                state.clear();
                let _ = state.evaluate(board, network);
                return;
            }
        }

        let net = match self.source.parameters() {
            NnueNetworkParameters::Akimbo(net) => net,
            #[cfg(any(
                feature = "stockfish-nnue",
                feature = "reckless-nnue",
                feature = "viridithas-nnue",
                feature = "obsidian-nnue",
                feature = "plentychess-nnue",
                feature = "ateed-nnue"
            ))]
            _ => unreachable!("non-Akimbo backends are handled above"),
        };
        let state = self
            .akimbo
            .as_mut()
            .expect("Akimbo source has matching accumulator state");
        state.clear();
        let _ = state.evaluate(board, net);
    }

    /// Get the current accumulator entry for the given king positions.
    pub fn get_entry(&mut self, _w_king: usize, _b_king: usize) -> &EvalEntry {
        let state = self
            .akimbo
            .as_mut()
            .expect("Akimbo source has matching accumulator state");
        state.sync_current_from_frame();
        state.current_entry()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Board, Color, Move, Square};

    fn akimbo_state() -> NNUEState {
        NNUEState::with_network(Arc::new(ActiveNetwork::Embedded))
    }

    fn scratch_eval(board: &Board) -> i32 {
        let mut state = akimbo_state();
        state.evaluate(board)
    }

    #[test]
    fn test_reinit_no_panic() {
        types::init();
        let board = Board::new();
        let mut state = NNUEState::new();
        state.reinit_from(&board);
    }

    #[test]
    fn test_accumulator_not_all_zeros_after_reinit() {
        types::init();
        let board = Board::new();
        let mut state = NNUEState::with_network(Arc::new(ActiveNetwork::Embedded));
        state.reinit_from(&board);
        let w_king = board.king_square(Color::White).index();
        let b_king = board.king_square(Color::Black).index();
        let entry = state.get_entry(w_king, b_king);
        let sum: i64 = entry.white.vals.iter().map(|&v| v as i64).sum();
        assert!(sum != 0, "White accumulator is all zeros after reinit");
    }

    #[test]
    fn test_evaluate_returns_reasonable() {
        types::init();
        let board = Board::new();
        let mut state = NNUEState::new();
        let score = state.evaluate(&board);
        assert!(
            score.abs() < 200,
            "Starting position eval {score} seems unreasonable"
        );
    }

    #[cfg(feature = "reckless-nnue")]
    #[test]
    fn reckless_evaluate_uses_concrete_active_network() {
        types::init();
        let mut state = NNUEState::with_network(Arc::new(ActiveNetwork::EmbeddedReckless));
        let score = state.evaluate(&Board::new());
        assert!(score.abs() < 500);
    }

    #[cfg(feature = "ateed-nnue")]
    #[test]
    fn ateed_evaluate_with_uncertainty_reports_wdl_variance() {
        types::init();
        let path = std::env::temp_dir().join("mujrim-ateed-uncertainty.bin");
        std::fs::write(
            &path,
            super::super::ateed_format::AteedNetwork::zero().to_bytes(),
        )
        .unwrap();
        let source = Arc::new(super::super::adapter::load_network(&path).expect("zero Ateed net"));
        let _ = std::fs::remove_file(&path);
        let mut state = NNUEState::with_network(source);
        let (score, variance) = state.evaluate_with_uncertainty(&Board::new());
        assert_eq!(score, 0);
        assert!(variance >= 0);
        assert_eq!(state.evaluate(&Board::new()), score);
    }

    #[cfg(feature = "reckless-nnue")]
    #[test]
    fn reckless_backend_does_not_allocate_an_akimbo_table() {
        let state = NNUEState::with_network(Arc::new(ActiveNetwork::EmbeddedReckless));
        assert!(state.akimbo.is_none());
        assert!(state.reckless.is_some());
    }

    #[test]
    fn test_evaluate_caching() {
        types::init();
        let board = Board::new();
        let mut state = NNUEState::new();
        let score1 = state.evaluate(&board);
        let score2 = state.evaluate(&board);
        assert_eq!(score1, score2, "Cached evaluation should be identical");
    }

    #[test]
    fn test_evaluate_different_positions() {
        types::init();
        let board1 = Board::new();
        let board2 =
            Board::from_fen("rnb1kbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let mut state = NNUEState::new();
        let score1 = state.evaluate(&board1);
        let score2 = state.evaluate(&board2);
        assert!(
            score2 > score1,
            "Missing queen should increase eval: start={score1}, missing_q={score2}"
        );
    }

    #[test]
    fn test_material_scaling() {
        types::init();
        let mg = Board::new();
        let eg = Board::from_fen("4k3/pppppppp/8/8/8/8/PPPPPPPP/4K3 w - - 0 1").unwrap();
        let mg_scale = super::super::akimbo_state::material_scale(&mg);
        let eg_scale = super::super::akimbo_state::material_scale(&eg);
        assert!(
            mg_scale > eg_scale,
            "Middlegame should have higher scale: mg={mg_scale}, eg={eg_scale}"
        );
    }

    #[test]
    fn test_incremental_update_consistency() {
        types::init();
        let board1 = Board::new();
        let board2 =
            Board::from_fen("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1").unwrap();

        let mut state1 = akimbo_state();
        let _ = state1.evaluate(&board1);
        let score_incremental = state1.evaluate(&board2);

        let mut state2 = akimbo_state();
        let score_scratch = state2.evaluate(&board2);

        assert_eq!(
            score_incremental, score_scratch,
            "Incremental ({score_incremental}) vs scratch ({score_scratch}) mismatch"
        );
    }

    #[test]
    fn king_move_incremental_matches_full_refresh() {
        types::init();
        let board_ka1 = Board::from_fen("4k3/8/8/8/8/8/8/K7 w - - 0 1").unwrap();
        let board_kb1 = Board::from_fen("4k3/8/8/8/8/8/8/1K6 w - - 0 1").unwrap();

        let mut warm = akimbo_state();
        let _ = warm.evaluate(&board_ka1);
        let after_king_move = warm.evaluate(&board_kb1);

        let mut fresh = akimbo_state();
        let from_scratch = fresh.evaluate(&board_kb1);

        assert_eq!(
            after_king_move, from_scratch,
            "After king move, NNUE must match full refresh: warm={after_king_move}, scratch={from_scratch}"
        );
    }

    #[test]
    fn ply_stack_e2e4_e7e5_matches_scratch_and_restores_parent() {
        types::init();
        let start = Board::new();
        let e2e4 = Move::double_pawn(Square::E2, Square::E4);
        let e7e5 = Move::double_pawn(Square::E7, Square::E5);

        let mut board = start;
        let mut state = akimbo_state();
        let start_score = state.evaluate(&board);
        assert_eq!(start_score, scratch_eval(&board));

        state.push_move(&board, e2e4);
        board.make_move(e2e4);
        let after_e4 = state.evaluate(&board);
        assert_eq!(after_e4, scratch_eval(&board));

        state.push_move(&board, e7e5);
        board.make_move(e7e5);
        let after_e5 = state.evaluate(&board);
        assert_eq!(after_e5, scratch_eval(&board));

        board.unmake_move(e7e5);
        state.pop_move();
        assert_eq!(state.evaluate(&board), after_e4);

        board.unmake_move(e2e4);
        state.pop_move();
        assert_eq!(state.evaluate(&board), start_score);
        assert_eq!(
            state
                .akimbo
                .as_ref()
                .map(AkimboAccumulatorState::stack_index),
            Some(0)
        );
    }

    #[test]
    fn same_bucket_king_walk_matches_scratch() {
        types::init();
        let ka1 = Board::from_fen("4k3/8/8/8/8/8/8/K7 w - - 0 1").unwrap();
        let kb1 = Board::from_fen("4k3/8/8/8/8/8/8/1K6 w - - 0 1").unwrap();
        let mut state = akimbo_state();
        let _ = state.evaluate(&ka1);
        assert_eq!(state.evaluate(&kb1), scratch_eval(&kb1));
    }

    #[test]
    fn bucket_change_king_move_matches_scratch() {
        types::init();
        let ke1 = Board::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let kg1 = Board::from_fen("4k3/8/8/8/8/8/8/6K1 w - - 0 1").unwrap();
        let mut state = akimbo_state();
        let _ = state.evaluate(&ke1);
        assert_eq!(state.evaluate(&kg1), scratch_eval(&kg1));
    }

    #[test]
    fn capture_promotion_castle_ep_match_scratch() {
        types::init();
        let cases = [
            (
                "rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2",
                Move::capture(Square::E4, Square::D5),
            ),
            (
                "4k3/P7/8/8/8/8/8/4K3 w - - 0 1",
                Move::promotion(Square::A7, Square::A8, types::Piece::Queen),
            ),
            (
                "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1",
                Move::king_castle(Square::E1, Square::G1),
            ),
            (
                "4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1",
                Move::en_passant(Square::E5, Square::D6),
            ),
        ];
        for (fen, mv) in cases {
            let mut board = Board::from_fen(fen).unwrap();
            let mut state = akimbo_state();
            let _ = state.evaluate(&board);
            state.push_move(&board, mv);
            board.make_move(mv);
            assert_eq!(
                state.evaluate(&board),
                scratch_eval(&board),
                "mismatch after {}",
                mv.to_uci()
            );
        }
    }

    #[test]
    fn null_move_push_pop_preserves_eval() {
        types::init();
        let mut board = Board::new();
        let mut state = akimbo_state();
        let before = state.evaluate(&board);
        state.push_null();
        board.make_null_move();
        let after_null = state.evaluate(&board);
        board.unmake_null_move();
        state.pop_move();
        assert_eq!(state.evaluate(&board), before);
        assert_ne!(after_null, i32::MIN);
    }

    #[test]
    fn akimbo_state_starts_at_ply_zero() {
        let state = akimbo_state();
        assert_eq!(
            state
                .akimbo
                .as_ref()
                .map(AkimboAccumulatorState::stack_index),
            Some(0)
        );
    }

    #[test]
    fn nnue_weight_diagnostic() {
        types::init();
        let net = super::super::network::net();

        let bias = &net.feature_bias.vals;
        let bias_nz = bias.iter().filter(|&&v| v != 0).count();
        eprintln!(
            "Feature bias: nonzero={}/{}, range=[{}, {}]",
            bias_nz,
            bias.len(),
            bias.iter().min().unwrap(),
            bias.iter().max().unwrap()
        );

        let ow0 = &net.output_weights[0].vals;
        let ow1 = &net.output_weights[1].vals;
        eprintln!(
            "Output weights[0]: nonzero={}/{}, range=[{}, {}]",
            ow0.iter().filter(|&&v| v != 0).count(),
            ow0.len(),
            ow0.iter().min().unwrap(),
            ow0.iter().max().unwrap()
        );
        eprintln!(
            "Output weights[1]: nonzero={}/{}, range=[{}, {}]",
            ow1.iter().filter(|&&v| v != 0).count(),
            ow1.len(),
            ow1.iter().min().unwrap(),
            ow1.iter().max().unwrap()
        );
        eprintln!("Output bias: {}", net.output_bias);

        let mut state = NNUEState::new();
        let board = Board::new();
        let s1 = state.evaluate(&board);
        eprintln!("Starting pos: {} cp", s1);

        let b2 =
            Board::from_fen("rnb1kbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let s2 = state.evaluate(&b2);
        eprintln!("W up queen: {} cp", s2);

        let b3 =
            Board::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNB1KBNR w Qkq - 0 1").unwrap();
        let s3 = state.evaluate(&b3);
        eprintln!("W down queen: {} cp", s3);

        let bk1 = Board::from_fen("1k1r4/pp1b1R2/3q2pp/4p3/2B5/4Q3/PPP2B2/2K5 b - - 0 1").unwrap();
        let bk1s = state.evaluate(&bk1);
        eprintln!("BK #1 (black, exp d6d1): {} cp", bk1s);

        assert!(s2 > s1, "White up queen ({s2}) should be > start ({s1})");
        assert!(s3 < s1, "White down queen ({s3}) should be < start ({s1})");
    }
}
