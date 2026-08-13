use std::path::{Path, PathBuf};

use crate::action::ToolAction;
use mujrim_updater::nnue::{
    DownloadStatus, NETWORKS, NetStatus, ProgressCallback, check_installed, disk_usage,
    download_all, download_network, find_by_engine, list_network_files, load_manifest,
    needs_update,
};

#[derive(clap::Subcommand, Debug)]
pub enum NnueCommand {
    /// Download all known networks.
    All {
        /// Directory for downloaded networks.
        #[arg(long, default_value = "crates/mujrim-eval/resources")]
        dir: PathBuf,
    },
    /// Download all networks for one engine family.
    Engine {
        #[arg(value_parser = ["akimbo", "reckless", "stockfish", "viridithas", "alexandria"])]
        engine: String,
        #[arg(long, default_value = "crates/mujrim-eval/resources")]
        dir: PathBuf,
    },
    /// Show current network files and registry status.
    Status {
        #[arg(long, default_value = "crates/mujrim-eval/resources")]
        dir: PathBuf,
    },
    /// Check which networks have updates available.
    CheckUpdates {
        #[arg(long, default_value = "crates/mujrim-eval/resources")]
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
            NnueCommand::CheckUpdates { dir } => {
                check_updates(dir);
                Ok(())
            }
        }
    }
}

fn download_all_networks(dir: &Path) -> Result<(), String> {
    let progress: ProgressCallback = Box::new(|name, status| match status {
        DownloadStatus::Skipped => println!("  skip  {name} (up to date)"),
        DownloadStatus::Downloading(size) => {
            let mb = size as f64 / (1024.0 * 1024.0);
            println!("  ↓     {name} (~{mb:.1} MB)");
        }
        DownloadStatus::Done => println!("  done  {name}"),
        DownloadStatus::Failed(err) => eprintln!("  FAIL  {name}: {err}"),
    });

    let summary = download_all(dir, Some(progress))?;
    println!(
        "\nDownloaded: {}  Skipped: {}  Failed: {}  Path: {}",
        summary.downloaded,
        summary.skipped,
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

    println!("Downloading {} network(s) for {engine}...", nets.len());
    for net in nets {
        let progress: ProgressCallback = Box::new(|name, status| match status {
            DownloadStatus::Skipped => println!("  skip  {name} (up to date)"),
            DownloadStatus::Downloading(size) => {
                let mb = size as f64 / (1024.0 * 1024.0);
                println!("  ↓     {name} (~{mb:.1} MB)");
            }
            DownloadStatus::Done => println!("  done  {name}"),
            DownloadStatus::Failed(err) => eprintln!("  FAIL  {name}: {err}"),
        });
        download_network(net, dir, Some(&progress))?;
    }
    Ok(())
}

fn print_status(dir: &Path) {
    let statuses = check_installed(dir);

    println!("NNUE Network Registry:");
    println!("{}", status_header());
    println!("  {}", "─".repeat(80));
    for (net, status) in &statuses {
        let (icon, status_text) = match status {
            NetStatus::Current => ("✓", "current"),
            NetStatus::UpdateAvailable => ("⬆", "update available"),
            NetStatus::Missing => ("·", "not downloaded"),
        };
        let size_str = if *status != NetStatus::Missing {
            let size = std::fs::metadata(dir.join(net.filename))
                .map(|m| m.len())
                .unwrap_or(0);
            format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
        } else {
            format!("~{:.1} MB", net.approx_size as f64 / (1024.0 * 1024.0))
        };
        println!(
            "  {icon} {:<19} {:<12} {:<26} {:>8}  {status_text}",
            net.id, net.engine, net.filename, size_str
        );
    }

    // Unregistered files
    let files = list_network_files(dir);
    let known_filenames: Vec<&str> = NETWORKS.iter().map(|n| n.filename).collect();
    let unknown: Vec<_> = files
        .iter()
        .filter(|(name, _)| !known_filenames.contains(&name.as_str()))
        .collect();
    if !unknown.is_empty() {
        println!("\n  Unregistered files:");
        for (name, size) in &unknown {
            let size_mb = *size as f64 / (1024.0 * 1024.0);
            println!("    ? {name:<40} {size_mb:>8.1} MB");
        }
    }

    let usage_mb = disk_usage(dir) as f64 / (1024.0 * 1024.0);
    println!(
        "\n  Total disk usage: {usage_mb:.1} MB in {}",
        dir.display()
    );
}

fn status_header() -> String {
    format!(
        "  {:<20} {:<12} {:<26} {:>8}  Status",
        "ID", "Engine", "Filename", "Size"
    )
}

fn check_updates(dir: &Path) {
    let manifest = load_manifest(dir);
    let mut any_update = false;

    for net in NETWORKS {
        if dir.join(net.filename).exists() && needs_update(net, &manifest) {
            if !any_update {
                println!("Updates available:");
                any_update = true;
            }
            let old_upstream = manifest
                .get(net.id)
                .map(|e| e.upstream_name.as_str())
                .unwrap_or("(no manifest)");
            println!(
                "  ⬆ {} ({}) : {} → {}",
                net.id, net.engine, old_upstream, net.upstream_name
            );
        }
    }

    if !any_update {
        println!("All installed networks are up to date.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_value_parser_accepts_known_values() {
        for engine in [
            "akimbo",
            "reckless",
            "stockfish",
            "viridithas",
            "alexandria",
        ] {
            assert!(matches!(
                engine.parse::<String>(),
                Ok(value) if value == engine
            ));
        }
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

    #[test]
    fn status_header_names_every_column() {
        let header = status_header();
        for column in ["ID", "Engine", "Filename", "Size", "Status"] {
            assert!(header.contains(column));
        }
    }
}
