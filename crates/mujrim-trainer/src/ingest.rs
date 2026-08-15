//! Download → decompress → decode → train-ready `FEN|score|wdl` in one step.

use std::path::{Path, PathBuf};

use crate::datagen::TrainingPosition;
use crate::formats::{self, DatasetFormat};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestReport {
    pub raw_path: PathBuf,
    pub ready_path: PathBuf,
    pub positions: usize,
    pub converted: bool,
}

pub fn ready_path_for(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data");
    if name.ends_with(".txt") || name.ends_with(".plain") {
        dest.to_path_buf()
    } else {
        let stem = name
            .trim_end_matches(".gz")
            .trim_end_matches(".zst")
            .trim_end_matches(".binpack")
            .trim_end_matches(".bin")
            .trim_end_matches(".pgn")
            .trim_end_matches(".plain");
        let stem = if stem.is_empty() { "data" } else { stem };
        dest.with_file_name(format!("{stem}.txt"))
    }
}

pub fn ingest_file(raw: &Path, ready: &Path) -> Result<IngestReport, String> {
    let positions = formats::load_positions_from_path(raw)?;
    if positions.is_empty() {
        return Err("ingest produced no training positions".into());
    }
    let already_ready = raw == ready && is_mujrim_text(raw);
    if !already_ready {
        formats::write_positions(ready, &positions, DatasetFormat::MujrimText)?;
    }
    Ok(IngestReport {
        raw_path: raw.to_path_buf(),
        ready_path: ready.to_path_buf(),
        positions: positions.len(),
        converted: !already_ready,
    })
}

pub fn fetch_and_ingest(url: &str, dest: &Path) -> Result<IngestReport, String> {
    crate::dataset::download_dataset(url, dest)?;
    let ready = ready_path_for(dest);
    ingest_file(dest, &ready)
}

pub fn ingest_bytes(bytes: &[u8]) -> Result<Vec<TrainingPosition>, String> {
    formats::decode_bytes(bytes)
}

fn is_mujrim_text(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    text.lines().any(|line| {
        crate::dataset::parse_training_line(line)
            .ok()
            .flatten()
            .is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::gzip_bytes;

    fn start_line() -> &'static str {
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1|12|0.5\n"
    }

    #[test]
    fn ready_path_strips_compression_suffixes() {
        assert_eq!(
            ready_path_for(Path::new("dumps/training.1.gz")),
            Path::new("dumps/training.1.txt")
        );
        assert_eq!(ready_path_for(Path::new("data.txt")), Path::new("data.txt"));
    }

    #[test]
    fn ingest_converts_gzip_text_to_plain_dataset() {
        let dir = std::env::temp_dir();
        let raw = dir.join(format!("mujrim-ingest-{}.gz", std::process::id()));
        let ready = dir.join(format!("mujrim-ingest-{}.txt", std::process::id()));
        std::fs::write(&raw, gzip_bytes(start_line().as_bytes()).unwrap()).unwrap();
        let report = ingest_file(&raw, &ready).unwrap();
        let text = std::fs::read_to_string(&ready).unwrap();
        let _ = std::fs::remove_file(&raw);
        let _ = std::fs::remove_file(&ready);
        assert!(report.converted);
        assert_eq!(report.positions, 1);
        assert!(text.contains("|12|"));
    }

    #[test]
    fn ingest_skips_rewrite_when_already_mujrim_text() {
        let path =
            std::env::temp_dir().join(format!("mujrim-ingest-ready-{}.txt", std::process::id()));
        std::fs::write(&path, start_line()).unwrap();
        let report = ingest_file(&path, &path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(!report.converted);
        assert_eq!(report.positions, 1);
    }
}
