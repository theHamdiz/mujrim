//! Text dataset used by datagen and the Ateed trainer: `FEN|score|wdl`.

use std::io;
use std::path::Path;

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
    let mut seen = std::collections::HashSet::new();
    positions
        .into_iter()
        .filter(|position| seen.insert(mujrim_study::ateed_index::position_key(&position.fen)))
        .collect()
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
