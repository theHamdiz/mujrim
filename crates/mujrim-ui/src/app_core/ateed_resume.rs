//! Durable Ateed studio job so fetch/train/datagen can be restarted after a crash.

use mujrim_study::durable;

use super::ateed_studio::AteedCliCommand;
use super::settings::AppSettings;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActiveAteedJob {
    pub command: AteedCliCommand,
    pub summary: String,
}

impl ActiveAteedJob {
    pub fn path() -> std::path::PathBuf {
        let mut path = AppSettings::config_path();
        path.set_file_name("active-ateed-job.toml");
        path
    }

    pub fn load() -> Option<Self> {
        let contents = durable::read_text(&Self::path())?;
        toml::from_str(&contents).ok()
    }

    pub fn save(&self) {
        if let Ok(encoded) = toml::to_string_pretty(self) {
            let _ = durable::atomic_write_text(&Self::path(), &encoded);
        }
    }

    pub fn clear() {
        durable::remove_file(&Self::path());
    }

    pub fn from_command(command: AteedCliCommand) -> Self {
        let summary = match &command {
            AteedCliCommand::Fetch { url, output, .. } => {
                format!("Resume fetch of {url} into {output}")
            }
            AteedCliCommand::Train {
                scope,
                epochs,
                data,
                ..
            } => format!("Resume {scope} training for {epochs} epoch(s) on {data}"),
            AteedCliCommand::Datagen { games, output, .. } => {
                format!("Resume self-play datagen of {games} game(s) into {output}")
            }
            AteedCliCommand::Decode { input, output, .. } => {
                format!("Resume decode of {input} into {output}")
            }
            AteedCliCommand::Merge { data, output, .. } => {
                format!("Resume merge of {data} into {output}")
            }
        };
        Self { command, summary }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_summary_names_the_interrupted_train_run() {
        let job = ActiveAteedJob::from_command(AteedCliCommand::Train {
            data: "data.txt".to_owned(),
            mix: String::new(),
            output: "ateed_default.bin".to_owned(),
            epochs: 8,
            lr: "1.0".to_owned(),
            wdl_weight: "0.25".to_owned(),
            scope: "moe".to_owned(),
            base: None,
        });
        assert!(job.summary.contains("moe"));
        assert!(job.summary.contains("8"));
    }
}
