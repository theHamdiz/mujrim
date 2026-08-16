//! Adapter-vs-native measurement card used to close Tournament V2 gaps.
//!
//! Isolated eval benches, nodes-equal match configs, and 3+2 clock configs live
//! here so later Elo claims have a fixed target instead of another mixed field.

use std::path::{Path, PathBuf};
use std::time::Duration;

use eval::nnue::load_network_for_preset;
use serde_json::{Value, json};

use mujrim_protocols::catalog::{dist_engines_root, packaged_engine_path};

use crate::nnue_bench::{self, NnueBenchConfig, NnueBenchResult};
use crate::strength::{EngineSpec, MatchClock, MatchConfig, MatchSummary, run_match};

/// Minimum adapter/native NPS ratio after eval work.
pub const TARGET_NPS_RATIO: f64 = 0.70;
/// Minimum adapter score in a same-clock H2H.
pub const TARGET_CLOCK_H2H: f64 = 0.48;
/// Minimum adapter score in a nodes-equal H2H.
pub const TARGET_NODES_H2H: f64 = 0.45;

/// V2 tournament clock: 3+2 with a 3-minute bonus after 40 moves.
pub fn v2_clock() -> MatchClock {
    MatchClock {
        initial: Duration::from_secs(3 * 60),
        increment: Duration::from_secs(2),
        bonus_after_moves: 40,
        bonus: Duration::from_secs(3 * 60),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdapterPair {
    pub adapter_id: &'static str,
    pub adapter_binary: &'static str,
    pub native_id: &'static str,
    pub native_binary: &'static str,
}

impl AdapterPair {
    pub const fn label(self) -> &'static str {
        self.adapter_id
    }
}

/// Every adapter that underperformed its native counterpart in V2, plus v60.
pub const GAUNTLET_PAIRS: &[AdapterPair] = &[
    AdapterPair {
        adapter_id: "viridithas",
        adapter_binary: "mujrim-viri",
        native_id: "viridithas",
        native_binary: "viridithas",
    },
    AdapterPair {
        adapter_id: "obsidian",
        adapter_binary: "mujrim-obs",
        native_id: "obsidian",
        native_binary: "obsidian",
    },
    AdapterPair {
        adapter_id: "plentychess",
        adapter_binary: "mujrim-plenty",
        native_id: "plentychess",
        native_binary: "plentychess",
    },
    AdapterPair {
        adapter_id: "akimbo",
        adapter_binary: "mujrim-ak",
        native_id: "akimbo",
        native_binary: "akimbo",
    },
    AdapterPair {
        adapter_id: "stockfish",
        adapter_binary: "mujrim-elite",
        native_id: "stockfish",
        native_binary: "stockfish",
    },
    AdapterPair {
        adapter_id: "reckless",
        adapter_binary: "mujrim-v60",
        native_id: "reckless",
        native_binary: "reckless",
    },
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GauntletTargets {
    pub nps_ratio: f64,
    pub clock_h2h: f64,
    pub nodes_h2h: f64,
}

impl GauntletTargets {
    pub const fn v2() -> Self {
        Self {
            nps_ratio: TARGET_NPS_RATIO,
            clock_h2h: TARGET_CLOCK_H2H,
            nodes_h2h: TARGET_NODES_H2H,
        }
    }

    pub fn nps_met(self, adapter_nps: f64, native_nps: f64) -> bool {
        nps_ratio(adapter_nps, native_nps) + f64::EPSILON >= self.nps_ratio
    }

    pub fn clock_met(self, adapter_points: f64, games: f64) -> bool {
        score_ratio(adapter_points, games) + f64::EPSILON >= self.clock_h2h
    }

    pub fn nodes_met(self, adapter_points: f64, games: f64) -> bool {
        score_ratio(adapter_points, games) + f64::EPSILON >= self.nodes_h2h
    }
}

pub fn nps_ratio(adapter_nps: f64, native_nps: f64) -> f64 {
    if !adapter_nps.is_finite() || !native_nps.is_finite() || native_nps <= 0.0 {
        return 0.0;
    }
    adapter_nps / native_nps
}

pub fn score_ratio(points: f64, games: f64) -> f64 {
    if !points.is_finite() || !games.is_finite() || games <= 0.0 {
        return 0.0;
    }
    points / games
}

/// Nodes-equal card: same node budget, 1 thread, 128 MB, no clock.
pub fn nodes_equal_match_config(pairs: usize, nodes: u64) -> MatchConfig {
    MatchConfig {
        pairs,
        nodes_per_move: nodes.max(1),
        move_time: None,
        clock: None,
        hash_mb: 128,
        engine_threads: 1,
        early_stop: false,
        max_engine_memory_mb: 768,
        max_match_memory_mb: 1536,
        ..MatchConfig::default()
    }
}

/// V2-shaped clock card: 3+2, 1 thread, 128 MB.
pub fn clock_match_config(pairs: usize) -> MatchConfig {
    MatchConfig {
        pairs,
        nodes_per_move: 0,
        move_time: None,
        clock: Some(v2_clock()),
        hash_mb: 128,
        engine_threads: 1,
        early_stop: false,
        max_engine_memory_mb: 768,
        max_match_memory_mb: 1536,
        ..MatchConfig::default()
    }
}

pub fn dist_engine_path(root: &Path, engine_id: &str) -> PathBuf {
    packaged_engine_path(&dist_engines_root(root), engine_id)
}

#[derive(Clone, Debug)]
pub struct AdapterEvalSample {
    pub preset: String,
    pub result: NnueBenchResult,
}

pub fn bench_adapter_eval(
    preset: &str,
    config: NnueBenchConfig,
) -> Result<AdapterEvalSample, String> {
    let network = load_network_for_preset(preset)?;
    let result = nnue_bench::run_with_network(config, network)?;
    Ok(AdapterEvalSample {
        preset: preset.to_owned(),
        result,
    })
}

pub fn selected_pairs(preset: Option<&str>) -> Result<Vec<AdapterPair>, String> {
    match preset {
        None => Ok(GAUNTLET_PAIRS.to_vec()),
        Some(preset) => {
            let pairs = GAUNTLET_PAIRS
                .iter()
                .copied()
                .filter(|pair| pair.adapter_id == preset || pair.adapter_binary == preset)
                .collect::<Vec<_>>();
            if pairs.is_empty() {
                Err(format!(
                    "unknown gauntlet preset '{preset}' (expected one of {})",
                    GAUNTLET_PAIRS
                        .iter()
                        .map(|pair| pair.adapter_id)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            } else {
                Ok(pairs)
            }
        }
    }
}

pub fn is_strength_pair(pair: AdapterPair) -> bool {
    pair.native_id != "stockfish" && pair.native_id != "lc0" && pair.adapter_id != "lc0"
}

pub fn resolve_engine_binary(root: &Path, engine_id: &str) -> Result<PathBuf, String> {
    let path = dist_engine_path(root, engine_id);
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "could not find {engine_id} in {}",
            dist_engines_root(root).display()
        ))
    }
}

pub fn resolve_pair(root: &Path, pair: AdapterPair) -> Result<(PathBuf, PathBuf), String> {
    let adapter = resolve_engine_binary(root, pair.adapter_binary)
        .map_err(|error| format!("{}: {error}", pair.adapter_binary))?;
    let native = resolve_engine_binary(root, pair.native_binary)
        .map_err(|error| format!("{}: {error}", pair.native_binary))?;
    Ok((adapter, native))
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayedMatchCard {
    pub games: u64,
    pub points: f64,
    pub score: f64,
    pub adapter_nps: Option<f64>,
    pub native_nps: Option<f64>,
    pub error: Option<String>,
}

impl PlayedMatchCard {
    pub fn from_summary(summary: &MatchSummary) -> Self {
        let json = summary.to_json_value();
        Self {
            games: summary.scores.games(),
            points: summary.scores.wins as f64 + 0.5 * summary.scores.draws as f64,
            score: summary.scores.score_rate(),
            adapter_nps: json_number(&json["telemetry"]["candidate"]["nps"]),
            native_nps: json_number(&json["telemetry"]["reference"]["nps"]),
            error: summary.error.clone(),
        }
    }

    pub fn nps_ratio(&self) -> f64 {
        match (self.adapter_nps, self.native_nps) {
            (Some(adapter), Some(native)) => nps_ratio(adapter, native),
            _ => 0.0,
        }
    }
}

fn json_number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_u64().map(|number| number as f64))
}

pub fn play_resolved_pair(
    adapter: PathBuf,
    native: PathBuf,
    adapter_name: &str,
    native_name: &str,
    config: MatchConfig,
) -> PlayedMatchCard {
    let mut candidate = EngineSpec::new(adapter);
    candidate.name = adapter_name.to_owned();
    let mut reference = EngineSpec::new(native);
    reference.name = native_name.to_owned();
    PlayedMatchCard::from_summary(&run_match(candidate, reference, None, config))
}

fn played_card_json(card: &PlayedMatchCard, target: f64, met: bool) -> Value {
    json!({
        "games": card.games,
        "points": card.points,
        "score": card.score,
        "adapter_nps": card.adapter_nps,
        "native_nps": card.native_nps,
        "nps_ratio": card.nps_ratio(),
        "target": target,
        "met": met,
        "error": card.error,
    })
}

#[derive(Clone, Debug)]
pub struct ValidationRequest {
    pub root: PathBuf,
    pub preset: Option<String>,
    pub play: bool,
    pub play_clock: bool,
    pub play_nodes: bool,
    pub pairs: usize,
    pub nodes: u64,
    pub eval: NnueBenchConfig,
}

pub fn run_validation(request: &ValidationRequest) -> Result<Value, String> {
    let pairs = selected_pairs(request.preset.as_deref())?;
    let targets = GauntletTargets::v2();
    let mut samples = Vec::new();
    let mut pair_cards = Vec::new();
    let mut errors = Vec::new();

    for pair in pairs {
        let eval = match bench_adapter_eval(pair.adapter_id, request.eval) {
            Ok(sample) => {
                samples.push(sample.clone());
                Some(sample)
            }
            Err(error) => {
                errors.push(format!("{} eval: {error}", pair.adapter_id));
                None
            }
        };

        let mut nodes_card = None;
        let mut clock_card = None;
        if request.play && request.preset.is_none() && !is_strength_pair(pair) {
            errors.push(format!(
                "{}: skipped official {} host pair (not a strength card)",
                pair.adapter_id, pair.native_id
            ));
        } else if request.play {
            match resolve_pair(&request.root, pair) {
                Ok((adapter, native)) => {
                    if request.play_nodes {
                        nodes_card = Some(play_resolved_pair(
                            adapter.clone(),
                            native.clone(),
                            pair.adapter_binary,
                            pair.native_binary,
                            nodes_equal_match_config(request.pairs.max(1), request.nodes),
                        ));
                    }
                    if request.play_clock {
                        clock_card = Some(play_resolved_pair(
                            adapter,
                            native,
                            pair.adapter_binary,
                            pair.native_binary,
                            clock_match_config(request.pairs.max(1)),
                        ));
                    }
                }
                Err(error) => errors.push(format!("{} binaries: {error}", pair.adapter_id)),
            }
        }

        let nps_ratio = clock_card
            .as_ref()
            .or(nodes_card.as_ref())
            .map(PlayedMatchCard::nps_ratio);
        let nps_met = nps_ratio.is_some_and(|ratio| ratio + f64::EPSILON >= targets.nps_ratio);
        let clock_met = clock_card
            .as_ref()
            .is_some_and(|card| targets.clock_met(card.points, card.games as f64));
        let nodes_met = nodes_card
            .as_ref()
            .is_some_and(|card| targets.nodes_met(card.points, card.games as f64));

        pair_cards.push(json!({
            "adapter_id": pair.adapter_id,
            "adapter_binary": pair.adapter_binary,
            "native_id": pair.native_id,
            "native_binary": pair.native_binary,
            "eval": eval.map(|sample| json!({
                "network": sample.result.network,
                "hot_ns_per_eval": sample.result.hot_ns_per_eval(),
                "incremental_ns_per_eval": sample.result.incremental_ns_per_eval(),
            })),
            "nodes_equal": nodes_card.as_ref().map(|card| {
                played_card_json(card, targets.nodes_h2h, nodes_met)
            }),
            "clock": clock_card.as_ref().map(|card| {
                played_card_json(card, targets.clock_h2h, clock_met)
            }),
            "verdicts": {
                "nps": nps_met,
                "clock_h2h": clock_met,
                "nodes_h2h": nodes_met,
                "nps_ratio": nps_ratio,
            },
        }));
    }

    let mut payload = eval_card_json(&samples);
    payload["type"] = json!("mujrim-adapter-validation-card");
    payload["errors"] = json!(errors);
    payload["played"] = json!(request.play);
    payload["pairs"] = json!(pair_cards);
    payload["nodes_equal"] = json!({
        "pairs": request.pairs.max(1),
        "nodes": request.nodes,
        "hash_mb": nodes_equal_match_config(request.pairs.max(1), request.nodes).hash_mb,
    });
    payload["clock"] = json!({
        "pairs": request.pairs.max(1),
        "hash_mb": clock_match_config(request.pairs.max(1)).hash_mb,
        "label": "3+2 (+3 after 40)",
    });
    Ok(payload)
}

pub fn eval_card_json(samples: &[AdapterEvalSample]) -> Value {
    json!({
        "type": "mujrim-adapter-eval-card",
        "targets": {
            "nps_ratio": TARGET_NPS_RATIO,
            "clock_h2h": TARGET_CLOCK_H2H,
            "nodes_h2h": TARGET_NODES_H2H,
        },
        "samples": samples.iter().map(|sample| {
            json!({
                "preset": sample.preset,
                "network": sample.result.network,
                "hot_ns_per_eval": sample.result.hot_ns_per_eval(),
                "incremental_ns_per_eval": sample.result.incremental_ns_per_eval(),
                "suite_ns_per_eval": sample.result.suite_ns_per_eval(),
                "hot_evals_per_second": sample.result.hot_evals_per_second(),
                "incremental_evals_per_second": sample.result.incremental_evals_per_second(),
            })
        }).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_targets_and_pairs_are_stable() {
        let targets = GauntletTargets::v2();
        assert_eq!(targets.nps_ratio, 0.70);
        assert_eq!(targets.clock_h2h, 0.48);
        assert_eq!(targets.nodes_h2h, 0.45);
        assert_eq!(GAUNTLET_PAIRS.len(), 6);
        assert_eq!(GAUNTLET_PAIRS[0].adapter_binary, "mujrim-viri");
        assert_eq!(GAUNTLET_PAIRS[5].adapter_binary, "mujrim-v60");
        assert!(targets.nps_met(700_000.0, 1_000_000.0));
        assert!(!targets.nps_met(699_000.0, 1_000_000.0));
        assert!(targets.clock_met(48.0, 100.0));
        assert!(!targets.clock_met(47.0, 100.0));
        assert!(targets.nodes_met(45.0, 100.0));
        assert!(!targets.nodes_met(44.0, 100.0));
    }

    #[test]
    fn match_configs_lock_v2_resources() {
        let nodes = nodes_equal_match_config(4, 20_000);
        assert_eq!(nodes.nodes_per_move, 20_000);
        assert!(nodes.clock.is_none());
        assert_eq!(nodes.hash_mb, 128);
        assert_eq!(nodes.engine_threads, 1);
        assert!(!nodes.early_stop);

        let clock = clock_match_config(4);
        assert_eq!(clock.nodes_per_move, 0);
        let MatchClock {
            initial,
            increment,
            bonus_after_moves,
            bonus,
        } = clock.clock.expect("v2 clock");
        assert_eq!(initial, Duration::from_secs(180));
        assert_eq!(increment, Duration::from_secs(2));
        assert_eq!(bonus_after_moves, 40);
        assert_eq!(bonus, Duration::from_secs(180));
        assert_eq!(clock.hash_mb, 128);
    }

    #[test]
    fn dist_paths_use_catalog_layout() {
        let root = Path::new("/opt/mujrim");
        let engines = dist_engines_root(root);
        assert_eq!(
            dist_engine_path(root, "mujrim-viri"),
            packaged_engine_path(&engines, "mujrim-viri")
        );
        assert_eq!(
            dist_engine_path(root, "viridithas"),
            packaged_engine_path(&engines, "viridithas")
        );
        let viri = dist_engine_path(root, "mujrim-viri");
        assert!(viri.starts_with(&engines), "{viri:?}");
        assert!(viri.components().any(|part| part.as_os_str() == "mujrim"));
        assert!(!viri.components().any(|part| part.as_os_str() == "target"));
        assert!(!engines.ends_with("dist/engines"));
    }

    #[test]
    fn score_and_nps_helpers_reject_empty_samples() {
        assert_eq!(nps_ratio(100.0, 0.0), 0.0);
        assert_eq!(score_ratio(3.0, 0.0), 0.0);
        assert!((nps_ratio(77_000.0, 764_000.0) - 77_000.0 / 764_000.0).abs() < 1e-12);
    }

    #[test]
    fn akimbo_eval_card_runs_embedded_network() {
        let sample = bench_adapter_eval(
            "akimbo",
            NnueBenchConfig {
                iterations: 8,
                warmup: 1,
            },
        )
        .expect("akimbo eval bench");
        assert_eq!(sample.preset, "akimbo");
        assert!(sample.result.incremental_ns_per_eval().is_finite());
        let json = eval_card_json(&[sample]);
        assert_eq!(json["type"], "mujrim-adapter-eval-card");
        assert_eq!(json["targets"]["nps_ratio"], 0.70);
    }

    #[test]
    fn selected_pairs_accept_adapter_id_or_binary() {
        assert_eq!(selected_pairs(None).unwrap().len(), 6);
        assert_eq!(
            selected_pairs(Some("viridithas")).unwrap()[0].adapter_binary,
            "mujrim-viri"
        );
        assert_eq!(
            selected_pairs(Some("mujrim-v60")).unwrap()[0].adapter_id,
            "reckless"
        );
        assert!(selected_pairs(Some("lc0")).is_err());
        assert_eq!(
            GAUNTLET_PAIRS
                .iter()
                .copied()
                .filter(|pair| is_strength_pair(*pair))
                .count(),
            5
        );
        assert!(!is_strength_pair(
            *GAUNTLET_PAIRS
                .iter()
                .find(|pair| pair.native_id == "stockfish")
                .expect("stockfish pair")
        ));
    }

    #[test]
    fn resolve_pair_prefers_dist_layout_when_present() {
        let root = std::env::temp_dir().join(format!(
            "mujrim-gauntlet-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let pair = GAUNTLET_PAIRS
            .iter()
            .find(|pair| pair.adapter_binary == "mujrim-ak")
            .copied()
            .expect("akimbo pair");
        let adapter = dist_engine_path(&root, pair.adapter_binary);
        let native = dist_engine_path(&root, pair.native_binary);
        std::fs::create_dir_all(adapter.parent().expect("bin dir")).unwrap();
        std::fs::create_dir_all(native.parent().expect("bin dir")).unwrap();
        std::fs::write(&adapter, []).unwrap();
        std::fs::write(&native, []).unwrap();
        let resolved = resolve_pair(&root, pair).expect("dist binaries resolve");
        assert_eq!(resolved.0, adapter);
        assert_eq!(resolved.1, native);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_native_uses_dist_arch_engines() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = resolve_engine_binary(&root, "viridithas").expect("dist viridithas");
        assert!(
            path.starts_with(dist_engines_root(&root)),
            "expected dist/<arch>/engines, got {path:?}"
        );
        assert!(path.ends_with("viridithas"), "{path:?}");
    }

    #[test]
    fn resolve_engine_ignores_target_release_and_repo_engines() {
        let root = std::env::temp_dir().join(format!(
            "mujrim-gauntlet-release-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let release = root.join("target").join("release").join("mujrim-ak");
        let vendored = root
            .join("engines")
            .join("akimbo")
            .join("bin")
            .join("linux-x86_64")
            .join("akimbo");
        let unscoped = root
            .join("dist")
            .join("engines")
            .join("mujrim-ak")
            .join("bin")
            .join("linux-x86_64")
            .join("mujrim-ak");
        std::fs::create_dir_all(release.parent().expect("release dir")).unwrap();
        std::fs::create_dir_all(vendored.parent().expect("vendored dir")).unwrap();
        std::fs::create_dir_all(unscoped.parent().expect("unscoped dir")).unwrap();
        std::fs::write(&release, []).unwrap();
        std::fs::write(&vendored, []).unwrap();
        std::fs::write(&unscoped, []).unwrap();
        let err = resolve_engine_binary(&root, "mujrim-ak").expect_err("only dist/<arch>/engines");
        assert!(err.contains("dist"), "{err}");
        assert!(err.contains("engines"), "{err}");
        assert!(resolve_engine_binary(&root, "akimbo").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_pair_reports_missing_binaries() {
        let err = resolve_pair(
            Path::new("/no/such/mujrim-gauntlet-root"),
            GAUNTLET_PAIRS[0],
        )
        .expect_err("missing binaries");
        assert!(err.contains("mujrim-viri"), "{err}");
    }

    #[test]
    fn played_card_from_forfeit_is_a_complete_loss() {
        use crate::strength::{FailedEngine, forfeit_match_summary};
        let candidate = EngineSpec::new(PathBuf::from("/no/such/mujrim-ak"));
        let reference = EngineSpec::new(PathBuf::from("/no/such/akimbo"));
        let summary = forfeit_match_summary(
            &candidate,
            &reference,
            &nodes_equal_match_config(1, 1_000),
            FailedEngine::Candidate,
            "missing binary",
        );
        let card = PlayedMatchCard::from_summary(&summary);
        assert_eq!(card.games, 2);
        assert_eq!(card.points, 0.0);
        assert_eq!(card.score, 0.0);
        assert!(
            card.error
                .as_deref()
                .is_some_and(|error| error.contains("forfeit"))
        );
        let targets = GauntletTargets::v2();
        assert!(!targets.nodes_met(card.points, card.games as f64));
    }

    #[test]
    fn validation_without_play_skips_matches_and_keeps_eval() {
        let card = run_validation(&ValidationRequest {
            root: PathBuf::from("/no/such/mujrim-gauntlet-root"),
            preset: Some("akimbo".to_owned()),
            play: false,
            play_clock: true,
            play_nodes: true,
            pairs: 1,
            nodes: 1_000,
            eval: NnueBenchConfig {
                iterations: 8,
                warmup: 1,
            },
        })
        .expect("akimbo eval card");
        assert_eq!(card["type"], "mujrim-adapter-validation-card");
        assert_eq!(card["played"], false);
        assert_eq!(card["pairs"][0]["adapter_id"], "akimbo");
        assert!(card["pairs"][0]["eval"].is_object());
        assert!(card["pairs"][0]["nodes_equal"].is_null());
        assert!(card["pairs"][0]["clock"].is_null());
        assert_eq!(card["pairs"][0]["verdicts"]["nps"], false);
    }

    #[test]
    fn validation_play_without_binaries_records_skip() {
        let card = run_validation(&ValidationRequest {
            root: PathBuf::from("/no/such/mujrim-gauntlet-root"),
            preset: Some("akimbo".to_owned()),
            play: true,
            play_clock: true,
            play_nodes: true,
            pairs: 1,
            nodes: 1_000,
            eval: NnueBenchConfig {
                iterations: 8,
                warmup: 1,
            },
        })
        .expect("akimbo card with skipped matches");
        assert_eq!(card["played"], true);
        assert!(card["pairs"][0]["nodes_equal"].is_null());
        let errors = card["errors"].as_array().expect("errors");
        assert!(
            errors
                .iter()
                .any(|error| error.as_str().is_some_and(|text| text.contains("binaries"))),
            "{errors:?}"
        );
    }

    #[test]
    fn v2_validation_card_is_adapter_vs_native_only() {
        assert_eq!(GAUNTLET_PAIRS.len(), 6);
        assert!(
            GAUNTLET_PAIRS
                .iter()
                .any(|pair| pair.adapter_binary == "mujrim-elite")
        );
        assert!(
            GAUNTLET_PAIRS
                .iter()
                .any(|pair| pair.adapter_binary == "mujrim-v60")
        );
        assert!(
            !GAUNTLET_PAIRS
                .iter()
                .any(|pair| pair.native_id == "lc0" || pair.adapter_id == "lc0")
        );
        let clock = clock_match_config(50);
        let nodes = nodes_equal_match_config(50, 25_000);
        assert_eq!(clock.engine_threads, 1);
        assert_eq!(clock.hash_mb, 128);
        assert!(clock.clock.is_some());
        assert_eq!(nodes.nodes_per_move, 25_000);
        assert!(nodes.clock.is_none());
    }
}
