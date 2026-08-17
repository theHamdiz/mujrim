//! Deduped Ateed training-position index for datagen and tournament games.

use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use types::{Board, Move};

use crate::durable;
use crate::tournament_store::StoredTournamentGame;

const INDEX_HEADER: &str = "ateed-index-v1";

/// Board identity without move clocks so transpositions collapse.
pub fn position_key(fen: &str) -> String {
    let mut parts = fen.split_whitespace();
    let placement = parts.next().unwrap_or("");
    let stm = parts.next().unwrap_or("w");
    let castle = parts.next().unwrap_or("-");
    let ep = parts.next().unwrap_or("-");
    format!("{placement} {stm} {castle} {ep}")
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndexedPosition {
    pub fen: String,
    pub score: i32,
    pub wdl: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PositionIndex {
    pub keys: HashSet<String>,
    pub games: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexScan {
    pub new_games: usize,
    pub known_games: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct IndexReport {
    pub games_indexed: usize,
    pub positions_added: usize,
    pub positions_skipped: usize,
}

impl PositionIndex {
    pub fn game_id(tournament_id: &str, game_index: usize) -> String {
        format!("{tournament_id}:{game_index}")
    }

    pub fn contains_fen(&self, fen: &str) -> bool {
        self.keys.contains(&position_key(fen))
    }

    pub fn insert_fen(&mut self, fen: &str) -> bool {
        self.keys.insert(position_key(fen))
    }

    pub fn contains_game(&self, tournament_id: &str, game_index: usize) -> bool {
        self.games
            .contains(&Self::game_id(tournament_id, game_index))
    }

    pub fn mark_game(&mut self, tournament_id: &str, game_index: usize) {
        self.games.insert(Self::game_id(tournament_id, game_index));
    }

    pub fn load(path: &Path) -> Self {
        let Ok(file) = fs::File::open(path) else {
            return Self::default();
        };
        let mut index = Self::default();
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line == INDEX_HEADER {
                continue;
            }
            if let Some(game) = line.strip_prefix("game ") {
                index.games.insert(game.to_owned());
            } else if let Some(key) = line.strip_prefix("pos ") {
                index.keys.insert(key.to_owned());
            } else {
                index.keys.insert(line.to_owned());
            }
        }
        index
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut body = String::from(INDEX_HEADER);
        body.push('\n');
        let mut games: Vec<_> = self.games.iter().cloned().collect();
        games.sort();
        for game in games {
            body.push_str("game ");
            body.push_str(&game);
            body.push('\n');
        }
        let mut keys: Vec<_> = self.keys.iter().cloned().collect();
        keys.sort();
        for key in keys {
            body.push_str("pos ");
            body.push_str(&key);
            body.push('\n');
        }
        durable::atomic_write_text(path, &body)
    }
}

pub fn data_dir(root: &Path) -> PathBuf {
    root.join("ateed")
}

pub fn index_path(root: &Path) -> PathBuf {
    data_dir(root).join("position.index")
}

pub fn tournament_dataset_path(root: &Path) -> PathBuf {
    data_dir(root).join("tournament.txt")
}

pub fn extract_game_positions(game: &StoredTournamentGame) -> Vec<IndexedPosition> {
    extract_game_positions_scored(game, |_| 0)
}

pub fn extract_game_positions_scored(
    game: &StoredTournamentGame,
    mut score_of: impl FnMut(&Board) -> i32,
) -> Vec<IndexedPosition> {
    let wdl = game.white_score.clamp(0.0, 1.0) as f32;
    let mut board = Board::from_fen(&game.initial_fen).unwrap_or_else(|_| Board::new());
    let mut out = Vec::new();
    push_quiet(&mut out, &board, wdl, &mut score_of);
    for notation in &game.moves {
        let Some(mv) = legal_uci(&mut board, notation) else {
            break;
        };
        board.make_move(mv);
        push_quiet(&mut out, &board, wdl, &mut score_of);
    }
    out
}

fn push_quiet(
    out: &mut Vec<IndexedPosition>,
    board: &Board,
    wdl: f32,
    score_of: &mut impl FnMut(&Board) -> i32,
) {
    if board.in_check() {
        return;
    }
    out.push(IndexedPosition {
        fen: board.to_fen(),
        score: score_of(board),
        wdl,
    });
}

fn legal_uci(board: &mut Board, notation: &str) -> Option<Move> {
    let uci = notation
        .trim()
        .trim_end_matches(['+', '#', '!', '?'])
        .to_ascii_lowercase();
    board
        .generate_legal_moves()
        .iter()
        .copied()
        .find(|mv| mv.to_uci() == uci)
}

pub fn scan_unindexed(
    tournaments: &[(String, Vec<StoredTournamentGame>)],
    index: &PositionIndex,
) -> IndexScan {
    let mut scan = IndexScan::default();
    for (tournament_id, games) in tournaments {
        for game in games {
            if index.contains_game(tournament_id, game.game_index) {
                scan.known_games += 1;
            } else {
                scan.new_games += 1;
            }
        }
    }
    scan
}

pub fn index_games(
    tournaments: &[(String, Vec<StoredTournamentGame>)],
    index: &mut PositionIndex,
    dataset: &Path,
) -> Result<IndexReport, String> {
    index_games_scored(tournaments, index, dataset, |_| 0)
}

pub fn index_games_scored(
    tournaments: &[(String, Vec<StoredTournamentGame>)],
    index: &mut PositionIndex,
    dataset: &Path,
    mut score_of: impl FnMut(&Board) -> i32,
) -> Result<IndexReport, String> {
    let mut report = IndexReport::default();
    let mut added = Vec::new();
    for (tournament_id, games) in tournaments {
        for game in games {
            if index.contains_game(tournament_id, game.game_index) {
                continue;
            }
            for position in extract_game_positions_scored(game, &mut score_of) {
                if index.insert_fen(&position.fen) {
                    added.push(position);
                    report.positions_added += 1;
                } else {
                    report.positions_skipped += 1;
                }
            }
            index.mark_game(tournament_id, game.game_index);
            report.games_indexed += 1;
        }
    }
    if !added.is_empty() {
        append_dataset(dataset, &added)?;
    }
    Ok(report)
}

fn append_dataset(path: &Path, positions: &[IndexedPosition]) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    for position in positions {
        writeln!(
            file,
            "{}|{}|{:.1}",
            position.fen, position.score, position.wdl
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opening::START_FEN;
    use crate::tournament_store::StoredTournamentGame;

    fn start_game(moves: &[&str], white_score: f64) -> StoredTournamentGame {
        StoredTournamentGame {
            game_index: 0,
            round: 1,
            white: "A".into(),
            black: "B".into(),
            white_score,
            initial_fen: START_FEN.into(),
            moves: moves.iter().map(|mv| (*mv).to_owned()).collect(),
        }
    }

    #[test]
    fn position_key_drops_clocks() {
        assert_eq!(
            position_key("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"),
            position_key("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 12 40")
        );
        assert_ne!(
            position_key("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1"),
            position_key("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")
        );
    }

    #[test]
    fn extract_and_index_dedupes_across_games() {
        types::init();
        let first = start_game(&["e2e4", "e7e5"], 1.0);
        let second = start_game(&["e2e4", "c7c5"], 0.5);
        let positions = extract_game_positions(&first);
        assert!(positions.len() >= 3);
        assert!(
            positions
                .iter()
                .all(|pos| (pos.wdl - 1.0).abs() < f32::EPSILON)
        );

        let dir = std::env::temp_dir().join(format!(
            "mujrim-ateed-index-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        let index_path = dir.join("position.index");
        let data_path = dir.join("tournament.txt");
        let mut index = PositionIndex::default();
        let first_report = index_games(
            &[("t-1".into(), vec![first.clone()])],
            &mut index,
            &data_path,
        )
        .expect("index first");
        assert_eq!(first_report.games_indexed, 1);
        assert!(first_report.positions_added >= 3);
        let second_report = index_games(
            &[("t-1".into(), vec![first]), ("t-2".into(), vec![second])],
            &mut index,
            &data_path,
        )
        .expect("index second");
        assert_eq!(second_report.games_indexed, 1);
        assert!(second_report.positions_skipped >= 2);
        index.save(&index_path).expect("save");
        let restored = PositionIndex::load(&index_path);
        assert!(restored.contains_game("t-1", 0));
        assert!(restored.contains_game("t-2", 0));
        let scan = scan_unindexed(
            &[("t-1".into(), vec![start_game(&["e2e4"], 0.0)])],
            &restored,
        );
        assert_eq!(scan.new_games, 0);
        assert_eq!(scan.known_games, 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn extract_game_positions_uses_the_provided_scorer() {
        types::init();
        let game = start_game(&["e2e4"], 0.0);
        let positions = extract_game_positions_scored(&game, |_| 42);
        assert!(positions.len() >= 2);
        assert!(positions.iter().all(|position| position.score == 42));
        assert!(positions.iter().all(|position| position.wdl == 0.0));
    }
}
