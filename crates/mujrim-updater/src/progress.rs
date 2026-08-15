//! Machine-readable progress lines for CLI jobs the GUI can tail.

use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Fetch,
    Train,
    Datagen,
}

impl JobKind {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "fetch" => Some(Self::Fetch),
            "train" => Some(Self::Train),
            "datagen" => Some(Self::Datagen),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Train => "train",
            Self::Datagen => "datagen",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JobProgress {
    pub kind: JobKind,
    pub pct: f32,
    pub epoch: Option<u32>,
    pub epochs: Option<u32>,
    pub loss: Option<f32>,
    pub expert: Option<usize>,
    pub bytes: Option<u64>,
    pub total: Option<u64>,
    pub game: Option<u64>,
    pub games: Option<u64>,
    pub positions: Option<u64>,
}

impl JobProgress {
    pub fn fetch(bytes: u64, total: Option<u64>) -> Self {
        Self {
            kind: JobKind::Fetch,
            pct: ratio(bytes as f32, total.map(|value| value as f32)),
            epoch: None,
            epochs: None,
            loss: None,
            expert: None,
            bytes: Some(bytes),
            total,
            game: None,
            games: None,
            positions: None,
        }
    }

    pub fn train(epoch: u32, epochs: u32, loss: f32, expert: usize) -> Self {
        Self {
            kind: JobKind::Train,
            pct: ratio(epoch as f32, Some(epochs as f32)),
            epoch: Some(epoch),
            epochs: Some(epochs),
            loss: Some(loss),
            expert: Some(expert),
            bytes: None,
            total: None,
            game: None,
            games: None,
            positions: None,
        }
    }

    pub fn datagen(game: u64, games: u64, positions: u64) -> Self {
        Self {
            kind: JobKind::Datagen,
            pct: ratio(game as f32, Some(games as f32)),
            epoch: None,
            epochs: None,
            loss: None,
            expert: None,
            bytes: None,
            total: None,
            game: Some(game),
            games: Some(games),
            positions: Some(positions),
        }
    }
}

pub fn format_progress(progress: &JobProgress) -> String {
    let mut line = format!(
        "progress kind={} pct={:.1}",
        progress.kind.as_str(),
        progress.pct
    );
    append_opt(&mut line, "epoch", progress.epoch);
    append_opt(&mut line, "epochs", progress.epochs);
    if let Some(loss) = progress.loss {
        let _ = write!(line, " loss={loss:.4}");
    }
    append_opt(&mut line, "expert", progress.expert);
    append_opt(&mut line, "bytes", progress.bytes);
    append_opt(&mut line, "total", progress.total);
    append_opt(&mut line, "game", progress.game);
    append_opt(&mut line, "games", progress.games);
    append_opt(&mut line, "positions", progress.positions);
    line
}

pub fn emit_progress(progress: &JobProgress) {
    use std::io::Write;
    println!("{}", format_progress(progress));
    let _ = std::io::stdout().flush();
}

pub fn parse_progress_line(line: &str) -> Option<JobProgress> {
    let line = line.trim();
    let mut parts = line.split_whitespace();
    if parts.next()? != "progress" {
        return None;
    }
    let mut kind = None;
    let mut pct = 0.0;
    let mut progress = JobProgress {
        kind: JobKind::Fetch,
        pct: 0.0,
        epoch: None,
        epochs: None,
        loss: None,
        expert: None,
        bytes: None,
        total: None,
        game: None,
        games: None,
        positions: None,
    };
    for part in parts {
        let (key, value) = part.split_once('=')?;
        match key {
            "kind" => kind = JobKind::parse(value),
            "pct" => pct = value.parse().ok()?,
            "epoch" => progress.epoch = value.parse().ok(),
            "epochs" => progress.epochs = value.parse().ok(),
            "loss" => progress.loss = value.parse().ok(),
            "expert" => progress.expert = value.parse().ok(),
            "bytes" => progress.bytes = value.parse().ok(),
            "total" => progress.total = value.parse().ok(),
            "game" => progress.game = value.parse().ok(),
            "games" => progress.games = value.parse().ok(),
            "positions" => progress.positions = value.parse().ok(),
            _ => {}
        }
    }
    progress.kind = kind?;
    progress.pct = pct;
    Some(progress)
}

pub fn should_report_step(completed: u64, total: u64) -> bool {
    if completed == 0 || completed == total {
        return true;
    }
    if total <= 32 {
        return true;
    }
    let stride = (total / 100).max(1);
    completed.is_multiple_of(stride)
}

fn ratio(done: f32, total: Option<f32>) -> f32 {
    match total {
        Some(total) if total > 0.0 => ((done / total) * 100.0).clamp(0.0, 100.0),
        _ => 0.0,
    }
}

fn append_opt<T: std::fmt::Display>(line: &mut String, key: &str, value: Option<T>) {
    if let Some(value) = value {
        let _ = write!(line, " {key}={value}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines_roundtrip_for_each_job() {
        let fetch = JobProgress::fetch(25, Some(100));
        let train = JobProgress::train(2, 8, 0.3125, 1);
        let datagen = JobProgress::datagen(4, 16, 80);
        for original in [fetch, train, datagen] {
            let parsed = parse_progress_line(&format_progress(&original)).expect("parse");
            assert_eq!(parsed.kind, original.kind);
            assert!((parsed.pct - original.pct).abs() < 0.15);
            assert_eq!(parsed.epoch, original.epoch);
            assert_eq!(parsed.game, original.game);
        }
        assert!(parse_progress_line("info string hello").is_none());
        assert!(parse_progress_line("progress pct=10").is_none());
    }

    #[test]
    fn datagen_reports_every_game_on_short_runs() {
        assert!(should_report_step(1, 8));
        assert!(should_report_step(8, 8));
        assert!(should_report_step(0, 200));
        assert!(should_report_step(200, 200));
        assert!(should_report_step(2, 200));
        assert!(!should_report_step(1, 200));
    }
}
