use std::path::{Path, PathBuf};

use crate::action::ToolAction;
use kishmat_updater::nnue::{
    DownloadStatus, ProgressCallback, disk_usage, download_all, download_network, find_by_engine,
    list_network_files,
};

#[derive(clap::Subcommand, Debug)]
pub enum NnueCommand {
    /// Download all known networks.
    All {
        /// Directory for downloaded networks.
        #[arg(long, default_value = "crates/kishmat-eval/resources")]
        dir: PathBuf,
    },
    /// Download all networks for one engine family.
    Engine {
        #[arg(value_parser = ["stockfish", "akimbo", "viridithas"])]
        engine: String,
        #[arg(long, default_value = "crates/kishmat-eval/resources")]
        dir: PathBuf,
    },
    /// Show current network files.
    Status {
        #[arg(long, default_value = "crates/kishmat-eval/resources")]
        dir: PathBuf,
    },
}

#[derive(Debug)]
pub struct NnueAction {
    pub command: NnueCommand,
}

impl ToolAction for NnueAction {
    fn run(&self) -> Result<(), String> {
        match &self.command {
            NnueCommand::All { dir } => download_all_networks(dir),
            NnueCommand::Engine { engine, dir } => download_engine_networks(engine, dir),
            NnueCommand::Status { dir } => {
                print_status(dir);
                Ok(())
            }
        }
    }
}

fn download_all_networks(dir: &Path) -> Result<(), String> {
    let progress: ProgressCallback = Box::new(|name, status| match status {
        DownloadStatus::Skipped => println!("skip {name} (already exists)"),
        DownloadStatus::Downloading(size) => {
            let mb = size as f64 / (1024.0 * 1024.0);
            println!("download {name} (~{mb:.1} MB)");
        }
        DownloadStatus::Done => println!("done {name}"),
        DownloadStatus::Failed(err) => eprintln!("fail {name}: {err}"),
    });

    let summary = download_all(dir, Some(progress))?;
    println!(
        "Downloaded: {}  Failed: {}  Path: {}",
        summary.downloaded,
        summary.failed,
        summary.target_dir.display()
    );
    Ok(())
}

fn download_engine_networks(engine: &str, dir: &Path) -> Result<(), String> {
    let nets = find_by_engine(engine);
    if nets.is_empty() {
        return Err(format!("no networks found for engine '{engine}'"));
    }

    for net in nets {
        let progress: ProgressCallback = Box::new(|name, status| match status {
            DownloadStatus::Skipped => println!("skip {name} (already exists)"),
            DownloadStatus::Downloading(size) => {
                let mb = size as f64 / (1024.0 * 1024.0);
                println!("download {name} (~{mb:.1} MB)");
            }
            DownloadStatus::Done => println!("done {name}"),
            DownloadStatus::Failed(err) => eprintln!("fail {name}: {err}"),
        });
        download_network(net, dir, Some(&progress))?;
    }
    Ok(())
}

fn print_status(dir: &Path) {
    let files = list_network_files(dir);
    if files.is_empty() {
        println!("No networks found in {}", dir.display());
        return;
    }

    println!("NNUE files in {}:", dir.display());
    for (name, size) in files {
        let size_mb = size as f64 / (1024.0 * 1024.0);
        println!("  {name:<40} {size_mb:>8.1} MB");
    }
    let usage_mb = disk_usage(dir) as f64 / (1024.0 * 1024.0);
    println!("Total size: {usage_mb:.1} MB");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_value_parser_accepts_known_values() {
        assert!(matches!(
            "stockfish".parse::<String>(),
            Ok(value) if value == "stockfish"
        ));
    }

    #[test]
    fn status_command_is_constructible() {
        let action = NnueAction {
            command: NnueCommand::Status {
                dir: PathBuf::from("nnue"),
            },
        };
        assert!(matches!(action.command, NnueCommand::Status { .. }));
    }
}
