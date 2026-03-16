//! The Arbiter: a thin wrapper around the search engine.
//! Provides a simple API for the CLI to search for moves.

use types::{Board, Move};
use search::SearchEngine;

pub struct Arbiter {
    engine: SearchEngine,
}

impl Default for Arbiter {
    fn default() -> Self {
        Self::new()
    }
}

impl Arbiter {
    pub fn new() -> Self {
        types::init();
        Self {
            engine: SearchEngine::new(64, 8),
        }
    }

    /// Searches for the best move at a given depth.
    pub fn best_move(&mut self, board: &mut Board, depth: i32) -> Move {
        let result = self.engine.search_depth(board, depth);
        result.best_move
    }

    /// Clears the engine state (for new game).
    pub fn new_game(&mut self) {
        self.engine.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arbiter_returns_legal_move() {
        types::init();
        let mut engine = SearchEngine::new(1, 1);
        let mut board = Board::new();
        let result = engine.search_depth(&mut board, 3);
        let mv = result.best_move;

        let legal = board.generate_legal_moves();
        assert!(
            legal.iter().any(|m| m.from == mv.from && m.to == mv.to),
            "Arbiter returned illegal move: {mv}"
        );
    }
}
