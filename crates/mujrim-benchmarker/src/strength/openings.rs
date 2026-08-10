//! Deterministic paired-opening sources.

use std::collections::HashSet;
use std::path::Path;

use types::{Board, Move};

pub const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
pub const DEFAULT_OPENING_COUNT: usize = 512;

const DEFAULT_LINES: &[&str] = &[
    "",
    "e2e4 e7e5",
    "d2d4 d7d5",
    "c2c4 e7e5",
    "g1f3 d7d5",
    "e2e4 c7c5",
    "e2e4 e7e6",
    "e2e4 c7c6",
    "d2d4 g8f6 c2c4 g7g6",
    "d2d4 g8f6 c2c4 e7e6",
    "c2c4 g8f6 b1c3 e7e5",
    "g1f3 g8f6 g2g3 g7g6",
    "e2e4 e7e5 g1f3 b8c6 f1b5",
    "e2e4 c7c5 g1f3 d7d6 d2d4 c5d4",
    "d2d4 d7d5 c2c4 e7e6 b1c3",
    "c2c4 e7e5 b1c3 g8f6 g2g3",
    "e2e4 e7e5 g1f3 b8c6 f1c4 f8c5",
    "e2e4 e7e5 g1f3 b8c6 f1c4 g8f6",
    "e2e4 e7e5 g1f3 b8c6 d2d4 e5d4",
    "e2e4 e7e5 g1f3 g8f6",
    "e2e4 e7e5 b1c3 g8f6 f2f4 d7d5",
    "e2e4 e7e5 f2f4 e5f4",
    "e2e4 e7e5 d2d4 e5d4 d1d4 b8c6",
    "e2e4 e7e5 f1c4 g8f6 d2d3 f8c5",
    "e2e4 c7c5 g1f3 b8c6 d2d4 c5d4 f3d4",
    "e2e4 c7c5 g1f3 e7e6 d2d4 c5d4 f3d4",
    "e2e4 c7c5 g1f3 d7d6 d2d4 c5d4 f3d4",
    "e2e4 c7c5 c2c3 d7d5 e4d5 d8d5",
    "e2e4 c7c5 b1c3 b8c6 g2g3 g7g6",
    "e2e4 c7c5 f2f4 d7d5 e4d5 g8f6",
    "e2e4 c7c5 g1f3 g7g6 d2d4 f8g7",
    "e2e4 c7c5 g1f3 a7a6 d2d4 c5d4 f3d4",
    "e2e4 e7e6 d2d4 d7d5 b1c3 g8f6",
    "e2e4 e7e6 d2d4 d7d5 e4e5 c7c5",
    "e2e4 e7e6 d2d4 d7d5 e4d5 e6d5",
    "e2e4 e7e6 d2d4 d7d5 b1d2",
    "e2e4 c7c6 d2d4 d7d5 e4e5 c8f5",
    "e2e4 c7c6 d2d4 d7d5 b1c3 d5e4",
    "e2e4 c7c6 d2d4 d7d5 e4d5 c6d5",
    "e2e4 d7d5 e4d5 d8d5 b1c3 d5d8",
    "d2d4 d7d5 c2c4 e7e6 b1c3 g8f6",
    "d2d4 d7d5 c2c4 c7c6 g1f3 g8f6",
    "d2d4 g8f6 c2c4 e7e6 b1c3 f8b4",
    "d2d4 g8f6 c2c4 g7g6 b1c3 f8g7",
    "d2d4 g8f6 c2c4 g7g6 b1c3 d7d5",
    "d2d4 g8f6 g1f3 e7e6 e2e3 d7d5",
    "d2d4 f7f5 c2c4 g8f6 b1c3 e7e6",
    "d2d4 d7d5 g1f3 g8f6 e2e3 e7e6",
    "c2c4 e7e5 b1c3 g8f6 g2g3 f8b4",
    "c2c4 c7c5 b1c3 b8c6 g2g3 g7g6",
    "c2c4 g8f6 b1c3 e7e5 g2g3 f8b4",
    "c2c4 e7e6 g1f3 d7d5 d2d4 g8f6",
    "g1f3 d7d5 g2g3 c7c5 f1g2 b8c6",
    "g1f3 g8f6 g2g3 g7g6 f1g2 f8g7",
    "g1f3 d7d5 d2d4 g8f6 c2c4 e7e6",
    "g1f3 c7c5 c2c4 b8c6 d2d4 c5d4",
    "b2b3 d7d5 c1b2 g8f6 g1f3 e7e6",
    "b2b3 e7e5 c1b2 b8c6 e2e3 g8f6",
    "g2g3 d7d5 f1g2 e7e5 d2d3 g8f6",
    "f2f4 d7d5 g1f3 g8f6 e2e3 e7e6",
    "b1c3 d7d5 e2e4 d5d4 c3e2 e7e5",
    "e2e3 d7d5 g1f3 g8f6 c2c4 e7e6",
    "d2d3 d7d5 b1d2 g8f6 e2e4 e7e5",
    "c2c3 d7d5 d2d4 g8f6 g1f3 e7e6",
];

#[derive(Clone, Debug)]
pub struct Opening {
    pub initial_fen: String,
    pub moves: Vec<String>,
}

impl Opening {
    pub fn board(&self) -> Result<Board, String> {
        let mut board = Board::from_fen(&self.initial_fen)?;
        for uci in &self.moves {
            let mv = resolve_legal_move(&mut board, uci)
                .ok_or_else(|| format!("illegal opening move '{uci}'"))?;
            board.make_move(mv);
        }
        Ok(board)
    }
}

pub fn default_openings() -> Vec<Opening> {
    let curated: Vec<_> = DEFAULT_LINES
        .iter()
        .map(|line| Opening {
            initial_fen: START_FEN.to_string(),
            moves: line.split_whitespace().map(ToString::to_string).collect(),
        })
        .collect();
    let mut openings = Vec::with_capacity(DEFAULT_OPENING_COUNT);
    let mut seen = HashSet::with_capacity(DEFAULT_OPENING_COUNT);
    for opening in &curated {
        let position = opening
            .board()
            .expect("built-in opening lines must remain legal");
        if seen.insert(position.tt_hash()) {
            openings.push(opening.clone());
        }
    }

    let mut round = 0u64;
    while openings.len() < DEFAULT_OPENING_COUNT {
        for (index, base) in curated.iter().enumerate().skip(1) {
            if openings.len() == DEFAULT_OPENING_COUNT {
                break;
            }
            let seed = 0x9e37_79b9_7f4a_7c15u64
                ^ (index as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9)
                ^ round.wrapping_mul(0x94d0_49bb_1331_11eb);
            if let Some((opening, position_hash)) = extend_opening(base, seed)
                && seen.insert(position_hash)
            {
                openings.push(opening);
            }
        }
        round += 1;
        assert!(round < 64, "failed to generate enough unique openings");
    }

    openings
}

fn extend_opening(base: &Opening, mut state: u64) -> Option<(Opening, u64)> {
    let mut opening = base.clone();
    let mut board = opening.board().ok()?;
    let plies = 4 + (splitmix64(&mut state) % 5) as usize;

    for _ in 0..plies {
        let legal = board.generate_legal_moves();
        if legal.is_empty() {
            return None;
        }
        let mv = legal[(splitmix64(&mut state) as usize) % legal.len()];
        opening.moves.push(mv.to_uci());
        board.make_move(mv);
    }

    (!board.is_game_over()).then(|| (opening, board.tt_hash()))
}

#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

pub fn openings_fingerprint(openings: &[Opening]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for opening in openings {
        for byte in opening
            .initial_fen
            .bytes()
            .chain(std::iter::once(0xff))
            .chain(
                opening
                    .moves
                    .iter()
                    .flat_map(|mv| mv.bytes().chain(std::iter::once(0xfe))),
            )
        {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

/// Lines accept either `startpos moves ...`, `fen <FEN> moves ...`, or a bare FEN.
pub fn load_openings(path: &Path) -> Result<Vec<Opening>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read '{}': {error}", path.display()))?;
    let mut openings = Vec::new();
    for (line_index, raw) in content.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let opening = parse_opening(line)
            .map_err(|error| format!("{}:{}: {error}", path.display(), line_index + 1))?;
        opening.board()?;
        openings.push(opening);
    }
    if openings.is_empty() {
        Err(format!("no openings found in '{}'", path.display()))
    } else {
        Ok(openings)
    }
}

fn parse_opening(line: &str) -> Result<Opening, String> {
    if let Some(moves) = line.strip_prefix("startpos") {
        let moves = moves.trim().strip_prefix("moves").unwrap_or(moves.trim());
        return Ok(Opening {
            initial_fen: START_FEN.to_string(),
            moves: moves.split_whitespace().map(ToString::to_string).collect(),
        });
    }

    let line = line.strip_prefix("fen ").unwrap_or(line);
    let (fen, moves) = line
        .split_once(" moves ")
        .map_or((line, ""), |(fen, moves)| (fen, moves));
    Board::from_fen(fen)?;
    Ok(Opening {
        initial_fen: fen.to_string(),
        moves: moves.split_whitespace().map(ToString::to_string).collect(),
    })
}

pub fn resolve_legal_move(board: &mut Board, uci: &str) -> Option<Move> {
    board
        .generate_legal_moves()
        .iter()
        .copied()
        .find(|mv| mv.to_uci() == uci)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_openings_are_legal() {
        let openings = default_openings();
        assert_eq!(openings.len(), DEFAULT_OPENING_COUNT);
        let unique: HashSet<_> = openings
            .iter()
            .map(|opening| opening.board().unwrap().tt_hash())
            .collect();
        assert_eq!(unique.len(), DEFAULT_OPENING_COUNT);
        for opening in openings {
            opening.board().unwrap();
        }
    }

    #[test]
    fn generated_openings_and_fingerprint_are_deterministic() {
        let first = default_openings();
        let second = default_openings();
        assert_eq!(openings_fingerprint(&first), openings_fingerprint(&second));
        assert_eq!(first[200].moves, second[200].moves);
    }

    #[test]
    fn parses_startpos_and_fen_formats() {
        let start = parse_opening("startpos moves e2e4 e7e5").unwrap();
        assert_eq!(start.moves, ["e2e4", "e7e5"]);
        start.board().unwrap();

        let fen = parse_opening(&format!("fen {START_FEN} moves d2d4")).unwrap();
        assert_eq!(fen.moves, ["d2d4"]);
        fen.board().unwrap();
    }
}
