//! Text dataset used by datagen and the Ateed trainer: `FEN|score|wdl`.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::datagen::TrainingPosition;

/// Parse one datagen line. Empty lines and `#` comments are ignored.
pub fn parse_training_line(line: &str) -> Result<Option<TrainingPosition>, String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let mut parts = line.rsplitn(3, '|');
    let wdl = parts
        .next()
        .ok_or_else(|| format!("missing WDL in `{line}`"))?;
    let score = parts
        .next()
        .ok_or_else(|| format!("missing score in `{line}`"))?;
    let fen = parts
        .next()
        .ok_or_else(|| format!("missing FEN in `{line}`"))?;
    let score = score
        .parse::<i32>()
        .map_err(|error| format!("invalid score `{score}`: {error}"))?;
    let wdl = wdl
        .parse::<f32>()
        .map_err(|error| format!("invalid WDL `{wdl}`: {error}"))?;
    if !(0.0..=1.0).contains(&wdl) {
        return Err(format!("WDL `{wdl}` is outside 0..=1"));
    }
    Ok(Some(TrainingPosition {
        fen: fen.to_string(),
        score,
        wdl,
    }))
}

pub fn fetch_dataset(url: &str, dest: &Path) -> Result<crate::ingest::IngestReport, String> {
    crate::ingest::fetch_and_ingest(url, dest)
}

pub fn download_dataset(url: &str, dest: &Path) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("dataset URL must be http(s)".to_string());
    }
    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    updater::download::download_url_with_progress(url, dest, None, |bytes, total| {
        updater::progress::emit_progress(&updater::progress::JobProgress::fetch(bytes, total));
    })
}

pub fn load_training_positions(path: &Path) -> io::Result<Vec<TrainingPosition>> {
    crate::formats::load_positions_from_path(path)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub fn load_mixed_positions(
    data_path: &str,
    mix_weights: &str,
    seed: u64,
) -> Result<Vec<TrainingPosition>, String> {
    let paths = crate::merge::parse_csv_list(data_path);
    if paths.is_empty() {
        return Err("pass at least one dataset path".into());
    }
    let weights = crate::merge::parse_mix_weights(mix_weights, paths.len())?;
    let mut sources = Vec::with_capacity(paths.len());
    for path in &paths {
        sources.push(crate::formats::load_positions_from_path(Path::new(path))?);
    }
    let merged = if sources.len() == 1 {
        sources.remove(0)
    } else {
        crate::merge::merge_weighted(&sources, &weights, seed)?
    };
    Ok(dedupe_positions(merged))
}

pub fn dedupe_positions(positions: Vec<TrainingPosition>) -> Vec<TrainingPosition> {
    let mut seen = HashSet::new();
    positions
        .into_iter()
        .filter(|position| seen.insert(mujrim_study::ateed_index::position_key_hash(&position.fen)))
        .collect()
}

pub const GENDATA_DIR: &str = "gendata";

/// Put relative `data.txt` under `./gendata` and make the path absolute when possible.
pub fn relocate_datagen_output(path: &str) -> String {
    let raw = Path::new(path);
    let file_name = raw
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data.txt");
    let under_gendata = raw
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        == Some(GENDATA_DIR);
    let relocated = if raw.is_absolute() && under_gendata {
        raw.to_path_buf()
    } else if raw.is_absolute() {
        raw.parent()
            .unwrap_or(Path::new("/"))
            .join(GENDATA_DIR)
            .join(file_name)
    } else if raw
        .components()
        .next()
        .is_some_and(|component| component.as_os_str() == GENDATA_DIR)
    {
        raw.to_path_buf()
    } else {
        PathBuf::from(GENDATA_DIR).join(file_name)
    };
    std::env::current_dir()
        .ok()
        .map(|cwd| {
            if relocated.is_absolute() {
                relocated.clone()
            } else {
                cwd.join(&relocated)
            }
        })
        .unwrap_or(relocated)
        .to_string_lossy()
        .into_owned()
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct CompactReport {
    pub read: u64,
    pub written: u64,
    pub duplicates: u64,
    pub invalid: u64,
    pub white: u64,
    pub draw: u64,
    pub black: u64,
}

/// Stream one or more `FEN|score|wdl` files, drop torn/invalid lines, keep the first
/// unique board (clocks ignored), and write a clean dataset.
pub fn compact_datagen_files(inputs: &[&Path], output: &Path) -> Result<CompactReport, String> {
    if inputs.is_empty() {
        return Err("pass at least one datagen file".into());
    }
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let staging = output.with_extension("txt.compact");
    let mut writer = BufWriter::with_capacity(
        1 << 20,
        File::create(&staging).map_err(|error| error.to_string())?,
    );
    let mut seen = HashSet::new();
    let estimated_lines = inputs
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .map(|meta| (meta.len() / 72) as usize)
        .sum::<usize>();
    if estimated_lines > 0 {
        seen.reserve(estimated_lines);
    }
    let mut report = CompactReport::default();
    for input in inputs {
        let file = File::open(input).map_err(|error| format!("{}: {error}", input.display()))?;
        let reader = BufReader::with_capacity(1 << 20, file);
        for line in reader.lines() {
            let line = match line {
                Ok(line) => line,
                Err(_) => {
                    report.invalid += 1;
                    continue;
                }
            };
            match parse_training_line(&line) {
                Ok(None) => {}
                Ok(Some(position)) => {
                    report.read += 1;
                    if !seen.insert(mujrim_study::ateed_index::position_key_hash(&position.fen)) {
                        report.duplicates += 1;
                        continue;
                    }
                    if position.wdl >= 0.75 {
                        report.white += 1;
                    } else if position.wdl <= 0.25 {
                        report.black += 1;
                    } else {
                        report.draw += 1;
                    }
                    writeln!(
                        writer,
                        "{}|{}|{:.1}",
                        position.fen, position.score, position.wdl
                    )
                    .map_err(|error| error.to_string())?;
                    report.written += 1;
                    if report.read % 5_000_000 == 0 {
                        eprintln!(
                            "compact progress: {} read / {} unique / {} dup / {} bad",
                            report.read, report.written, report.duplicates, report.invalid
                        );
                    }
                }
                Err(_) => report.invalid += 1,
            }
        }
    }
    writer.flush().map_err(|error| error.to_string())?;
    drop(writer);
    fs::rename(&staging, output).map_err(|error| {
        let _ = fs::remove_file(&staging);
        error.to_string()
    })?;
    Ok(report)
}

pub fn fetch_catalog_dataset(
    id: Option<&str>,
    url: &str,
    dest: &Path,
) -> Result<crate::ingest::IngestReport, String> {
    let (url, filename) = updater::datasets::resolve_fetch_url(id, url)?;
    let dest = if dest.as_os_str().is_empty() {
        Path::new(&filename)
    } else {
        dest
    };
    crate::ingest::fetch_and_ingest(&url, dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_training_line_reads_fen_score_and_wdl() {
        let position =
            parse_training_line("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1|12|0.5")
                .expect("valid line")
                .expect("not a comment");
        assert!(position.fen.starts_with("rnbqkbnr/"));
        assert_eq!(position.score, 12);
        assert_eq!(position.wdl, 0.5);
    }

    #[test]
    fn parse_training_line_skips_comments_and_rejects_bad_wdl() {
        assert_eq!(parse_training_line("# ignore").unwrap(), None);
        assert_eq!(parse_training_line("   ").unwrap(), None);
        assert!(parse_training_line("fen|1|1.5").is_err());
        assert!(parse_training_line("fen|x|0.5").is_err());
    }

    #[test]
    fn dedupe_positions_collapses_clock_transpositions() {
        let a = TrainingPosition {
            fen: "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1".into(),
            score: 10,
            wdl: 0.5,
        };
        let b = TrainingPosition {
            fen: "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 5 8".into(),
            score: 20,
            wdl: 1.0,
        };
        let c = TrainingPosition {
            fen: "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1".into(),
            score: 0,
            wdl: 0.5,
        };
        let out = dedupe_positions(vec![a, b, c]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].score, 10);
    }

    #[test]
    fn relocate_datagen_output_puts_legacy_files_under_gendata() {
        let relocated = relocate_datagen_output("data.txt");
        assert!(relocated.ends_with("gendata/data.txt"), "{relocated}");
        assert!(!relocated.ends_with("/data.txt/data.txt"));
        let already = relocate_datagen_output("/tmp/gendata/data.txt");
        assert_eq!(already, "/tmp/gendata/data.txt");
    }

    #[test]
    fn compact_datagen_files_drops_duplicates_and_torn_lines() {
        let dir = std::env::temp_dir().join(format!(
            "mujrim-compact-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        let out = dir.join("gendata").join("data.txt");
        fs::write(
            &a,
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1|10|0.5\nnot-a-line\n",
        )
        .unwrap();
        fs::write(
            &b,
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 12 40|99|1.0\nrnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1|0|0.0\n",
        )
        .unwrap();
        let report = compact_datagen_files(&[&a, &b], &out).expect("compact");
        assert_eq!(report.written, 2);
        assert_eq!(report.duplicates, 1);
        assert_eq!(report.invalid, 1);
        assert_eq!(report.draw, 1);
        assert_eq!(report.black, 1);
        let text = fs::read_to_string(&out).unwrap();
        assert!(text.contains("|10|0.5"));
        assert!(!text.contains("|99|1.0"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_training_positions_reads_a_file() {
        let path = std::env::temp_dir().join("mujrim-ateed-dataset.txt");
        std::fs::write(
            &path,
            "# header\nrnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1|0|0.5\n",
        )
        .unwrap();
        let positions = load_training_positions(&path).expect("load dataset");
        let _ = std::fs::remove_file(&path);
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].score, 0);
    }

    #[test]
    fn fetch_dataset_rejects_non_http_urls() {
        let dest = std::env::temp_dir().join("mujrim-dataset-local.txt");
        assert!(
            fetch_dataset("file:///tmp/data.txt", &dest)
                .unwrap_err()
                .contains("http(s)")
        );
        assert!(
            fetch_catalog_dataset(Some("stockfish-plain"), "", &dest)
                .unwrap_err()
                .contains("directory")
        );
    }

    #[test]
    fn load_mixed_positions_interleaves_two_text_files() {
        let dir = std::env::temp_dir();
        let a = dir.join(format!("mujrim-mix-a-{}.txt", std::process::id()));
        let b = dir.join(format!("mujrim-mix-b-{}.txt", std::process::id()));
        std::fs::write(
            &a,
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1|1|1.0\n",
        )
        .unwrap();
        std::fs::write(
            &b,
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1|2|0.0\n",
        )
        .unwrap();
        let mixed =
            load_mixed_positions(&format!("{},{}", a.display(), b.display()), "1,1", 1).unwrap();
        let same =
            load_mixed_positions(&format!("{},{}", a.display(), a.display()), "1,1", 1).unwrap();
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
        assert_eq!(mixed.len(), 2);
        let scores: Vec<i32> = mixed.iter().map(|p| p.score).collect();
        assert!(scores.contains(&1) && scores.contains(&2));
        assert_eq!(same.len(), 1);
    }
}
