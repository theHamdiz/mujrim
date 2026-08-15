//! Text dataset used by datagen and the Ateed trainer: `FEN|score|wdl`.

use std::io::{self, BufRead};
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

pub fn fetch_dataset(url: &str, dest: &Path) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("dataset URL must be http(s)".to_string());
    }
    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    updater::download::download_url(url, dest, None)
}

pub fn load_training_positions(path: &Path) -> io::Result<Vec<TrainingPosition>> {
    let file = std::fs::File::open(path)?;
    let reader = io::BufReader::new(file);
    let mut positions = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line?;
        match parse_training_line(&line) {
            Ok(Some(position)) => positions.push(position),
            Ok(None) => {}
            Err(error) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("line {}: {error}", index + 1),
                ));
            }
        }
    }
    Ok(positions)
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
    }
}
