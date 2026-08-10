//! PGN parsing and legal SAN-to-UCI conversion.

use std::collections::BTreeMap;

use types::chess_move::MoveFlag;
use types::{Board, Move, Piece, Square};

use crate::database::GameMetadata;
use crate::opening::START_FEN;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedGame {
    pub metadata: GameMetadata,
    pub initial_fen: String,
    pub moves: Vec<String>,
    pub result: String,
    pub source: String,
}

pub fn parse_games(input: &str) -> Result<Vec<ParsedGame>, String> {
    types::init();
    let blocks = split_games(input);
    if blocks.is_empty() {
        return Err("PGN input does not contain a game".to_owned());
    }
    blocks
        .into_iter()
        .enumerate()
        .map(|(index, block)| {
            parse_game(&block).map_err(|error| format!("game {}: {error}", index + 1))
        })
        .collect()
}

fn split_games(input: &str) -> Vec<String> {
    let mut games = Vec::new();
    let mut current = String::new();
    let mut saw_movetext = false;
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && saw_movetext {
            if !current.trim().is_empty() {
                games.push(current.trim().to_owned());
            }
            current.clear();
            saw_movetext = false;
        }
        if !trimmed.is_empty() && !trimmed.starts_with('[') {
            saw_movetext = true;
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        games.push(current.trim().to_owned());
    }
    games
}

fn parse_game(source: &str) -> Result<ParsedGame, String> {
    let mut tags = BTreeMap::new();
    let mut movetext = String::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            let (name, value) = parse_tag(trimmed)?;
            tags.insert(name, value);
        } else {
            movetext.push_str(line);
            movetext.push('\n');
        }
    }

    let initial_fen = tags
        .get("FEN")
        .cloned()
        .unwrap_or_else(|| START_FEN.to_owned());
    let mut board = Board::from_fen(&initial_fen)?;
    let cleaned = strip_comments_and_variations(&movetext)?;
    let mut moves = Vec::new();
    let mut result = tags
        .get("Result")
        .cloned()
        .unwrap_or_else(|| "*".to_owned());

    for raw in cleaned.split_whitespace() {
        let token = strip_move_number(raw);
        if token.is_empty() || token.starts_with('$') || token.eq_ignore_ascii_case("e.p.") {
            continue;
        }
        if is_result(token) {
            result = token.to_owned();
            continue;
        }
        let mv = resolve_notation(&mut board, token).ok_or_else(|| {
            format!(
                "cannot resolve move '{token}' after {} plies in {}",
                moves.len(),
                board.to_fen()
            )
        })?;
        moves.push(mv.to_uci());
        board.make_move(mv);
    }
    if moves.is_empty() {
        return Err("PGN contains no legal moves".to_owned());
    }

    Ok(ParsedGame {
        metadata: GameMetadata {
            event: tag(&tags, "Event"),
            site: tag(&tags, "Site"),
            date: tag(&tags, "Date"),
            round: tag(&tags, "Round"),
            white: tag(&tags, "White"),
            black: tag(&tags, "Black"),
            result: result.clone(),
            white_elo: parse_rating(tags.get("WhiteElo")),
            black_elo: parse_rating(tags.get("BlackElo")),
            eco: tag(&tags, "ECO"),
        },
        initial_fen,
        moves,
        result,
        source: format!("{}\n", source.trim()),
    })
}

fn parse_tag(line: &str) -> Result<(String, String), String> {
    let body = line
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .ok_or_else(|| format!("invalid PGN tag '{line}'"))?;
    let separator = body
        .find(char::is_whitespace)
        .ok_or_else(|| format!("invalid PGN tag '{line}'"))?;
    let name = body[..separator].trim();
    let value = body[separator..].trim();
    if name.is_empty() || !value.starts_with('"') || !value.ends_with('"') {
        return Err(format!("invalid PGN tag '{line}'"));
    }
    let mut decoded = String::with_capacity(value.len().saturating_sub(2));
    let mut escaped = false;
    for character in value[1..value.len() - 1].chars() {
        if escaped {
            decoded.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else {
            decoded.push(character);
        }
    }
    if escaped {
        return Err(format!("truncated escape in PGN tag '{line}'"));
    }
    Ok((name.to_owned(), decoded))
}

fn strip_comments_and_variations(input: &str) -> Result<String, String> {
    let mut output = String::with_capacity(input.len());
    let mut brace_depth = 0usize;
    let mut variation_depth = 0usize;
    let mut line_comment = false;
    for character in input.chars() {
        if line_comment {
            if character == '\n' {
                line_comment = false;
                output.push(' ');
            }
            continue;
        }
        if brace_depth > 0 {
            match character {
                '{' => brace_depth += 1,
                '}' => brace_depth -= 1,
                _ => {}
            }
            continue;
        }
        if variation_depth > 0 {
            match character {
                '(' => variation_depth += 1,
                ')' => variation_depth -= 1,
                _ => {}
            }
            continue;
        }
        match character {
            '{' => brace_depth = 1,
            '(' => variation_depth = 1,
            ';' => line_comment = true,
            '}' => return Err("unmatched PGN comment terminator".to_owned()),
            ')' => return Err("unmatched PGN variation terminator".to_owned()),
            _ => output.push(character),
        }
    }
    if brace_depth != 0 {
        return Err("unterminated PGN comment".to_owned());
    }
    if variation_depth != 0 {
        return Err("unterminated PGN variation".to_owned());
    }
    Ok(output)
}

fn strip_move_number(mut token: &str) -> &str {
    if let Some(index) = token.rfind('.')
        && token[..=index]
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
    {
        token = &token[index + 1..];
    }
    token
}

fn resolve_notation(board: &mut Board, notation: &str) -> Option<Move> {
    let legal = board.generate_legal_moves();
    if let Some(parsed) = Move::from_uci(notation)
        && let Some(mv) = legal.iter().find(|mv| {
            mv.from == parsed.from && mv.to == parsed.to && mv.promotion == parsed.promotion
        })
    {
        return Some(*mv);
    }

    let mut san = notation
        .trim_end_matches(['!', '?', '+', '#'])
        .replace('0', "O");
    while san.ends_with(['!', '?', '+', '#']) {
        san.pop();
    }
    if san == "O-O" || san == "O-O-O" {
        let king_side = san == "O-O";
        return legal
            .iter()
            .find(|mv| {
                mv.is_castling()
                    && matches!(
                        (king_side, mv.flag),
                        (true, MoveFlag::KingCastle) | (false, MoveFlag::QueenCastle)
                    )
            })
            .copied();
    }

    let promotion = san
        .find('=')
        .and_then(|index| san[index + 1..].chars().next())
        .and_then(Piece::from_char);
    if let Some(index) = san.find('=') {
        san.truncate(index);
    }
    if san.len() < 2 {
        return None;
    }
    let destination = san[san.len() - 2..].parse::<Square>().ok()?;
    let prefix = &san[..san.len() - 2];
    let (piece, disambiguation) = match prefix.chars().next() {
        Some('N') => (Piece::Knight, &prefix[1..]),
        Some('B') => (Piece::Bishop, &prefix[1..]),
        Some('R') => (Piece::Rook, &prefix[1..]),
        Some('Q') => (Piece::Queen, &prefix[1..]),
        Some('K') => (Piece::King, &prefix[1..]),
        _ => (Piece::Pawn, prefix),
    };
    let disambiguation = disambiguation.replace('x', "");
    let mut candidates = legal.iter().filter(|mv| {
        mv.to == destination
            && mv.promotion == promotion
            && board
                .piece_on(mv.from)
                .is_some_and(|(moving, _)| moving == piece)
            && disambiguation.chars().all(|character| {
                if character.is_ascii_alphabetic() {
                    mv.from.file() == character.to_ascii_lowercase() as u8 - b'a'
                } else if character.is_ascii_digit() {
                    mv.from.rank() == character as u8 - b'1'
                } else {
                    false
                }
            })
    });
    let candidate = candidates.next().copied()?;
    candidates.next().is_none().then_some(candidate)
}

fn tag(tags: &BTreeMap<String, String>, name: &str) -> String {
    tags.get(name).cloned().unwrap_or_default()
}

fn parse_rating(value: Option<&String>) -> Option<u32> {
    value.and_then(|rating| rating.parse().ok())
}

fn is_result(token: &str) -> bool {
    matches!(token, "1-0" | "0-1" | "1/2-1/2" | "*")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_san_comments_variations_castling_and_metadata() {
        let input = r#"[Event "Training"]
[White "Alpha"]
[Black "Beta"]
[WhiteElo "2100"]
[BlackElo "2050"]
[ECO "C60"]
[Result "1-0"]

1. e4 {main} e5 2. Nf3 (2. Bc4) Nc6 3. Bb5 a6 4. Ba4 Nf6 5. O-O Be7 1-0"#;
        let games = parse_games(input).unwrap();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].metadata.white, "Alpha");
        assert_eq!(games[0].metadata.white_elo, Some(2100));
        assert_eq!(games[0].moves[0], "e2e4");
        assert_eq!(games[0].moves[8], "e1g1");
        assert_eq!(games[0].result, "1-0");
    }

    #[test]
    fn parses_multiple_games_and_uci_exports() {
        let input =
            "[Event \"One\"]\n\n1. e2e4 e7e5 1-0\n\n[Event \"Two\"]\n\n1. d2d4 d7d5 1/2-1/2\n";
        let games = parse_games(input).unwrap();
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].moves, ["e2e4", "e7e5"]);
        assert_eq!(games[1].moves, ["d2d4", "d7d5"]);
    }

    #[test]
    fn resolves_disambiguation_and_promotion() {
        let input =
            "[SetUp \"1\"]\n[FEN \"4k3/P7/8/8/8/8/3N3N/4K3 w - - 0 1\"]\n\n1. Ndf3 Kf7 2. a8=Q *";
        let games = parse_games(input).unwrap();
        assert_eq!(games[0].moves, ["d2f3", "e8f7", "a7a8q"]);
    }

    #[test]
    fn rejects_ambiguous_or_illegal_moves() {
        let input = "[Event \"Broken\"]\n\n1. e5 *";
        assert!(parse_games(input).unwrap_err().contains("cannot resolve"));
    }
}
