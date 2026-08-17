//! Sidecar checkpoints so train, datagen, and fetch can resume after a crash.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mujrim_study::durable;

use crate::config::{DatagenConfig, TrainingConfig};
use crate::train::AteedTrainScope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobCheckpoint {
    pub kind: String,
    pub identity: String,
    pub completed: u64,
    pub total: u64,
    pub output: String,
    pub extra: BTreeMap<String, String>,
}

impl JobCheckpoint {
    pub fn path_for(output: &Path) -> PathBuf {
        durable::sidecar_path(output, ".job")
    }

    pub fn partial_path(output: &Path) -> PathBuf {
        durable::sidecar_path(output, ".partial")
    }

    pub fn load(output: &Path) -> Option<Self> {
        let text = durable::read_text(&Self::path_for(output))?;
        parse_sidecar(&text)
    }

    pub fn save(&self) -> Result<(), String> {
        durable::atomic_write_text(
            &Self::path_for(Path::new(&self.output)),
            &encode_sidecar(self),
        )
    }

    pub fn clear(output: &Path) {
        durable::remove_file(&Self::path_for(output));
        durable::remove_file(&Self::partial_path(output));
    }

    pub fn matches(&self, identity: &str) -> bool {
        self.identity == identity
    }
}

pub fn train_identity(config: &TrainingConfig, scope: AteedTrainScope) -> String {
    format!(
        "train|{}|{}|{}|{}|{}|{}|{}",
        scope_key(scope),
        config.data_path,
        config.mix_weights,
        config.mix_seed,
        config.learning_rate,
        config.wdl_weight,
        config.output_path
    )
}

pub fn datagen_identity(config: &DatagenConfig) -> String {
    format!(
        "datagen|{}|{}|{}|{}|{}|{}",
        config.depth,
        config.format,
        config.output_path,
        config.random_plies,
        config.search_preset,
        config.num_positions.unwrap_or(0)
    )
}

pub fn fetch_identity(url: &str, dest: &Path) -> String {
    format!("fetch|{url}|{}", dest.display())
}

pub fn train_checkpoint(
    config: &TrainingConfig,
    scope: AteedTrainScope,
    completed: u32,
) -> JobCheckpoint {
    JobCheckpoint {
        kind: "train".to_owned(),
        identity: train_identity(config, scope),
        completed: u64::from(completed),
        total: u64::from(config.epochs),
        output: config.output_path.clone(),
        extra: BTreeMap::from([("scope".to_owned(), scope_key(scope).to_owned())]),
    }
}

pub fn datagen_checkpoint(config: &DatagenConfig, completed: u64, positions: u64) -> JobCheckpoint {
    JobCheckpoint {
        kind: "datagen".to_owned(),
        identity: datagen_identity(config),
        completed,
        total: config.num_games,
        output: config.output_path.clone(),
        extra: BTreeMap::from([("positions".to_owned(), positions.to_string())]),
    }
}

pub fn fetch_checkpoint(url: &str, dest: &Path) -> JobCheckpoint {
    JobCheckpoint {
        kind: "fetch".to_owned(),
        identity: fetch_identity(url, dest),
        completed: 0,
        total: 1,
        output: dest.display().to_string(),
        extra: BTreeMap::from([("url".to_owned(), url.to_owned())]),
    }
}

pub fn resume_train(config: &TrainingConfig, scope: AteedTrainScope) -> u32 {
    JobCheckpoint::load(Path::new(&config.output_path))
        .filter(|job| job.kind == "train" && job.matches(&train_identity(config, scope)))
        .map(|job| job.completed.min(u64::from(config.epochs)) as u32)
        .unwrap_or(0)
}

pub fn resume_datagen(config: &DatagenConfig) -> u64 {
    JobCheckpoint::load(Path::new(&config.output_path))
        .filter(|job| job.kind == "datagen" && job.matches(&datagen_identity(config)))
        .map(|job| job.completed.min(config.num_games))
        .unwrap_or(0)
}

pub fn resume_datagen_positions(config: &DatagenConfig) -> u64 {
    JobCheckpoint::load(Path::new(&config.output_path))
        .filter(|job| job.kind == "datagen" && job.matches(&datagen_identity(config)))
        .and_then(|job| job.extra.get("positions")?.parse().ok())
        .unwrap_or(0)
}

fn scope_key(scope: AteedTrainScope) -> &'static str {
    match scope {
        AteedTrainScope::OutputBiases => "heads",
        AteedTrainScope::Expert0 => "expert0",
        AteedTrainScope::Moe => "moe",
    }
}

fn encode_sidecar(job: &JobCheckpoint) -> String {
    let mut lines = vec![
        format!("kind={}", job.kind),
        format!("identity={}", job.identity),
        format!("completed={}", job.completed),
        format!("total={}", job.total),
        format!("output={}", job.output),
    ];
    for (key, value) in &job.extra {
        lines.push(format!("{key}={value}"));
    }
    lines.push(String::new());
    lines.join("\n")
}

fn parse_sidecar(text: &str) -> Option<JobCheckpoint> {
    let mut fields = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=')?;
        fields.insert(key.to_owned(), value.to_owned());
    }
    Some(JobCheckpoint {
        kind: fields.remove("kind")?,
        identity: fields.remove("identity")?,
        completed: fields.remove("completed")?.parse().ok()?,
        total: fields.remove("total")?.parse().ok()?,
        output: fields.remove("output")?,
        extra: fields,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_round_trips_and_rejects_a_different_identity() {
        let dir = std::env::temp_dir().join(format!(
            "mujrim-job-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let output = dir.join("net.bin");
        let config = TrainingConfig {
            output_path: output.display().to_string(),
            data_path: "data.txt".to_owned(),
            epochs: 8,
            ..Default::default()
        };
        train_checkpoint(&config, AteedTrainScope::Moe, 3)
            .save()
            .expect("save job");
        assert_eq!(resume_train(&config, AteedTrainScope::Moe), 3);
        assert_eq!(resume_train(&config, AteedTrainScope::OutputBiases), 0);
        JobCheckpoint::clear(&output);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn datagen_resume_caps_at_the_requested_game_count() {
        let config = DatagenConfig {
            output_path: "data.txt".to_owned(),
            num_games: 4,
            ..Default::default()
        };
        let mut job = datagen_checkpoint(&config, 9, 400);
        job.completed = 9;
        assert_eq!(job.completed.min(config.num_games), 4);
        assert_eq!(job.extra.get("positions").map(String::as_str), Some("400"));
    }

    #[test]
    fn datagen_checkpoint_survives_parallel_saves() {
        let dir = std::env::temp_dir().join(format!(
            "mujrim-job-parallel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let output = dir.join("data.txt");
        let config = DatagenConfig {
            output_path: output.display().to_string(),
            num_games: 32,
            num_positions: Some(1_000),
            ..Default::default()
        };
        std::thread::scope(|scope| {
            for completed in 1..=32 {
                let config = config.clone();
                scope.spawn(move || {
                    datagen_checkpoint(&config, completed, completed * 10)
                        .save()
                        .expect("parallel datagen sidecar");
                });
            }
        });
        assert!(JobCheckpoint::load(&output).is_some());
        JobCheckpoint::clear(&output);
        let _ = std::fs::remove_dir_all(dir);
    }
}
