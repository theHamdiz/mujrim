//! Machine-readable progress lines for CLI jobs the GUI can tail.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

pub const HIST_BUCKETS: usize = 16;
pub const REPORT_INTERVAL: Duration = Duration::from_millis(250);

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DatagenBatch {
    pub game: u64,
    pub games: u64,
    pub positions: u64,
    pub nps: u64,
    pub throughput: f32,
    pub white: u64,
    pub draw: u64,
    pub black: u64,
    pub pass: u64,
    pub drop: u64,
    pub bytes: u64,
    pub hist: [u32; HIST_BUCKETS],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrainerBatch {
    pub epoch: u32,
    pub epochs: u32,
    pub loss: f32,
    pub val_loss: Option<f32>,
    pub expert: usize,
    pub lr: f32,
    pub mpos: f32,
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
    pub nps: Option<u64>,
    pub throughput: Option<f32>,
    pub white: Option<u64>,
    pub draw: Option<u64>,
    pub black: Option<u64>,
    pub pass: Option<u64>,
    pub drop: Option<u64>,
    pub hist: Option<[u32; HIST_BUCKETS]>,
    pub val_loss: Option<f32>,
    pub lr: Option<f32>,
    pub mpos: Option<f32>,
}

impl JobProgress {
    fn blank(kind: JobKind) -> Self {
        Self {
            kind,
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
            nps: None,
            throughput: None,
            white: None,
            draw: None,
            black: None,
            pass: None,
            drop: None,
            hist: None,
            val_loss: None,
            lr: None,
            mpos: None,
        }
    }

    pub fn fetch(bytes: u64, total: Option<u64>) -> Self {
        let mut progress = Self::blank(JobKind::Fetch);
        progress.pct = ratio(bytes as f32, total.map(|value| value as f32));
        progress.bytes = Some(bytes);
        progress.total = total;
        progress
    }

    pub fn train(epoch: u32, epochs: u32, loss: f32, expert: usize) -> Self {
        let mut progress = Self::blank(JobKind::Train);
        progress.pct = ratio(epoch as f32, Some(epochs as f32));
        progress.epoch = Some(epoch);
        progress.epochs = Some(epochs);
        progress.loss = Some(loss);
        progress.expert = Some(expert);
        progress
    }

    pub fn train_batch(batch: TrainerBatch) -> Self {
        let mut progress = Self::train(batch.epoch, batch.epochs, batch.loss, batch.expert);
        progress.val_loss = batch.val_loss;
        progress.lr = Some(batch.lr);
        progress.mpos = Some(batch.mpos);
        progress.throughput = Some(batch.mpos);
        progress
    }

    pub fn datagen(game: u64, games: u64, positions: u64) -> Self {
        let mut progress = Self::blank(JobKind::Datagen);
        progress.pct = ratio(game as f32, Some(games as f32));
        progress.game = Some(game);
        progress.games = Some(games);
        progress.positions = Some(positions);
        progress
    }

    pub fn datagen_batch(batch: DatagenBatch) -> Self {
        let mut progress = Self::datagen(batch.game, batch.games, batch.positions);
        progress.nps = Some(batch.nps);
        progress.throughput = Some(batch.throughput);
        progress.white = Some(batch.white);
        progress.draw = Some(batch.draw);
        progress.black = Some(batch.black);
        progress.pass = Some(batch.pass);
        progress.drop = Some(batch.drop);
        progress.bytes = Some(batch.bytes);
        progress.hist = Some(batch.hist);
        progress
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
    append_opt(&mut line, "nps", progress.nps);
    if let Some(throughput) = progress.throughput {
        let _ = write!(line, " throughput={throughput:.4}");
    }
    append_opt(&mut line, "white", progress.white);
    append_opt(&mut line, "draw", progress.draw);
    append_opt(&mut line, "black", progress.black);
    append_opt(&mut line, "pass", progress.pass);
    append_opt(&mut line, "drop", progress.drop);
    if let Some(hist) = progress.hist {
        let _ = write!(line, " hist=");
        for (index, bucket) in hist.iter().enumerate() {
            if index > 0 {
                line.push(',');
            }
            let _ = write!(line, "{bucket}");
        }
    }
    if let Some(val_loss) = progress.val_loss {
        let _ = write!(line, " val_loss={val_loss:.4}");
    }
    if let Some(lr) = progress.lr {
        let _ = write!(line, " lr={lr:.6}");
    }
    if let Some(mpos) = progress.mpos {
        let _ = write!(line, " mpos={mpos:.6}");
    }
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
    let mut progress = JobProgress::blank(JobKind::Fetch);
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
            "nps" => progress.nps = value.parse().ok(),
            "throughput" => progress.throughput = value.parse().ok(),
            "white" => progress.white = value.parse().ok(),
            "draw" => progress.draw = value.parse().ok(),
            "black" => progress.black = value.parse().ok(),
            "pass" => progress.pass = value.parse().ok(),
            "drop" => progress.drop = value.parse().ok(),
            "hist" => progress.hist = Some(parse_hist(value)),
            "val_loss" => progress.val_loss = value.parse().ok(),
            "lr" => progress.lr = value.parse().ok(),
            "mpos" => progress.mpos = value.parse().ok(),
            _ => {}
        }
    }
    progress.kind = kind?;
    progress.pct = pct;
    Some(progress)
}

pub fn parse_hist(value: &str) -> [u32; HIST_BUCKETS] {
    let mut hist = [0u32; HIST_BUCKETS];
    for (slot, part) in hist.iter_mut().zip(value.split(',')) {
        if let Ok(count) = part.parse() {
            *slot = count;
        }
    }
    hist
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

pub fn should_report_now(completed: u64, total: u64, last_emit: Instant, now: Instant) -> bool {
    if completed == 0 || completed == total {
        return true;
    }
    should_report_step(completed, total) && now.duration_since(last_emit) >= REPORT_INTERVAL
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
    fn datagen_batch_roundtrips_telemetry_fields() {
        let mut hist = [0u32; HIST_BUCKETS];
        hist[0] = 3;
        hist[15] = 9;
        let original = JobProgress::datagen_batch(DatagenBatch {
            game: 4,
            games: 16,
            positions: 80,
            nps: 12_000,
            throughput: 1.25,
            white: 2,
            draw: 1,
            black: 1,
            pass: 70,
            drop: 10,
            bytes: 4096,
            hist,
        });
        let parsed = parse_progress_line(&format_progress(&original)).expect("parse");
        assert_eq!(parsed.kind, JobKind::Datagen);
        assert_eq!(parsed.nps, Some(12_000));
        assert!((parsed.throughput.unwrap() - 1.25).abs() < 0.001);
        assert_eq!(parsed.white, Some(2));
        assert_eq!(parsed.draw, Some(1));
        assert_eq!(parsed.black, Some(1));
        assert_eq!(parsed.pass, Some(70));
        assert_eq!(parsed.drop, Some(10));
        assert_eq!(parsed.bytes, Some(4096));
        assert_eq!(parsed.hist, Some(hist));
    }

    #[test]
    fn train_batch_roundtrips_val_loss_lr_and_mpos() {
        let original = JobProgress::train_batch(TrainerBatch {
            epoch: 3,
            epochs: 8,
            loss: 0.4,
            val_loss: Some(0.55),
            expert: 1,
            lr: 0.01,
            mpos: 2.5,
        });
        let parsed = parse_progress_line(&format_progress(&original)).expect("parse");
        assert_eq!(parsed.kind, JobKind::Train);
        assert!((parsed.loss.unwrap() - 0.4).abs() < 0.001);
        assert!((parsed.val_loss.unwrap() - 0.55).abs() < 0.001);
        assert!((parsed.lr.unwrap() - 0.01).abs() < 0.0001);
        assert!((parsed.mpos.unwrap() - 2.5).abs() < 0.001);
        assert_eq!(parsed.expert, Some(1));
    }

    #[test]
    fn hist_pads_short_and_truncates_long() {
        assert_eq!(parse_hist("1,2,3")[0], 1);
        assert_eq!(parse_hist("1,2,3")[3], 0);
        let long = (0..32).map(|n| n.to_string()).collect::<Vec<_>>().join(",");
        let hist = parse_hist(&long);
        assert_eq!(hist[0], 0);
        assert_eq!(hist[15], 15);
    }

    #[test]
    fn unknown_progress_keys_are_ignored() {
        let parsed = parse_progress_line(
            "progress kind=train pct=50.0 epoch=1 epochs=2 loss=0.1 extra=nope",
        )
        .expect("parse");
        assert_eq!(parsed.epoch, Some(1));
        assert!((parsed.loss.unwrap() - 0.1).abs() < 0.001);
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

    #[test]
    fn time_throttle_caps_mid_run_emits() {
        let start = Instant::now();
        assert!(should_report_now(0, 200, start, start));
        assert!(should_report_now(200, 200, start, start));
        assert!(!should_report_now(
            2,
            200,
            start,
            start + Duration::from_millis(10)
        ));
        assert!(should_report_now(
            2,
            200,
            start,
            start + Duration::from_millis(250)
        ));
        assert!(!should_report_now(
            1,
            200,
            start,
            start + Duration::from_millis(250)
        ));
    }
}
