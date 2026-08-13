//! Opening repertoire and explorer models backed by legal UCI move lines.

use std::collections::BTreeMap;

pub const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepertoireLine {
    pub name: String,
    pub moves: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MoveStatistics {
    pub games: u64,
    pub white_wins: u64,
    pub draws: u64,
    pub black_wins: u64,
}

#[derive(Clone, Debug, Default)]
pub struct OpeningExplorer {
    positions: BTreeMap<String, BTreeMap<String, MoveStatistics>>,
}

impl OpeningExplorer {
    pub fn record_game(
        &mut self,
        initial_fen: &str,
        moves: &[String],
        result: &str,
    ) -> Result<(), String> {
        types::init();
        let mut board = types::Board::from_fen(initial_fen)?;
        for uci in moves {
            let position = board.to_fen();
            let mv = resolve_uci(&mut board, uci)
                .ok_or_else(|| format!("illegal opening move '{uci}' in '{position}'"))?;
            let statistics = self
                .positions
                .entry(position)
                .or_default()
                .entry(uci.clone())
                .or_default();
            statistics.games += 1;
            match result {
                "1-0" => statistics.white_wins += 1,
                "0-1" => statistics.black_wins += 1,
                "1/2-1/2" => statistics.draws += 1,
                _ => {}
            }
            board.make_move(mv);
        }
        Ok(())
    }

    pub fn moves(&self, fen: &str) -> Vec<(&str, &MoveStatistics)> {
        let mut moves = self
            .positions
            .get(fen)
            .into_iter()
            .flat_map(|entries| entries.iter())
            .map(|(mv, statistics)| (mv.as_str(), statistics))
            .collect::<Vec<_>>();
        moves.sort_by(|left, right| {
            right
                .1
                .games
                .cmp(&left.1.games)
                .then_with(|| left.0.cmp(right.0))
        });
        moves
    }
}

impl RepertoireLine {
    pub fn validate(&self, initial_fen: &str) -> Result<(), String> {
        types::init();
        let mut board = types::Board::from_fen(initial_fen)?;
        for (ply, uci) in self.moves.iter().enumerate() {
            let mv = resolve_uci(&mut board, uci)
                .ok_or_else(|| format!("illegal move '{uci}' at ply {}", ply + 1))?;
            board.make_move(mv);
        }
        Ok(())
    }
}

fn resolve_uci(board: &mut types::Board, uci: &str) -> Option<types::Move> {
    board
        .generate_legal_moves()
        .into_iter()
        .find(|mv| mv.to_uci() == uci)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repertoire_rejects_illegal_lines() {
        let valid = RepertoireLine {
            name: "Ruy Lopez".to_owned(),
            moves: ["e2e4", "e7e5", "g1f3", "b8c6", "f1b5"]
                .map(str::to_owned)
                .to_vec(),
        };
        assert!(valid.validate(START_FEN).is_ok());
        let invalid = RepertoireLine {
            name: "Invalid".to_owned(),
            moves: vec!["e2e5".to_owned()],
        };
        assert!(invalid.validate(START_FEN).is_err());
    }

    #[test]
    fn explorer_orders_moves_by_database_frequency() {
        let mut explorer = OpeningExplorer::default();
        explorer
            .record_game(START_FEN, &["e2e4".to_owned()], "1-0")
            .unwrap();
        explorer
            .record_game(START_FEN, &["d2d4".to_owned()], "1/2-1/2")
            .unwrap();
        explorer
            .record_game(START_FEN, &["e2e4".to_owned()], "0-1")
            .unwrap();
        let moves = explorer.moves(START_FEN);
        assert_eq!(moves[0].0, "e2e4");
        assert_eq!(moves[0].1.games, 2);
        assert_eq!(moves[1].1.draws, 1);
    }
}
