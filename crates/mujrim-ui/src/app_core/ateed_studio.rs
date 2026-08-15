//! Ateed studio domain: gate, CLI discovery, job plans, and in-memory probes.
//!
//! Fetch / train / datagen require a `mujrim` CLI beside the UI (or in
//! `target/{debug,release}`). Evaluate and latency probes stay in-process.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use eval::nnue::ateed_format::{L1, L2};
use eval::nnue::{AteedNetwork, wdl_variance};
use gpu::{CpuCompute, TrainCompute};
use types::Board;

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

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq)]
pub struct AteedMonitorTick {
    pub epoch: u32,
    pub progress: f32,
    pub loss: f32,
    pub expert: usize,
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
                return Err("datagen source must be a positive game count".to_string());
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

pub fn mujrim_cli_name() -> &'static str {
    if cfg!(windows) {
        "mujrim.exe"
    } else {
        "mujrim"
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

pub fn discover_mujrim_cli(executable: &Path, current_dir: &Path) -> Option<PathBuf> {
    let name = mujrim_cli_name();
    let mut candidates = Vec::new();
    if let Some(dir) = executable.parent() {
        candidates.push(dir.join(name));
        if let Some(parent) = dir.parent() {
            candidates.push(parent.join(name));
        }
    }
    candidates.push(current_dir.join(name));
    candidates.push(current_dir.join("target").join("release").join(name));
    candidates.push(current_dir.join("target").join("debug").join(name));
    candidates.into_iter().find(|path| is_runnable_cli(path))
}

pub fn discover_mujrim_cli_from_environment() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let cwd = std::env::current_dir().ok()?;
    discover_mujrim_cli(&exe, &cwd)
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
            depth,
            output,
            format,
        } => vec![
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
        ],
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

pub fn run_mujrim_cli(
    cli: &Path,
    args: &[String],
    mut on_line: impl FnMut(&str),
) -> Result<i32, String> {
    let mut child = Command::new(cli)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to start {}: {error}", cli.display()))?;
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
    Ok(status.code().unwrap_or(1))
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
                summary: format!("Fetch {http} HTTP source(s) via mujrim train fetch"),
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
            let games = sources
                .iter()
                .find(|source| source.kind == AteedSourceKind::Datagen)
                .and_then(|source| source.value.parse::<u64>().ok())
                .unwrap_or(0);
            if games == 0 {
                return Err("add a datagen source with a positive game count".to_string());
            }
            Ok(AteedJobPlan {
                kind,
                summary: format!("Generate {games} self-play game(s) via mujrim train datagen"),
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
                epoch,
                progress,
                loss: (1.0 - progress) * 0.85 + 0.05,
                expert,
                message: format!("epoch {epoch}/{epochs} routed expert {expert}"),
            }
        })
        .collect()
}

pub fn evaluate_zero_net() -> AteedStrengthReport {
    types::init();
    let net = AteedNetwork::zero();
    let board = Board::new();
    let eval = net.evaluate_full(&board);
    AteedStrengthReport {
        score: eval.score,
        variance: wdl_variance(eval.wdl),
        expert: eval.expert,
        file_size: eval::nnue::ateed_format::FILE_SIZE as u64,
    }
}

pub fn probe_compute() -> AteedPerfReport {
    types::init();
    let matrix = vec![0.25f32; L2 * L1];
    let vector = vec![0.5f32; L1];
    let mut out = vec![0.0f32; L2];
    let start = Instant::now();
    CpuCompute.matvec_f32(&matrix, &vector, L2, L1, &mut out);
    let matvec_ns = start.elapsed().as_nanos();
    let net = AteedNetwork::zero();
    let board = Board::new();
    let start = Instant::now();
    let _ = net.evaluate(&board);
    let eval_ns = start.elapsed().as_nanos();
    AteedPerfReport { matvec_ns, eval_ns }
}

pub fn format_strength(report: &AteedStrengthReport) -> String {
    format!(
        "score {:+} · WDL σ² {} · expert {} · {} bytes",
        report.score, report.variance, report.expert, report.file_size
    )
}

pub fn monitor_from_progress(progress: &updater::progress::JobProgress) -> AteedMonitorTick {
    AteedMonitorTick {
        epoch: progress.epoch.unwrap_or(0),
        progress: (progress.pct / 100.0).clamp(0.0, 1.0),
        loss: progress.loss.unwrap_or(0.0),
        expert: progress.expert.unwrap_or(0),
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
        let datagen = [validate_source(AteedSourceKind::Datagen, "16").unwrap()];
        assert!(plan_job(AteedJobKind::Datagen, &datagen, "heads", 1, true).is_ok());
        assert!(plan_job(AteedJobKind::Datagen, &[], "heads", 1, true).is_err());
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
        let cli = root.join(mujrim_cli_name());
        std::fs::write(&cli, []).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&cli).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&cli, perms).unwrap();
        }
        let ui = root.join("elsewhere").join("mujrim-ui");
        std::fs::create_dir_all(ui.parent().unwrap()).unwrap();
        std::fs::write(&ui, []).unwrap();
        let found = discover_mujrim_cli(&ui, &root).expect("discover sibling CLI");
        assert_eq!(found, cli);
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
    fn evaluate_zero_net_and_perf_probe_stay_in_process() {
        let strength = evaluate_zero_net();
        assert_eq!(strength.score, 0);
        assert_eq!(
            strength.file_size,
            eval::nnue::ateed_format::FILE_SIZE as u64
        );
        assert!(format_strength(&strength).contains("expert"));
        let perf = probe_compute();
        assert!(perf.matvec_ns > 0);
        assert!(perf.eval_ns > 0);
        assert!(format_perf(&perf).contains("matvec"));
        let tick = monitor_from_progress(&updater::progress::JobProgress::train(3, 8, 0.2, 1));
        assert_eq!(tick.epoch, 3);
        assert!((tick.progress - 0.375).abs() < 0.01);
    }
}
