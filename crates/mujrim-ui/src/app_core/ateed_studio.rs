//! Ateed studio domain: gate, multi-source plans, dry-run jobs, and strength probes.
//!
//! The unlock secret is stored only as Base64. Jobs that would download or run a
//! full train stay in the planner; `dry_run_*` and in-memory eval cover the UI.

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AteedJobKind {
    Fetch,
    Train,
    Evaluate,
    Bench,
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
    })
}

pub fn plan_job(
    kind: AteedJobKind,
    sources: &[AteedDataSource],
    scope: &str,
    epochs: u32,
) -> Result<AteedJobPlan, String> {
    match kind {
        AteedJobKind::Fetch => {
            if sources.is_empty() {
                return Err("add at least one data source".to_string());
            }
            let http = sources
                .iter()
                .filter(|source| source.kind == AteedSourceKind::Http)
                .count();
            Ok(AteedJobPlan {
                kind,
                summary: format!(
                    "Queue {} source(s) ({} HTTP Range) without starting a download",
                    sources.len(),
                    http
                ),
            })
        }
        AteedJobKind::Train => {
            parse_train_scope(scope)?;
            if epochs == 0 {
                return Err("epochs must be at least 1".to_string());
            }
            if sources.is_empty() {
                return Err("training needs a dataset source".to_string());
            }
            Ok(AteedJobPlan {
                kind,
                summary: format!(
                    "Dry-run {scope} for {epochs} epoch(s) on {} source(s)",
                    sources.len()
                ),
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
        let sources = [validate_source(AteedSourceKind::Http, "https://example.test/a").unwrap()];
        let fetch = plan_job(AteedJobKind::Fetch, &sources, "heads", 8).unwrap();
        assert!(fetch.summary.contains("without starting a download"));
        assert_eq!(parse_train_scope("heads").unwrap(), "heads");
        assert_eq!(parse_train_scope("moe").unwrap(), "moe");
        assert!(parse_train_scope("bullet").is_err());
        assert!(plan_job(AteedJobKind::Train, &sources, "moe", 4).is_ok());
        assert!(plan_job(AteedJobKind::Train, &sources, "nope", 4).is_err());
        assert!(plan_job(AteedJobKind::Train, &[], "heads", 4).is_err());
        assert!(plan_job(AteedJobKind::Evaluate, &[], "heads", 0).is_ok());
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
    }
}
