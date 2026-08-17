//! Ateed studio domain: gate, CLI discovery, job plans, and runtime probes.
//!
//! Fetch / train / datagen require a trainer-capable CLI from
//! `engines/mujrim/` (preferred: `mujrim-train`), then a sibling `mujrim`,
//! then `target/{debug,release}`. The GUI never links an engine.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use crate::app_core::uci_process::{self, ExternalEngineProtocol, ExternalSearchConfig};

/// Base64 of the Ateed studio unlock secret. Never store the plaintext here.
pub const ATEED_GATE_B64: &str = "SkFIQU5BTQ==";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AteedSourceKind {
    LocalFile,
    Http,
    Datagen,
}

impl AteedSourceKind {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "local" | "file" => Ok(Self::LocalFile),
            "http" | "https" | "url" => Ok(Self::Http),
            "datagen" => Ok(Self::Datagen),
            other => Err(format!("unknown Ateed source `{other}`")),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::LocalFile => "Local file",
            Self::Http => "HTTP Range",
            Self::Datagen => "Self-play",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AteedDataSource {
    pub kind: AteedSourceKind,
    pub value: String,
    pub weight: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AteedJobKind {
    Fetch,
    Train,
    Datagen,
    Decode,
    Merge,
    Evaluate,
    Bench,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AteedCliCommand {
    Fetch {
        id: Option<String>,
        url: String,
        output: String,
    },
    Train {
        data: String,
        mix: String,
        output: String,
        epochs: u32,
        lr: String,
        wdl_weight: String,
        scope: String,
        base: Option<String>,
    },
    Datagen {
        games: u64,
        #[serde(default)]
        positions: Option<u64>,
        depth: u32,
        output: String,
        format: String,
    },
    Decode {
        input: String,
        output: String,
        format: String,
    },
    Merge {
        data: String,
        mix: String,
        output: String,
        format: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AteedJobPlan {
    pub kind: AteedJobKind,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AteedIndexPrompt {
    pub new_games: usize,
    pub summary: String,
}

pub const ATEED_NETWORK_FILENAME: &str = "ateed_default.bin";

pub fn writable_nnue_directory() -> PathBuf {
    if let Ok(executable) = std::env::current_exe() {
        for ancestor in executable
            .parent()
            .into_iter()
            .flat_map(|path| path.ancestors())
            .take(5)
        {
            let dir = ancestor.join("nnue");
            if dir.is_dir() {
                return dir;
            }
        }
    }
    let dir = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("nnue");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn ateed_artifact_path() -> PathBuf {
    let dir = writable_nnue_directory();
    let candidate = dir.join(ATEED_NETWORK_FILENAME);
    if candidate.is_file() {
        candidate
    } else {
        dir.join(ATEED_NETWORK_FILENAME)
    }
}

fn ateed_file_size() -> u64 {
    std::fs::metadata(ateed_artifact_path())
        .map(|meta| meta.len())
        .unwrap_or(0)
}

pub fn scan_tournament_index(
    db: &mujrim_study::database::StudyDatabase,
) -> Option<AteedIndexPrompt> {
    scan_tournament_index_in(db, &writable_nnue_directory())
}

pub fn scan_tournament_index_in(
    db: &mujrim_study::database::StudyDatabase,
    root: &Path,
) -> Option<AteedIndexPrompt> {
    let tournaments = db.list_tournaments().ok()?;
    let index = mujrim_study::ateed_index::PositionIndex::load(
        &mujrim_study::ateed_index::index_path(root),
    );
    let pairs: Vec<_> = tournaments
        .into_iter()
        .map(|tournament| (tournament.id, tournament.games))
        .collect();
    let scan = mujrim_study::ateed_index::scan_unindexed(&pairs, &index);
    if scan.new_games == 0 {
        return None;
    }
    Some(AteedIndexPrompt {
        new_games: scan.new_games,
        summary: format!(
            "{} new tournament game(s) can be indexed for Ateed training",
            scan.new_games
        ),
    })
}

pub fn index_tournament_positions(
    db: &mujrim_study::database::StudyDatabase,
) -> Result<(String, String), String> {
    index_tournament_positions_in(db, &writable_nnue_directory())
}

pub fn index_tournament_positions_in(
    db: &mujrim_study::database::StudyDatabase,
    root: &Path,
) -> Result<(String, String), String> {
    let tournaments = db.list_tournaments()?;
    let index_path = mujrim_study::ateed_index::index_path(root);
    let dataset = mujrim_study::ateed_index::tournament_dataset_path(root);
    let mut index = mujrim_study::ateed_index::PositionIndex::load(&index_path);
    let pairs: Vec<_> = tournaments
        .into_iter()
        .map(|tournament| (tournament.id, tournament.games))
        .collect();
    let report =
        mujrim_study::ateed_index::index_games_scored(&pairs, &mut index, &dataset, |_board| 0)?;
    index.save(&index_path)?;
    Ok((
        dataset.to_string_lossy().into_owned(),
        format!(
            "indexed {} game(s), {} new positions, {} duplicates skipped",
            report.games_indexed, report.positions_added, report.positions_skipped
        ),
    ))
}

pub fn ensure_local_source(sources: &mut Vec<AteedDataSource>, path: &str) {
    if sources.iter().any(|source| source.value == path) {
        return;
    }
    sources.push(AteedDataSource {
        kind: AteedSourceKind::LocalFile,
        value: path.to_owned(),
        weight: 1,
    });
}

pub const METRIC_RING_CAP: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetricRing {
    buf: [f32; METRIC_RING_CAP],
    next: usize,
    filled: usize,
}

impl Default for MetricRing {
    fn default() -> Self {
        Self {
            buf: [0.0; METRIC_RING_CAP],
            next: 0,
            filled: 0,
        }
    }
}

impl MetricRing {
    pub fn push(&mut self, value: f32) {
        self.buf[self.next] = value;
        self.next = (self.next + 1) % METRIC_RING_CAP;
        if self.filled < METRIC_RING_CAP {
            self.filled += 1;
        }
    }

    pub fn len(&self) -> usize {
        self.filled
    }

    pub fn copy_oldest_first(&self, out: &mut [f32]) -> usize {
        let n = self.filled.min(out.len());
        if n == 0 {
            return 0;
        }
        let start = if self.filled == METRIC_RING_CAP {
            self.next
        } else {
            0
        };
        for (index, slot) in out.iter_mut().enumerate().take(n) {
            *slot = self.buf[(start + index) % METRIC_RING_CAP];
        }
        n
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LossRing {
    pub train: MetricRing,
    pub val: MetricRing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AteedMonitorTick {
    pub kind: updater::progress::JobKind,
    pub epoch: u32,
    pub progress: f32,
    pub loss: f32,
    pub val_loss: f32,
    pub expert: usize,
    pub nps: u64,
    pub games: u64,
    pub positions: u64,
    pub mbps: f32,
    pub mpos: f32,
    pub lr: f32,
    pub wdl: (u64, u64, u64),
    pub pass: u64,
    pub drop: u64,
    pub hist: [u32; updater::progress::HIST_BUCKETS],
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AteedStrengthReport {
    pub score: i32,
    pub variance: i32,
    pub expert: usize,
    pub file_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AteedPerfReport {
    pub matvec_ns: u128,
    pub eval_ns: u128,
}

pub fn decode_base64(input: &str) -> Result<Vec<u8>, &'static str> {
    let input = input.trim().as_bytes();
    if !input.len().is_multiple_of(4) {
        return Err("invalid base64 length");
    }
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    for chunk in input.chunks_exact(4) {
        let a = b64_val(chunk[0])?;
        let b = b64_val(chunk[1])?;
        let pad_c = chunk[2] == b'=';
        let pad_d = chunk[3] == b'=';
        let c = if pad_c { 0 } else { b64_val(chunk[2])? };
        let d = if pad_d { 0 } else { b64_val(chunk[3])? };
        out.push((a << 2) | (b >> 4));
        if !pad_c {
            out.push(((b & 0x0f) << 4) | (c >> 2));
        }
        if !pad_d {
            out.push(((c & 0x03) << 6) | d);
        }
    }
    Ok(out)
}

fn b64_val(byte: u8) -> Result<u8, &'static str> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err("invalid base64 symbol"),
    }
}

pub fn ateed_gate_secret() -> Result<String, &'static str> {
    String::from_utf8(decode_base64(ATEED_GATE_B64)?).map_err(|_| "gate is not utf-8")
}

pub fn unlock_ateed(password: &str) -> bool {
    ateed_gate_secret().is_ok_and(|secret| secret == password)
}

pub fn parse_train_scope(name: &str) -> Result<&'static str, String> {
    match name {
        "heads" | "output-biases" => Ok("heads"),
        "expert0" => Ok("expert0"),
        "moe" => Ok("moe"),
        other => Err(format!("unknown Ateed train scope `{other}`")),
    }
}

pub fn validate_source(kind: AteedSourceKind, value: &str) -> Result<AteedDataSource, String> {
    validate_weighted_source(kind, value, 1)
}

pub fn validate_weighted_source(
    kind: AteedSourceKind,
    value: &str,
    weight: u32,
) -> Result<AteedDataSource, String> {
    if weight == 0 {
        return Err("mix weight must be at least 1".to_string());
    }
    let value = value.trim();
    if value.is_empty() {
        return Err("source value is empty".to_string());
    }
    match kind {
        AteedSourceKind::Http => {
            if !(value.starts_with("http://") || value.starts_with("https://")) {
                return Err("HTTP source must be an http(s) URL".to_string());
            }
        }
        AteedSourceKind::LocalFile => {
            if value.starts_with("http://") || value.starts_with("https://") {
                return Err("local source must be a filesystem path".to_string());
            }
        }
        AteedSourceKind::Datagen => {
            if value.parse::<u64>().unwrap_or(0) == 0 {
                return Err("datagen source must be a positive position count".to_string());
            }
        }
    }
    Ok(AteedDataSource {
        kind,
        value: value.to_string(),
        weight,
    })
}

pub fn catalog_draft(id: &str) -> Result<(AteedSourceKind, &'static str), String> {
    let offer =
        updater::datasets::find_dataset(id).ok_or_else(|| format!("unknown catalog id `{id}`"))?;
    if offer.url.is_empty() {
        Ok((AteedSourceKind::LocalFile, offer.filename))
    } else {
        Ok((AteedSourceKind::Http, offer.url))
    }
}

pub fn dataset_format_for_path(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.ends_with(".plain") {
        "plain"
    } else if name.contains(".binpack") || name.ends_with(".mjbp") {
        "binpack"
    } else {
        "text"
    }
}

pub fn datagen_batch_size(sources: &[AteedDataSource]) -> u64 {
    sources
        .iter()
        .find(|source| source.kind == AteedSourceKind::Datagen)
        .and_then(|source| source.value.parse::<u64>().ok())
        .unwrap_or(0)
}

/// When the output net already exists, the next train session fine-tunes it.
pub fn continuing_train_base(output: &str) -> Option<String> {
    let path = Path::new(output);
    path.is_file().then(|| output.to_owned())
}

pub const FIELD_HELP: &[(&str, &str)] = &[
    (
        "source",
        "Queue downloads and local files here. Stockfish / Lc0 / Self-play chips fill a catalog URL. Datagen is a separate Play control — it is not a source chip.",
    ),
    (
        "source_url",
        "Paste an HTTP(S) file link, then Add source. Catalog chips fill this for you.",
    ),
    (
        "source_path",
        "Path to a dataset already on this computer, then Add source.",
    ),
    (
        "strategy",
        "Best path for this net: fetch Stockfish/Lc0 dumps for volume, mix in your own Datagen labels, train heads first on the growing file, then expert0 or moe. Keep the same dataset path and output net every day so sessions continue instead of starting over.",
    ),
    (
        "selfplay",
        "Self-play catalog = download existing dumped games. Datagen = this engine plays itself and appends new positions. Use the catalog for fast volume; use Datagen when you want labels from this net.",
    ),
    (
        "datagen_when",
        "Use Datagen to grow the dataset with this engine’s self-play. It never starts on its own. Play begins a batch, Pause freezes the process, Stop ends it and keeps the sidecar so Resume job can continue.",
    ),
    (
        "mix",
        "How often this source is picked when several files are mixed. A higher number means more of this data in each training pass.",
    ),
    (
        "scope",
        "What part of the net to teach. heads = only the final “who is winning” numbers (fastest). expert0 = the first specialist plus how it sees the board. moe = whichever specialist the net actually uses for each position (slowest, usually strongest).",
    ),
    (
        "epochs",
        "How many times to walk through today’s dataset. More passes learn more this session, but too many can memorize the file instead of getting generally stronger.",
    ),
    (
        "lr",
        "How big each correction is. Higher learns faster but can jump around; lower is slower and steadier. Keep the same value when you continue tomorrow.",
    ),
    (
        "wdl",
        "How much to care about the real game result (win / draw / loss) versus the numeric score. Raise it if you want the net to think more about “did White win?”.",
    ),
    (
        "dataset",
        "The growing file of positions. Each Datagen Play batch appends here. Training reads this whole file, so yesterday’s positions stay in the mix.",
    ),
    (
        "output",
        "The net file (the “brain”). If this file already exists, training starts from it instead of from scratch — so daily sessions continue where you left off.",
    ),
    (
        "positions",
        "How many new positions to add this Play run (default 1000000000, one billion). Tomorrow, use the same dataset path and this appends another chunk instead of replacing the old ones.",
    ),
    (
        "depth",
        "How hard the engine thinks while playing itself. Deeper labels are smarter but much slower. 6 is a practical daily default.",
    ),
];

pub fn field_help(id: &str) -> &'static str {
    FIELD_HELP
        .iter()
        .find(|(key, _)| *key == id)
        .map(|(_, text)| *text)
        .unwrap_or("")
}

pub fn local_mix(sources: &[AteedDataSource]) -> (String, String) {
    let locals: Vec<&AteedDataSource> = sources
        .iter()
        .filter(|source| source.kind == AteedSourceKind::LocalFile)
        .collect();
    (
        locals
            .iter()
            .map(|source| source.value.as_str())
            .collect::<Vec<_>>()
            .join(","),
        locals
            .iter()
            .map(|source| source.weight.to_string())
            .collect::<Vec<_>>()
            .join(","),
    )
}

/// Trainer CLIs published under `engines/mujrim/`, preferred first.
pub const TRAINER_CLI_STEMS: &[&str] =
    &["mujrim-train", "mujrim", "mujrim-ateed", "mujrim-external"];

pub fn mujrim_cli_name() -> &'static str {
    if cfg!(windows) {
        "mujrim.exe"
    } else {
        "mujrim"
    }
}

pub fn cli_filename(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

pub fn is_runnable_cli(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub fn mujrim_cli_candidates(executable: &Path, current_dir: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut push = |path: PathBuf| {
        if !candidates.iter().any(|existing| existing == &path) {
            candidates.push(path);
        }
    };
    for root in mujrim_protocols::catalog::engine_search_roots(executable, current_dir) {
        let mujrim_dir = root.join(mujrim_protocols::catalog::mujrim_engines_directory());
        let arch = mujrim_protocols::catalog::RuntimePlatform::current().directory_name();
        for stem in TRAINER_CLI_STEMS {
            let name = cli_filename(stem);
            push(mujrim_dir.join(&name));
            push(mujrim_dir.join("bin").join(&arch).join(&name));
        }
    }
    let name = mujrim_cli_name();
    if let Some(dir) = executable.parent() {
        push(dir.join(name));
        if let Some(parent) = dir.parent() {
            push(parent.join(name));
        }
    }
    push(current_dir.join(name));
    push(current_dir.join("target").join("release").join(name));
    push(current_dir.join("target").join("debug").join(name));
    candidates
}

pub fn discover_mujrim_cli(executable: &Path, current_dir: &Path) -> Option<PathBuf> {
    mujrim_cli_candidates(executable, current_dir)
        .into_iter()
        .find(|path| is_runnable_cli(path))
}

pub fn cli_supports_train(path: &Path) -> bool {
    Command::new(path)
        .args(["train", "catalog"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn discover_mujrim_cli_from_environment() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let cwd = std::env::current_dir().ok()?;
    mujrim_cli_candidates(&exe, &cwd)
        .into_iter()
        .find(|path| is_runnable_cli(path) && cli_supports_train(path))
}

pub fn cli_args(command: &AteedCliCommand) -> Vec<String> {
    match command {
        AteedCliCommand::Fetch { id, url, output } => {
            let mut args = vec!["train".into(), "fetch".into(), "-o".into(), output.clone()];
            if let Some(id) = id {
                args.push("--id".into());
                args.push(id.clone());
            }
            if !url.is_empty() {
                args.push("--url".into());
                args.push(url.clone());
            }
            args
        }
        AteedCliCommand::Train {
            data,
            mix,
            output,
            epochs,
            lr,
            wdl_weight,
            scope,
            base,
        } => {
            let mut args = vec![
                "train".into(),
                "ateed".into(),
                "--data".into(),
                data.clone(),
                "-o".into(),
                output.clone(),
                "-e".into(),
                epochs.to_string(),
                "--lr".into(),
                lr.clone(),
                "--wdl-weight".into(),
                wdl_weight.clone(),
                "--scope".into(),
                scope.clone(),
            ];
            if !mix.is_empty() {
                args.push("--mix".into());
                args.push(mix.clone());
            }
            if let Some(base) = base {
                args.push("--base".into());
                args.push(base.clone());
            }
            args
        }
        AteedCliCommand::Datagen {
            games,
            positions,
            depth,
            output,
            format,
        } => {
            let mut args = vec![
                "train".into(),
                "datagen".into(),
                "-g".into(),
                games.to_string(),
                "-d".into(),
                depth.to_string(),
                "-o".into(),
                output.clone(),
                "--format".into(),
                format.clone(),
            ];
            if let Some(positions) = positions {
                args.push("--positions".into());
                args.push(positions.to_string());
            }
            args
        }
        AteedCliCommand::Decode {
            input,
            output,
            format,
        } => vec![
            "train".into(),
            "decode".into(),
            "-i".into(),
            input.clone(),
            "-o".into(),
            output.clone(),
            "--format".into(),
            format.clone(),
        ],
        AteedCliCommand::Merge {
            data,
            mix,
            output,
            format,
        } => {
            let mut args = vec![
                "train".into(),
                "merge".into(),
                "--data".into(),
                data.clone(),
                "-o".into(),
                output.clone(),
                "--format".into(),
                format.clone(),
            ];
            if !mix.is_empty() {
                args.push("--mix".into());
                args.push(mix.clone());
            }
            args
        }
    }
}

pub fn require_cli(cli: Option<&Path>, kind: AteedJobKind) -> Result<(), String> {
    match kind {
        AteedJobKind::Fetch
        | AteedJobKind::Train
        | AteedJobKind::Datagen
        | AteedJobKind::Decode
        | AteedJobKind::Merge => {
            if cli.is_some() {
                Ok(())
            } else {
                Err("Mujrim CLI not found — this action is disabled".to_string())
            }
        }
        AteedJobKind::Evaluate | AteedJobKind::Bench => Ok(()),
    }
}

static LIVE_CLI_PID: AtomicU32 = AtomicU32::new(0);
static CLI_STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliProcessSignal {
    Pause,
    Resume,
    Stop,
}

pub fn live_cli_pid() -> Option<u32> {
    match LIVE_CLI_PID.load(Ordering::SeqCst) {
        0 => None,
        pid => Some(pid),
    }
}

pub fn set_live_cli_pid(pid: Option<u32>) {
    LIVE_CLI_PID.store(pid.unwrap_or(0), Ordering::SeqCst);
}

pub fn cli_signal_command(pid: u32, signal: CliProcessSignal) -> (&'static str, Vec<String>) {
    let flag = match signal {
        CliProcessSignal::Pause => "-STOP",
        CliProcessSignal::Resume => "-CONT",
        CliProcessSignal::Stop => "-TERM",
    };
    ("kill", vec![flag.to_owned(), pid.to_string()])
}

pub fn signal_live_cli(signal: CliProcessSignal) -> Result<(), String> {
    let pid = live_cli_pid().ok_or_else(|| "no live CLI process".to_string())?;
    if matches!(signal, CliProcessSignal::Stop) {
        CLI_STOP_REQUESTED.store(true, Ordering::SeqCst);
    }
    #[cfg(unix)]
    {
        let (prog, args) = cli_signal_command(pid, signal);
        let status = Command::new(prog)
            .args(&args)
            .status()
            .map_err(|error| format!("failed to signal CLI: {error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("kill {pid} failed"))
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Err("CLI process signals are only available on Unix".to_string())
    }
}

pub fn run_mujrim_cli(
    cli: &Path,
    args: &[String],
    mut on_line: impl FnMut(&str),
) -> Result<i32, String> {
    CLI_STOP_REQUESTED.store(false, Ordering::SeqCst);
    let mut child = Command::new(cli)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to start {}: {error}", cli.display()))?;
    set_live_cli_pid(Some(child.id()));
    let result = (|| {
        if let Some(stdout) = child.stdout.take() {
            for line in BufReader::new(stdout).lines() {
                let line = line.map_err(|error| error.to_string())?;
                on_line(&line);
            }
        }
        if let Some(stderr) = child.stderr.take() {
            for line in BufReader::new(stderr).lines() {
                let line = line.map_err(|error| error.to_string())?;
                if !line.is_empty() {
                    on_line(&line);
                }
            }
        }
        let status = child
            .wait()
            .map_err(|error| format!("CLI wait failed: {error}"))?;
        if CLI_STOP_REQUESTED.swap(false, Ordering::SeqCst) {
            return Ok(0);
        }
        Ok(status.code().unwrap_or(1))
    })();
    set_live_cli_pid(None);
    result
}

pub fn plan_job(
    kind: AteedJobKind,
    sources: &[AteedDataSource],
    scope: &str,
    epochs: u32,
    cli_available: bool,
) -> Result<AteedJobPlan, String> {
    require_cli(cli_available.then_some(Path::new("mujrim")), kind)?;
    match kind {
        AteedJobKind::Fetch => {
            let http = sources
                .iter()
                .filter(|source| source.kind == AteedSourceKind::Http)
                .count();
            if http == 0 {
                return Err("add at least one HTTP data source".to_string());
            }
            Ok(AteedJobPlan {
                kind,
                summary: format!("Fetch and convert {http} HTTP source(s) via mujrim train fetch"),
            })
        }
        AteedJobKind::Train => {
            parse_train_scope(scope)?;
            if epochs == 0 {
                return Err("epochs must be at least 1".to_string());
            }
            if sources
                .iter()
                .all(|source| source.kind != AteedSourceKind::LocalFile)
            {
                return Err("training needs a local dataset path".to_string());
            }
            Ok(AteedJobPlan {
                kind,
                summary: format!("Train {scope} for {epochs} epoch(s) via mujrim train ateed"),
            })
        }
        AteedJobKind::Datagen => {
            let positions = datagen_batch_size(sources);
            if positions == 0 {
                return Err("set a positive batch size (positions to add this run)".to_string());
            }
            Ok(AteedJobPlan {
                kind,
                summary: format!(
                    "Append about {positions} new self-play position(s) via mujrim train datagen"
                ),
            })
        }
        AteedJobKind::Decode => {
            if sources
                .iter()
                .all(|source| source.kind != AteedSourceKind::LocalFile)
            {
                return Err("decode needs a local dump path".to_string());
            }
            Ok(AteedJobPlan {
                kind,
                summary: "Decode a local dump via mujrim train decode".to_string(),
            })
        }
        AteedJobKind::Merge => {
            let locals = sources
                .iter()
                .filter(|source| source.kind == AteedSourceKind::LocalFile)
                .count();
            if locals < 2 {
                return Err("merge needs at least two local sources".to_string());
            }
            Ok(AteedJobPlan {
                kind,
                summary: format!("Weighted-merge {locals} sources via mujrim train merge"),
            })
        }
        AteedJobKind::Evaluate | AteedJobKind::Bench => Ok(AteedJobPlan {
            kind,
            summary: match kind {
                AteedJobKind::Evaluate => "Evaluate the in-memory zero Ateed net".to_string(),
                AteedJobKind::Bench => "Probe CPU matvec and Ateed eval latency".to_string(),
                _ => unreachable!(),
            },
        }),
    }
}

pub fn dry_run_train(epochs: u32, expert: usize) -> Vec<AteedMonitorTick> {
    let epochs = epochs.max(1);
    (1..=epochs)
        .map(|epoch| {
            let progress = epoch as f32 / epochs as f32;
            AteedMonitorTick {
                kind: updater::progress::JobKind::Train,
                epoch,
                progress,
                loss: (1.0 - progress) * 0.85 + 0.05,
                val_loss: (1.0 - progress) * 0.9 + 0.08,
                expert,
                nps: 0,
                games: 0,
                positions: 0,
                mbps: 0.0,
                mpos: 1.5,
                lr: 1.0,
                wdl: (0, 0, 0),
                pass: 0,
                drop: 0,
                hist: [0; updater::progress::HIST_BUCKETS],
                message: format!("epoch {epoch}/{epochs} routed expert {expert}"),
            }
        })
        .collect()
}

fn runtime_engine_probe(
    depth: i32,
    movetime: Duration,
) -> Result<uci_process::ExternalMoveResult, String> {
    let path = discover_mujrim_cli_from_environment().ok_or_else(|| {
        "Mujrim engine binary not found. Place mujrim beside the UI or in target/release."
            .to_owned()
    })?;
    let search = ExternalSearchConfig {
        ponder: false,
        use_nnue: true,
        own_book: false,
        eval_file: None,
    };
    uci_process::query_best_move(
        path.to_str()
            .ok_or_else(|| "engine path is not UTF-8".to_owned())?,
        ExternalEngineProtocol::Uci,
        mujrim_study::opening::START_FEN,
        depth,
        movetime,
        16,
        1,
        &search,
    )
}

pub fn evaluate_zero_net() -> AteedStrengthReport {
    let file_size = ateed_file_size();
    match runtime_engine_probe(1, Duration::from_millis(80)) {
        Ok(info) => AteedStrengthReport {
            score: info.score,
            variance: 0,
            expert: 0,
            file_size,
        },
        Err(_) => AteedStrengthReport {
            score: 0,
            variance: 0,
            expert: 0,
            file_size,
        },
    }
}

pub fn probe_compute() -> AteedPerfReport {
    let start = Instant::now();
    let ok = runtime_engine_probe(1, Duration::from_millis(80)).is_ok();
    let eval_ns = start.elapsed().as_nanos();
    AteedPerfReport {
        matvec_ns: if ok { eval_ns } else { 0 },
        eval_ns: if ok { eval_ns } else { 0 },
    }
}

pub fn format_strength(report: &AteedStrengthReport) -> String {
    format!(
        "score {:+} · WDL σ² {} · expert {} · {} bytes",
        report.score, report.variance, report.expert, report.file_size
    )
}

pub fn monitor_from_progress(progress: &updater::progress::JobProgress) -> AteedMonitorTick {
    AteedMonitorTick {
        kind: progress.kind,
        epoch: progress.epoch.unwrap_or(0),
        progress: (progress.pct / 100.0).clamp(0.0, 1.0),
        loss: progress.loss.unwrap_or(0.0),
        val_loss: progress.val_loss.unwrap_or(0.0),
        expert: progress.expert.unwrap_or(0),
        nps: progress.nps.unwrap_or(0),
        games: progress.game.unwrap_or(progress.games.unwrap_or(0)),
        positions: progress.positions.unwrap_or(0),
        mbps: if progress.kind == updater::progress::JobKind::Datagen {
            progress.throughput.unwrap_or(0.0)
        } else {
            0.0
        },
        mpos: progress.mpos.unwrap_or(progress.throughput.unwrap_or(0.0)),
        lr: progress.lr.unwrap_or(0.0),
        wdl: (
            progress.white.unwrap_or(0),
            progress.draw.unwrap_or(0),
            progress.black.unwrap_or(0),
        ),
        pass: progress.pass.unwrap_or(0),
        drop: progress.drop.unwrap_or(0),
        hist: progress
            .hist
            .unwrap_or([0; updater::progress::HIST_BUCKETS]),
        message: updater::progress::format_progress(progress),
    }
}

pub fn format_perf(report: &AteedPerfReport) -> String {
    format!(
        "matvec {} ns · eval {} ns",
        report.matvec_ns, report.eval_ns
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn gate_payload_is_base64_and_not_plaintext_in_source() {
        let source = include_str!("ateed_studio.rs");
        let production = source.split("#[cfg(test)]").next().expect("source");
        assert!(production.contains(ATEED_GATE_B64));
        assert!(!production.contains("JAHANAM"));
        let secret = ateed_gate_secret().expect("decode gate");
        assert_eq!(secret.len(), 7);
        assert!(unlock_ateed(&secret));
        assert!(!unlock_ateed(""));
        assert!(!unlock_ateed("wrong"));
        assert!(!unlock_ateed(ATEED_GATE_B64));
    }

    #[test]
    fn decode_base64_roundtrips_the_gate() {
        assert_eq!(decode_base64("QQ==").unwrap(), b"A");
        assert_eq!(decode_base64(ATEED_GATE_B64).unwrap().len(), 7);
        assert!(decode_base64("***").is_err());
    }

    #[test]
    fn sources_and_jobs_validate_without_touching_the_network() {
        assert!(validate_source(AteedSourceKind::Http, "https://example.test/data.txt").is_ok());
        assert!(validate_source(AteedSourceKind::Http, "ftp://x").is_err());
        assert!(validate_source(AteedSourceKind::LocalFile, "data.txt").is_ok());
        assert!(validate_source(AteedSourceKind::LocalFile, "https://x").is_err());
        assert!(validate_source(AteedSourceKind::Datagen, "16").is_ok());
        assert!(validate_source(AteedSourceKind::Datagen, "0").is_err());
        let http = [validate_source(AteedSourceKind::Http, "https://example.test/a").unwrap()];
        let local = [validate_source(AteedSourceKind::LocalFile, "data.txt").unwrap()];
        let fetch = plan_job(AteedJobKind::Fetch, &http, "heads", 8, true).unwrap();
        assert!(fetch.summary.contains("mujrim train fetch"));
        assert!(fetch.summary.contains("convert"));
        assert!(
            plan_job(AteedJobKind::Fetch, &http, "heads", 8, false)
                .unwrap_err()
                .contains("disabled")
        );
        assert_eq!(parse_train_scope("heads").unwrap(), "heads");
        assert_eq!(parse_train_scope("moe").unwrap(), "moe");
        assert!(parse_train_scope("bullet").is_err());
        assert!(plan_job(AteedJobKind::Train, &local, "moe", 4, true).is_ok());
        assert!(plan_job(AteedJobKind::Train, &local, "nope", 4, true).is_err());
        assert!(plan_job(AteedJobKind::Train, &http, "heads", 4, true).is_err());
        assert!(plan_job(AteedJobKind::Evaluate, &[], "heads", 0, false).is_ok());
        let datagen = [validate_source(AteedSourceKind::Datagen, "1000000").unwrap()];
        let plan = plan_job(AteedJobKind::Datagen, &datagen, "heads", 1, true).unwrap();
        assert!(plan.summary.contains("1000000"));
        assert!(plan.summary.contains("Append"));
        assert!(plan_job(AteedJobKind::Datagen, &[], "heads", 1, true).is_err());
        assert_eq!(datagen_batch_size(&datagen), 1_000_000);
        assert!(field_help("positions").contains("1000000000"));
        assert!(field_help("strategy").contains("heads first"));
        assert!(field_help("selfplay").contains("catalog"));
        assert!(field_help("datagen_when").contains("never starts on its own"));
        assert!(field_help("source_url").contains("HTTP"));
        assert!(field_help("output").contains("already exists"));
        assert_eq!(
            cli_signal_command(4242, CliProcessSignal::Pause),
            ("kill", vec!["-STOP".into(), "4242".into()])
        );
        assert_eq!(
            cli_signal_command(4242, CliProcessSignal::Resume),
            ("kill", vec!["-CONT".into(), "4242".into()])
        );
        assert_eq!(
            cli_signal_command(4242, CliProcessSignal::Stop),
            ("kill", vec!["-TERM".into(), "4242".into()])
        );
        assert_eq!(TRAINER_CLI_STEMS[0], "mujrim-train");
        let missing = std::env::temp_dir().join("mujrim-missing-ateed-net.bin");
        let _ = std::fs::remove_file(&missing);
        assert!(continuing_train_base(&missing.display().to_string()).is_none());
        assert!(plan_job(AteedJobKind::Decode, &local, "heads", 1, true).is_ok());
        assert!(plan_job(AteedJobKind::Merge, &local, "heads", 1, true).is_err());
        let two = [
            validate_weighted_source(AteedSourceKind::LocalFile, "a.txt", 2).unwrap(),
            validate_weighted_source(AteedSourceKind::LocalFile, "b.plain", 1).unwrap(),
        ];
        assert!(plan_job(AteedJobKind::Merge, &two, "heads", 1, true).is_ok());
        assert_eq!(local_mix(&two), ("a.txt,b.plain".into(), "2,1".into()));
        assert!(validate_weighted_source(AteedSourceKind::LocalFile, "a.txt", 0).is_err());
        let (kind, value) = catalog_draft("lc0-training").unwrap();
        assert_eq!(kind, AteedSourceKind::Http);
        assert!(value.contains("lczero.org"));
        assert_eq!(dataset_format_for_path("games.binpack.gz"), "binpack");
    }

    #[test]
    fn cli_discovery_and_argv_stay_local() {
        let root = std::env::temp_dir().join(format!(
            "mujrim-cli-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let write_cli = |path: &Path| {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, []).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(path).unwrap().permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(path, perms).unwrap();
            }
        };
        let sibling = root.join(mujrim_cli_name());
        write_cli(&sibling);
        let engines = root
            .join("engines")
            .join(mujrim_protocols::catalog::mujrim_engines_directory())
            .join(cli_filename("mujrim-train"));
        write_cli(&engines);
        let ui = root.join("mujrim-ui");
        std::fs::write(&ui, []).unwrap();
        let found = discover_mujrim_cli(&ui, &root).expect("discover engines CLI");
        assert_eq!(found, engines);
        assert!(
            mujrim_cli_candidates(&ui, &root)
                .iter()
                .any(|path| path.ends_with(cli_filename("mujrim-ateed")))
        );
        let isolated = std::env::temp_dir().join(format!(
            "mujrim-cli-empty-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&isolated).unwrap();
        let lonely_ui = isolated.join("bin").join("mujrim-ui");
        std::fs::create_dir_all(lonely_ui.parent().unwrap()).unwrap();
        std::fs::write(&lonely_ui, []).unwrap();
        assert!(discover_mujrim_cli(&lonely_ui, &isolated).is_none());
        let _ = std::fs::remove_dir_all(&isolated);
        let args = cli_args(&AteedCliCommand::Fetch {
            id: Some("lc0-training".into()),
            url: "https://example.test/a".into(),
            output: "data.txt".into(),
        });
        assert_eq!(
            args,
            [
                "train",
                "fetch",
                "-o",
                "data.txt",
                "--id",
                "lc0-training",
                "--url",
                "https://example.test/a"
            ]
        );
        let merge = cli_args(&AteedCliCommand::Merge {
            data: "a.txt,b.plain".into(),
            mix: "2,1".into(),
            output: "mix.txt".into(),
            format: "text".into(),
        });
        assert!(merge.contains(&"--mix".to_string()));
        assert!(merge.contains(&"2,1".to_string()));
        let datagen = cli_args(&AteedCliCommand::Datagen {
            games: 1_000_000,
            positions: Some(1_000_000),
            depth: 6,
            output: "data.txt".into(),
            format: "text".into(),
        });
        assert!(datagen.contains(&"--positions".to_string()));
        assert!(datagen.contains(&"1000000".to_string()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn run_mujrim_cli_streams_stub_progress() {
        let path = std::env::temp_dir().join(format!(
            "mujrim-stub-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::write(
            &path,
            "#!/bin/sh\necho 'progress kind=train pct=50.0 epoch=1 epochs=2 loss=0.4'\nexit 0\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        let mut lines = Vec::new();
        let code =
            run_mujrim_cli(&path, &[], |line| lines.push(line.to_owned())).expect("run stub");
        let _ = std::fs::remove_file(&path);
        assert_eq!(code, 0);
        assert!(
            lines
                .iter()
                .any(|line| updater::progress::parse_progress_line(line).is_some())
        );
    }

    #[test]
    fn dry_run_train_emits_a_monotonic_progress_curve() {
        let ticks = dry_run_train(4, 1);
        assert_eq!(ticks.len(), 4);
        assert_eq!(ticks[0].epoch, 1);
        assert_eq!(ticks[3].progress, 1.0);
        assert!(ticks[0].loss > ticks[3].loss);
        assert_eq!(ticks[2].expert, 1);
        let start = Instant::now();
        let many = dry_run_train(256, 0);
        assert_eq!(many.len(), 256);
        assert!(
            start.elapsed().as_millis() < 50,
            "dry-run tick generation must stay cheap"
        );
    }

    #[test]
    fn ateed_probes_use_the_dedicated_engine_binary() {
        let src = include_str!("ateed_studio.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        assert!(production.contains("discover_mujrim_cli"));
        assert!(production.contains("query_best_move"));
        assert!(!production.contains("use eval::"));
        assert!(!production.contains("use gpu::"));
        assert!(!production.contains("AteedNetwork"));
        assert!(!production.contains("SearchEngine"));
        let strength = evaluate_zero_net();
        assert!(format_strength(&strength).contains("expert"));
        let perf = probe_compute();
        assert!(format_perf(&perf).contains("matvec"));
        let tick = monitor_from_progress(&updater::progress::JobProgress::train(3, 8, 0.2, 1));
        assert_eq!(tick.epoch, 3);
        assert!((tick.progress - 0.375).abs() < 0.01);
    }

    #[test]
    fn metric_ring_wraps_at_capacity() {
        let mut ring = MetricRing::default();
        for i in 0..150 {
            ring.push(i as f32);
        }
        assert_eq!(ring.len(), METRIC_RING_CAP);
        let mut out = [0.0f32; METRIC_RING_CAP];
        let n = ring.copy_oldest_first(&mut out);
        assert_eq!(n, METRIC_RING_CAP);
        assert!((out[0] - 50.0).abs() < f32::EPSILON);
        assert!((out[99] - 149.0).abs() < f32::EPSILON);
    }

    #[test]
    fn monitor_from_progress_maps_datagen_and_train_batches() {
        let mut hist = [0u32; updater::progress::HIST_BUCKETS];
        hist[2] = 7;
        let datagen = monitor_from_progress(&updater::progress::JobProgress::datagen_batch(
            updater::progress::DatagenBatch {
                game: 4,
                games: 16,
                positions: 80,
                nps: 9_000,
                throughput: 1.5,
                white: 2,
                draw: 1,
                black: 1,
                pass: 70,
                drop: 5,
                bytes: 2048,
                hist,
            },
        ));
        assert_eq!(datagen.kind, updater::progress::JobKind::Datagen);
        assert_eq!(datagen.nps, 9_000);
        assert!((datagen.mbps - 1.5).abs() < 0.001);
        assert_eq!(datagen.wdl, (2, 1, 1));
        assert_eq!(datagen.pass, 70);
        assert_eq!(datagen.drop, 5);
        assert_eq!(datagen.hist[2], 7);
        let train = monitor_from_progress(&updater::progress::JobProgress::train_batch(
            updater::progress::TrainerBatch {
                epoch: 3,
                epochs: 8,
                loss: 0.2,
                val_loss: Some(0.3),
                expert: 1,
                lr: 0.05,
                mpos: 2.25,
            },
        ));
        assert_eq!(train.kind, updater::progress::JobKind::Train);
        assert!((train.loss - 0.2).abs() < 0.001);
        assert!((train.val_loss - 0.3).abs() < 0.001);
        assert!((train.lr - 0.05).abs() < 0.0001);
        assert!((train.mpos - 2.25).abs() < 0.001);
    }

    #[test]
    fn tournament_index_scan_indexes_and_dedupes() {
        types::init();
        let stamp = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        );
        let root = std::env::temp_dir().join(format!("mujrim-ateed-studio-{stamp}"));
        let db_dir = root.join("library");
        let nnue = root.join("nnue");
        let mut db = mujrim_study::database::StudyDatabase::open(&db_dir).expect("db");
        db.save_tournament(&mujrim_study::tournament_store::StoredTournament {
            id: format!("t-scan-{stamp}"),
            name: "scan".into(),
            format: mujrim_study::tournament::TournamentFormat::RoundRobin,
            created_at: 1,
            status: "complete".into(),
            entrants: Vec::new(),
            results: Vec::new(),
            games: vec![mujrim_study::tournament_store::StoredTournamentGame {
                game_index: 0,
                round: 1,
                white: "A".into(),
                black: "B".into(),
                white_score: 1.0,
                initial_fen: mujrim_study::opening::START_FEN.into(),
                moves: vec!["e2e4".into(), "e7e5".into()],
            }],
        })
        .expect("save");
        let prompt = scan_tournament_index_in(&db, &nnue).expect("new games");
        assert_eq!(prompt.new_games, 1);
        let (dataset, summary) = index_tournament_positions_in(&db, &nnue).expect("index");
        assert!(summary.contains("indexed"));
        assert!(std::path::Path::new(&dataset).is_file());
        assert!(scan_tournament_index_in(&db, &nnue).is_none());
        let mut sources = Vec::new();
        ensure_local_source(&mut sources, &dataset);
        ensure_local_source(&mut sources, &dataset);
        assert_eq!(sources.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }
}
