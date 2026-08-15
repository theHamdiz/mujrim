//! Portable game export: PGN, EPD, UCI, JSON, Stockfish-plain, and gzip binpack.

use std::io::{Read, Write};
use std::path::Path;

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use types::Board;

use crate::opening::START_FEN;
use crate::pgn::{self, ParsedGame};

pub const BINPACK_MAGIC: &[u8; 4] = b"MJBP";
pub const BINPACK_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameExportFormat {
    Pgn,
    Epd,
    Uci,
    Json,
    Plain,
    Binpack,
}

impl GameExportFormat {
    pub fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("pgn") => Self::Pgn,
            Some("epd") | Some("fen") => Self::Epd,
            Some("uci") | Some("txt") => Self::Uci,
            Some("json") => Self::Json,
            Some("plain") => Self::Plain,
            Some("binpack") | Some("gz") => Self::Binpack,
            _ => Self::Pgn,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Pgn => "pgn",
            Self::Epd => "epd",
            Self::Uci => "uci",
            Self::Json => "json",
            Self::Plain => "plain",
            Self::Binpack => "binpack",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pgn => "PGN",
            Self::Epd => "EPD",
            Self::Uci => "UCI",
            Self::Json => "JSON",
            Self::Plain => "Plain",
            Self::Binpack => "Binpack",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameRecord {
    pub event: String,
    pub site: String,
    pub date: String,
    pub round: String,
    pub white: String,
    pub black: String,
    pub result: String,
    pub initial_fen: String,
    pub moves: Vec<String>,
    pub comments: Vec<(usize, String)>,
}

impl GameRecord {
    pub fn from_parsed(game: ParsedGame) -> Self {
        Self {
            event: game.metadata.event,
            site: game.metadata.site,
            date: game.metadata.date,
            round: game.metadata.round,
            white: game.metadata.white,
            black: game.metadata.black,
            result: game.result,
            initial_fen: game.initial_fen,
            moves: game.moves,
            comments: Vec::new(),
        }
    }

    pub fn result_wdl(&self) -> i8 {
        match self.result.as_str() {
            "1-0" => 1,
            "0-1" => -1,
            _ => 0,
        }
    }
}

pub fn result_from_white_score(score: f64) -> &'static str {
    if score >= 0.75 {
        "1-0"
    } else if score <= 0.25 {
        "0-1"
    } else {
        "1/2-1/2"
    }
}

pub fn encode_games(games: &[GameRecord], format: GameExportFormat) -> Result<Vec<u8>, String> {
    match format {
        GameExportFormat::Pgn => Ok(encode_pgn(games).into_bytes()),
        GameExportFormat::Epd => Ok(encode_epd(games)?.into_bytes()),
        GameExportFormat::Uci => Ok(encode_uci(games).into_bytes()),
        GameExportFormat::Json => Ok(encode_json(games).into_bytes()),
        GameExportFormat::Plain => Ok(encode_plain(games)?.into_bytes()),
        GameExportFormat::Binpack => encode_binpack(games),
    }
}

pub fn encode_positions_binpack(positions: &[TrainingPosition]) -> Result<Vec<u8>, String> {
    let records = encode_training_records(positions);
    let mut compressed = Vec::new();
    {
        let mut encoder = GzEncoder::new(&mut compressed, Compression::default());
        encoder
            .write_all(&records)
            .map_err(|error| format!("failed to compress binpack: {error}"))?;
        encoder
            .finish()
            .map_err(|error| format!("failed to finish binpack: {error}"))?;
    }
    let mut out = Vec::with_capacity(14 + compressed.len());
    out.extend_from_slice(BINPACK_MAGIC);
    out.extend_from_slice(&BINPACK_VERSION.to_le_bytes());
    out.extend_from_slice(&(records.len() as u64).to_le_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
}

pub fn write_games(
    games: &[GameRecord],
    path: impl AsRef<Path>,
) -> Result<GameExportFormat, String> {
    let path = path.as_ref();
    let format = GameExportFormat::from_path(path);
    let bytes = encode_games(games, format)?;
    std::fs::write(path, bytes)
        .map_err(|error| format!("failed to write '{}': {error}", path.display()))?;
    Ok(format)
}

pub fn decode_binpack(bytes: &[u8]) -> Result<Vec<TrainingPosition>, String> {
    if bytes.len() < 14 || &bytes[..4] != BINPACK_MAGIC {
        return Err("not a Mujrim binpack".to_owned());
    }
    let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
    if version != BINPACK_VERSION {
        return Err(format!("unsupported binpack version {version}"));
    }
    let uncompressed_len = u64::from_le_bytes(bytes[6..14].try_into().unwrap()) as usize;
    let mut decoder = GzDecoder::new(&bytes[14..]);
    let mut plain = Vec::new();
    decoder
        .read_to_end(&mut plain)
        .map_err(|error| format!("failed to inflate binpack: {error}"))?;
    if uncompressed_len != 0 && plain.len() != uncompressed_len {
        return Err("binpack length mismatch".to_owned());
    }
    decode_training_records(&plain)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrainingPosition {
    pub fen: String,
    pub mv: String,
    pub score_cp: i16,
    pub ply: u16,
    pub wdl: i8,
}

fn encode_pgn(games: &[GameRecord]) -> String {
    let mut out = String::new();
    for (index, game) in games.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&pgn_text(game));
        out.push('\n');
    }
    out
}

fn pgn_text(game: &GameRecord) -> String {
    types::init();
    let fen = if game.initial_fen.is_empty() {
        START_FEN
    } else {
        game.initial_fen.as_str()
    };
    let mut tags = format!(
        "[Event \"{}\"]\n[Site \"{}\"]\n[Date \"{}\"]\n[Round \"{}\"]\n[White \"{}\"]\n[Black \"{}\"]\n[Result \"{}\"]\n",
        escape_tag(&game.event),
        escape_tag(&game.site),
        escape_tag(if game.date.is_empty() {
            "????.??.??"
        } else {
            game.date.as_str()
        }),
        escape_tag(if game.round.is_empty() {
            "-"
        } else {
            game.round.as_str()
        }),
        escape_tag(&game.white),
        escape_tag(&game.black),
        escape_tag(&game.result),
    );
    if fen != START_FEN {
        tags.push_str(&format!("[FEN \"{}\"]\n[SetUp \"1\"]\n", escape_tag(fen)));
    }
    tags.push('\n');
    let Ok(mut board) = Board::from_fen(fen) else {
        return format!("{tags}*");
    };
    for (index, uci) in game.moves.iter().enumerate() {
        if index % 2 == 0 {
            tags.push_str(&format!("{}. ", index / 2 + 1));
        }
        tags.push_str(&pgn::uci_to_san(&board, uci));
        if let Some((_, comment)) = game.comments.iter().find(|(ply, _)| *ply == index + 1) {
            tags.push_str(&format!(" {{ {} }}", escape_comment(comment)));
        }
        tags.push(' ');
        if let Some(mv) = pgn::resolve_uci(&board, uci) {
            board.make_move(mv);
        }
    }
    tags.push_str(&game.result);
    tags
}

fn encode_epd(games: &[GameRecord]) -> Result<String, String> {
    types::init();
    let mut out = String::new();
    for game in games {
        let fen = if game.initial_fen.is_empty() {
            START_FEN
        } else {
            game.initial_fen.as_str()
        };
        let mut board = Board::from_fen(fen).map_err(|error| error.to_string())?;
        for (index, uci) in game.moves.iter().enumerate() {
            let ops = format!(
                "c0 \"{} vs {}\"; result \"{}\"; ply {};",
                escape_tag(&game.white),
                escape_tag(&game.black),
                game.result,
                index
            );
            out.push_str(&epd_from_fen(&board.to_fen(), &ops));
            out.push('\n');
            if let Some(mv) = pgn::resolve_uci(&board, uci) {
                board.make_move(mv);
            }
        }
        if game.moves.is_empty() {
            out.push_str(&epd_from_fen(
                fen,
                &format!(
                    "c0 \"{} vs {}\"; result \"{}\";",
                    escape_tag(&game.white),
                    escape_tag(&game.black),
                    game.result
                ),
            ));
            out.push('\n');
        }
    }
    Ok(out)
}

fn epd_from_fen(fen: &str, ops: &str) -> String {
    let fields: Vec<&str> = fen.split_whitespace().collect();
    if fields.len() >= 4 {
        format!(
            "{} {} {} {} {ops}",
            fields[0], fields[1], fields[2], fields[3]
        )
    } else {
        format!("{fen} {ops}")
    }
}

fn encode_uci(games: &[GameRecord]) -> String {
    let mut out = String::new();
    for game in games {
        out.push_str(&format!(
            "id {} vs {}\nfen {}\nmoves {}\nresult {}\n\n",
            game.white,
            game.black,
            if game.initial_fen.is_empty() {
                START_FEN
            } else {
                game.initial_fen.as_str()
            },
            game.moves.join(" "),
            game.result
        ));
    }
    out
}

fn encode_json(games: &[GameRecord]) -> String {
    let mut out = String::from("{\"games\":[");
    for (index, game) in games.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"event\":");
        out.push_str(&json_string(&game.event));
        out.push_str(",\"site\":");
        out.push_str(&json_string(&game.site));
        out.push_str(",\"date\":");
        out.push_str(&json_string(&game.date));
        out.push_str(",\"round\":");
        out.push_str(&json_string(&game.round));
        out.push_str(",\"white\":");
        out.push_str(&json_string(&game.white));
        out.push_str(",\"black\":");
        out.push_str(&json_string(&game.black));
        out.push_str(",\"result\":");
        out.push_str(&json_string(&game.result));
        out.push_str(",\"fen\":");
        out.push_str(&json_string(&game.initial_fen));
        out.push_str(",\"moves\":[");
        for (move_index, mv) in game.moves.iter().enumerate() {
            if move_index > 0 {
                out.push(',');
            }
            out.push_str(&json_string(mv));
        }
        out.push_str("],\"comments\":[");
        for (comment_index, (ply, note)) in game.comments.iter().enumerate() {
            if comment_index > 0 {
                out.push(',');
            }
            out.push_str(&format!("{{\"ply\":{ply},\"text\":{}}}", json_string(note)));
        }
        out.push_str("]}");
    }
    out.push_str("]}");
    out
}

fn encode_plain(games: &[GameRecord]) -> Result<String, String> {
    let positions = training_positions(games)?;
    let mut out = String::new();
    for position in positions {
        out.push_str(&format!(
            "fen {}\nmove {}\nscore {}\nply {}\nresult {}\ne\n",
            position.fen, position.mv, position.score_cp, position.ply, position.wdl as f32
        ));
    }
    Ok(out)
}

fn encode_binpack(games: &[GameRecord]) -> Result<Vec<u8>, String> {
    encode_positions_binpack(&training_positions(games)?)
}

fn training_positions(games: &[GameRecord]) -> Result<Vec<TrainingPosition>, String> {
    types::init();
    let mut positions = Vec::new();
    for game in games {
        let fen = if game.initial_fen.is_empty() {
            START_FEN
        } else {
            game.initial_fen.as_str()
        };
        let mut board = Board::from_fen(fen).map_err(|error| error.to_string())?;
        let wdl = game.result_wdl();
        for (index, uci) in game.moves.iter().enumerate() {
            positions.push(TrainingPosition {
                fen: board.to_fen(),
                mv: pgn::resolve_uci(&board, uci)
                    .map(|mv| mv.to_uci())
                    .unwrap_or_else(|| uci.trim_end_matches(['+', '#']).to_owned()),
                score_cp: 0,
                ply: (index + 1) as u16,
                wdl,
            });
            if let Some(mv) = pgn::resolve_uci(&board, uci) {
                board.make_move(mv);
            }
        }
    }
    Ok(positions)
}

fn encode_training_records(positions: &[TrainingPosition]) -> Vec<u8> {
    let mut out = Vec::new();
    for position in positions {
        let fen = position.fen.as_bytes();
        let mv = position.mv.as_bytes();
        out.extend_from_slice(&(fen.len() as u16).to_le_bytes());
        out.extend_from_slice(fen);
        out.push(mv.len() as u8);
        out.extend_from_slice(mv);
        out.extend_from_slice(&position.score_cp.to_le_bytes());
        out.extend_from_slice(&position.ply.to_le_bytes());
        out.push(position.wdl as u8);
    }
    out
}

fn decode_training_records(bytes: &[u8]) -> Result<Vec<TrainingPosition>, String> {
    let mut positions = Vec::new();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        if cursor + 2 > bytes.len() {
            return Err("truncated fen length".to_owned());
        }
        let fen_len = u16::from_le_bytes(bytes[cursor..cursor + 2].try_into().unwrap()) as usize;
        cursor += 2;
        if cursor + fen_len + 1 > bytes.len() {
            return Err("truncated fen".to_owned());
        }
        let fen = String::from_utf8(bytes[cursor..cursor + fen_len].to_vec())
            .map_err(|error| format!("invalid fen: {error}"))?;
        cursor += fen_len;
        let move_len = bytes[cursor] as usize;
        cursor += 1;
        if cursor + move_len + 5 > bytes.len() {
            return Err("truncated move record".to_owned());
        }
        let mv = String::from_utf8(bytes[cursor..cursor + move_len].to_vec())
            .map_err(|error| format!("invalid move: {error}"))?;
        cursor += move_len;
        let score_cp = i16::from_le_bytes(bytes[cursor..cursor + 2].try_into().unwrap());
        cursor += 2;
        let ply = u16::from_le_bytes(bytes[cursor..cursor + 2].try_into().unwrap());
        cursor += 2;
        let wdl = bytes[cursor] as i8;
        cursor += 1;
        positions.push(TrainingPosition {
            fen,
            mv,
            score_cp,
            ply,
            wdl,
        });
    }
    Ok(positions)
}

pub fn import_text(text: &str) -> Result<Vec<GameRecord>, String> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        return parse_json_games(trimmed);
    }
    if trimmed.contains("[Event") || trimmed.contains("[White") {
        return pgn::parse_games(trimmed)
            .map(|games| games.into_iter().map(GameRecord::from_parsed).collect());
    }
    if let Ok(board) = {
        types::init();
        Board::from_fen(trimmed.split('\n').next().unwrap_or_default())
    } {
        return Ok(vec![GameRecord {
            event: "Imported position".to_owned(),
            site: "Local".to_owned(),
            date: String::new(),
            round: String::new(),
            white: "White".to_owned(),
            black: "Black".to_owned(),
            result: "*".to_owned(),
            initial_fen: board.to_fen(),
            moves: Vec::new(),
            comments: Vec::new(),
        }]);
    }
    pgn::parse_games(trimmed).map(|games| games.into_iter().map(GameRecord::from_parsed).collect())
}

fn parse_json_games(text: &str) -> Result<Vec<GameRecord>, String> {
    let games = pgn::parse_games(&json_games_to_pgn(text)?)?;
    Ok(games.into_iter().map(GameRecord::from_parsed).collect())
}

fn json_games_to_pgn(text: &str) -> Result<String, String> {
    let mut records = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("\"moves\"") {
        let white = json_extract_string(rest, "white").unwrap_or_else(|| "White".to_owned());
        let black = json_extract_string(rest, "black").unwrap_or_else(|| "Black".to_owned());
        let result = json_extract_string(rest, "result").unwrap_or_else(|| "*".to_owned());
        let fen = json_extract_string(rest, "fen").unwrap_or_default();
        let event = json_extract_string(rest, "event").unwrap_or_else(|| "Imported".to_owned());
        let moves = json_extract_string_array(&rest[start..]).unwrap_or_default();
        records.push(GameRecord {
            event,
            site: json_extract_string(rest, "site").unwrap_or_else(|| "Local".to_owned()),
            date: json_extract_string(rest, "date").unwrap_or_default(),
            round: json_extract_string(rest, "round").unwrap_or_default(),
            white,
            black,
            result,
            initial_fen: fen,
            moves,
            comments: Vec::new(),
        });
        rest = rest.get(start + 7..).unwrap_or("");
        if records.len() > 10_000 {
            break;
        }
    }
    if records.is_empty() {
        return Err("JSON did not contain any games".to_owned());
    }
    Ok(encode_pgn(&records))
}

fn json_extract_string(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let after = text.split(&needle).nth(1)?;
    let after = after.trim_start_matches(|ch: char| ch == ':' || ch.is_whitespace());
    if !after.starts_with('"') {
        return None;
    }
    let mut out = String::new();
    let mut escaped = false;
    for character in after[1..].chars() {
        if escaped {
            out.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            return Some(out);
        } else {
            out.push(character);
        }
    }
    None
}

fn json_extract_string_array(text: &str) -> Option<Vec<String>> {
    let start = text.find('[')?;
    let end = text[start..].find(']')? + start;
    let body = &text[start + 1..end];
    let mut items = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut escaped = false;
    for character in body.chars() {
        if !in_string {
            if character == '"' {
                in_string = true;
                current.clear();
            }
            continue;
        }
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            items.push(std::mem::take(&mut current));
            in_string = false;
        } else {
            current.push(character);
        }
    }
    Some(items)
}

fn escape_tag(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn escape_comment(value: &str) -> String {
    value.replace('}', "\\}").replace('\n', " ")
}

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn italian() -> GameRecord {
        GameRecord {
            event: "Training".into(),
            site: "Local".into(),
            date: "2026.08.13".into(),
            round: "1".into(),
            white: "Alpha".into(),
            black: "Beta".into(),
            result: "1-0".into(),
            initial_fen: START_FEN.into(),
            moves: vec!["e2e4".into(), "e7e5".into(), "g1f3".into()],
            comments: vec![(1, "King's pawn".into())],
        }
    }

    #[test]
    fn pgn_includes_san_and_comments() {
        let pgn = encode_pgn(&[italian()]);
        assert!(pgn.contains("[White \"Alpha\"]"));
        assert!(pgn.contains("1. e4 { King's pawn } e5 2. Nf3 1-0"));
    }

    #[test]
    fn binpack_round_trips_training_positions() {
        let bytes = encode_binpack(&[italian()]).unwrap();
        assert!(bytes.starts_with(BINPACK_MAGIC));
        let positions = decode_binpack(&bytes).unwrap();
        assert_eq!(positions.len(), 3);
        assert_eq!(positions[0].mv, "e2e4");
        assert_eq!(positions[0].wdl, 1);
        assert_eq!(positions[2].mv, "g1f3");
        assert!(
            positions[0]
                .fen
                .contains("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR")
        );
        let scored = encode_positions_binpack(&positions).unwrap();
        assert_eq!(decode_binpack(&scored).unwrap(), positions);
    }

    #[test]
    fn json_and_uci_export_the_move_list() {
        let json = encode_json(&[italian()]);
        assert!(json.contains("\"e2e4\""));
        assert!(json.contains("\"Alpha\""));
        let uci = encode_uci(&[italian()]);
        assert!(uci.contains("moves e2e4 e7e5 g1f3"));
    }

    #[test]
    fn import_pgn_and_fen_text() {
        let games =
            import_text("[Event \"X\"]\n[White \"A\"]\n[Black \"B\"]\n\n1. e4 e5 1-0\n").unwrap();
        assert_eq!(games[0].moves.len(), 2);
        let fen = import_text(START_FEN).unwrap();
        assert_eq!(fen[0].moves.len(), 0);
        assert!(fen[0].initial_fen.contains("rnbqkbnr"));
    }

    #[test]
    fn format_is_inferred_from_extension() {
        assert_eq!(
            GameExportFormat::from_path(Path::new("games.binpack")),
            GameExportFormat::Binpack
        );
        assert_eq!(
            GameExportFormat::from_path(Path::new("games.pgn")),
            GameExportFormat::Pgn
        );
    }
}
