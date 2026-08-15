//! Detect, decompress, and decode training dumps into `FEN|score|wdl` positions.

use std::io::{Read, Write};
use std::path::Path;

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use mujrim_study::game_export::{self, GameRecord};
use mujrim_study::pgn;

use crate::datagen::TrainingPosition;
use crate::dataset::parse_training_line;

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
const STOCKFISH_BINP: &[u8; 4] = b"BINP";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetFormat {
    MujrimText,
    StockfishPlain,
    MujrimBinpack,
    Pgn,
}

impl DatasetFormat {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "text" | "txt" | "mujrim" => Ok(Self::MujrimText),
            "plain" | "stockfish" => Ok(Self::StockfishPlain),
            "binpack" | "mjbp" => Ok(Self::MujrimBinpack),
            "pgn" => Ok(Self::Pgn),
            other => Err(format!("unknown dataset format `{other}`")),
        }
    }
}

pub fn load_positions_from_path(path: &Path) -> Result<Vec<TrainingPosition>, String> {
    let file = std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    decode_read(file)
}

pub fn decode_bytes(bytes: &[u8]) -> Result<Vec<TrainingPosition>, String> {
    decode_read(bytes)
}

pub fn decode_read(reader: impl Read) -> Result<Vec<TrainingPosition>, String> {
    decode_boxed(Box::new(reader))
}

fn decode_boxed(mut reader: Box<dyn Read + '_>) -> Result<Vec<TrainingPosition>, String> {
    loop {
        let mut magic = [0u8; 4];
        let n = reader
            .read(&mut magic)
            .map_err(|error| format!("failed to read dataset header: {error}"))?;
        if n >= 2 && magic.starts_with(&GZIP_MAGIC) {
            reader = Box::new(GzDecoder::new(
                std::io::Cursor::new(magic[..n].to_vec()).chain(reader),
            ));
            continue;
        }
        if n == 4 && magic == ZSTD_MAGIC {
            let decoder = zstd::stream::Decoder::new(std::io::Cursor::new(magic).chain(reader))
                .map_err(|error| format!("zstd decoder failed: {error}"))?;
            reader = Box::new(decoder);
            continue;
        }
        if n == 4 && crate::lc0::record_size(u32::from_le_bytes(magic)).is_some() {
            return crate::lc0::decode_stream(magic, reader);
        }
        let mut rest = magic[..n].to_vec();
        reader
            .read_to_end(&mut rest)
            .map_err(|error| format!("failed to read dataset: {error}"))?;
        return decode_uncompressed(&rest);
    }
}

fn decode_uncompressed(bytes: &[u8]) -> Result<Vec<TrainingPosition>, String> {
    if bytes.starts_with(STOCKFISH_BINP) {
        return Err("Stockfish training uses .plain or .plain.gz, not chained BINP".into());
    }
    if bytes.starts_with(game_export::BINPACK_MAGIC) {
        return decode_mujrim_binpack(bytes);
    }
    if crate::lc0::looks_like_lc0(bytes) {
        return crate::lc0::decode_records(bytes);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| {
        "dataset is not a known training dump after decompress (need text, .plain, MJBP, PGN, or Lc0 v3–v6)".to_string()
    })?;
    decode_text(text)
}

pub fn decode_text(text: &str) -> Result<Vec<TrainingPosition>, String> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('[') || trimmed.contains("\n[Event") {
        return decode_pgn(text);
    }
    if looks_like_plain(text) {
        return decode_stockfish_plain(text);
    }
    let mut positions = Vec::new();
    for (index, line) in text.lines().enumerate() {
        match parse_training_line(line) {
            Ok(Some(position)) => positions.push(position),
            Ok(None) => {}
            Err(error) => return Err(format!("line {}: {error}", index + 1)),
        }
    }
    Ok(positions)
}

pub fn decode_stockfish_plain(text: &str) -> Result<Vec<TrainingPosition>, String> {
    let mut positions = Vec::new();
    let mut fen = None;
    let mut score = 0i32;
    let mut result = 0.5f32;
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line == "e" {
            let fen = fen
                .take()
                .ok_or_else(|| format!("plain record ending at line {} has no fen", index + 1))?;
            positions.push(TrainingPosition {
                fen,
                score,
                wdl: result,
            });
            score = 0;
            result = 0.5;
            continue;
        }
        let (key, value) = line
            .split_once(' ')
            .ok_or_else(|| format!("invalid plain line {}: `{line}`", index + 1))?;
        match key {
            "fen" => fen = Some(value.to_string()),
            "score" => {
                score = value
                    .parse()
                    .map_err(|error| format!("invalid score on line {}: {error}", index + 1))?;
            }
            "result" => result = parse_plain_result(value)?,
            "move" | "ply" => {}
            other => return Err(format!("unknown plain key `{other}` on line {}", index + 1)),
        }
    }
    if fen.is_some() {
        return Err("truncated Stockfish plain record".into());
    }
    Ok(positions)
}

pub fn encode_stockfish_plain(positions: &[TrainingPosition]) -> String {
    let mut out = String::new();
    for (index, position) in positions.iter().enumerate() {
        let result = if position.wdl >= 0.75 {
            1.0
        } else if position.wdl <= 0.25 {
            -1.0
        } else {
            0.0
        };
        out.push_str(&format!(
            "fen {}\nmove 0000\nscore {}\nply {}\nresult {result}\ne\n",
            position.fen,
            position.score,
            index + 1
        ));
    }
    out
}

pub fn encode_mujrim_text(positions: &[TrainingPosition]) -> String {
    let mut out = String::new();
    for position in positions {
        out.push_str(&format!(
            "{}|{}|{:.3}\n",
            position.fen, position.score, position.wdl
        ));
    }
    out
}

pub fn write_positions(
    path: &Path,
    positions: &[TrainingPosition],
    format: DatasetFormat,
) -> Result<(), String> {
    let bytes = match format {
        DatasetFormat::MujrimText => encode_mujrim_text(positions).into_bytes(),
        DatasetFormat::StockfishPlain => encode_stockfish_plain(positions).into_bytes(),
        DatasetFormat::MujrimBinpack => {
            let records = positions
                .iter()
                .enumerate()
                .map(|(index, position)| game_export::TrainingPosition {
                    fen: position.fen.clone(),
                    mv: "0000".into(),
                    score_cp: position.score.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                    ply: (index + 1) as u16,
                    wdl: if position.wdl >= 0.75 {
                        1
                    } else if position.wdl <= 0.25 {
                        -1
                    } else {
                        0
                    },
                })
                .collect::<Vec<_>>();
            game_export::encode_positions_binpack(&records)?
        }
        DatasetFormat::Pgn => {
            return Err("writing PGN from scored positions is not supported".into());
        }
    };
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let payload = if name.ends_with(".gz") {
        gzip_bytes(&bytes)?
    } else if name.ends_with(".zst") {
        zstd::encode_all(bytes.as_slice(), 0)
            .map_err(|error| format!("zstd compress failed: {error}"))?
    } else {
        bytes
    };
    std::fs::write(path, payload).map_err(|error| error.to_string())
}

pub fn gzip_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = GzEncoder::new(&mut out, Compression::default());
        encoder
            .write_all(bytes)
            .map_err(|error| format!("gzip compress failed: {error}"))?;
        encoder
            .finish()
            .map_err(|error| format!("gzip finish failed: {error}"))?;
    }
    Ok(out)
}

fn decode_mujrim_binpack(bytes: &[u8]) -> Result<Vec<TrainingPosition>, String> {
    game_export::decode_binpack(bytes)
        .map(|records| records.into_iter().map(from_study_position).collect())
}

fn decode_pgn(text: &str) -> Result<Vec<TrainingPosition>, String> {
    let games = pgn::parse_games(text)?;
    let records: Vec<GameRecord> = games.into_iter().map(GameRecord::from_parsed).collect();
    let encoded = game_export::encode_games(&records, game_export::GameExportFormat::Binpack)?;
    decode_mujrim_binpack(&encoded)
}

fn from_study_position(position: game_export::TrainingPosition) -> TrainingPosition {
    TrainingPosition {
        fen: position.fen,
        score: i32::from(position.score_cp),
        wdl: match position.wdl {
            1 => 1.0,
            -1 => 0.0,
            _ => 0.5,
        },
    }
}

fn looks_like_plain(text: &str) -> bool {
    let mut has_fen = false;
    let mut has_end = false;
    for line in text.lines().take(32) {
        let line = line.trim();
        has_fen |= line.starts_with("fen ");
        has_end |= line == "e";
    }
    has_fen && has_end
}

fn parse_plain_result(value: &str) -> Result<f32, String> {
    match value {
        "1-0" | "1.0" | "1" => Ok(1.0),
        "0-1" | "-1.0" | "-1" => Ok(0.0),
        "1/2-1/2" | "0.5" | "0.0" | "0" => Ok(0.5),
        other => {
            let parsed = other
                .parse::<f32>()
                .map_err(|error| format!("invalid plain result `{other}`: {error}"))?;
            if parsed < 0.0 {
                Ok(0.0)
            } else if parsed > 1.0 {
                Ok(1.0)
            } else {
                Ok(parsed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn startpos(score: i32, wdl: f32) -> TrainingPosition {
        TrainingPosition {
            fen: "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1".into(),
            score,
            wdl,
        }
    }

    #[test]
    fn stockfish_plain_roundtrips_score_and_wdl() {
        let encoded = encode_stockfish_plain(&[startpos(42, 1.0), startpos(-8, 0.0)]);
        let decoded = decode_stockfish_plain(&encoded).expect("plain");
        assert_eq!(decoded[0].score, 42);
        assert_eq!(decoded[0].wdl, 1.0);
        assert_eq!(decoded[1].wdl, 0.0);
    }

    #[test]
    fn gzip_text_and_binp_magic_are_detected() {
        let text = encode_mujrim_text(&[startpos(3, 0.5)]);
        let gz = gzip_bytes(text.as_bytes()).expect("gzip");
        let decoded = decode_bytes(&gz).expect("decode gzip");
        assert_eq!(decoded[0].score, 3);
        assert!(decode_bytes(b"BINPxxxx").unwrap_err().contains(".plain"));
    }

    #[test]
    fn gzip_lc0_v6_chunk_decodes_without_a_separate_step() {
        let record = crate::lc0::encode_v6_startpos(0.0, 0.5);
        let gz = gzip_bytes(&record).expect("gzip lc0");
        let decoded = decode_bytes(&gz).expect("decode lc0 gzip");
        assert_eq!(
            decoded[0].fen,
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        );
        assert_eq!(decoded[0].wdl, 0.75);
    }

    #[test]
    fn mujrim_binpack_and_zstd_roundtrip() {
        let positions = [startpos(9, 1.0)];
        let dir = std::env::temp_dir();
        let bin = dir.join(format!("mujrim-fmt-{}.binpack", std::process::id()));
        let zst = dir.join(format!("mujrim-fmt-{}.txt.zst", std::process::id()));
        write_positions(&bin, &positions, DatasetFormat::MujrimBinpack).unwrap();
        write_positions(&zst, &positions, DatasetFormat::MujrimText).unwrap();
        let from_bin = load_positions_from_path(&bin).unwrap();
        let from_zst = load_positions_from_path(&zst).unwrap();
        let _ = std::fs::remove_file(&bin);
        let _ = std::fs::remove_file(&zst);
        assert_eq!(from_bin[0].score, 9);
        assert_eq!(from_bin[0].wdl, 1.0);
        assert_eq!(from_zst[0].score, 9);
    }

    #[test]
    fn pgn_selfplay_becomes_training_positions() {
        let pgn = r#"
[Event "Self"]
[White "A"]
[Black "B"]
[Result "1-0"]

1. e4 e5 2. Nf3 1-0
"#;
        let positions = decode_text(pgn).expect("pgn");
        assert!(positions.len() >= 2);
        assert_eq!(positions[0].wdl, 1.0);
    }
}
