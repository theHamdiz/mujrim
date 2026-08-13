mod action;
mod agent;
mod agent_tools;
mod build_variant;
#[cfg(test)]
mod github_ci;
mod install;
mod nnue;
mod process;
mod release;
mod uninstall;

use clap::{Parser, Subcommand};

use crate::action::ToolAction;
use crate::agent::{AgentAction, AgentCommand};
use crate::build_variant::{BuildVariant, BuildVariantAction};
use crate::install::InstallAction;
use crate::nnue::{NnueAction, NnueCommand};
use crate::release::{ReleaseAction, ReleaseTarget};
use crate::uninstall::UninstallAction;

#[derive(Parser, Debug)]
#[command(
    name = "mujrim-tooling",
    about = "Developer tooling for Mujrim recipes",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Cross-platform release builds.
    Release {
        #[arg(value_enum, default_value_t = ReleaseTarget::Native)]
        target: ReleaseTarget,
    },
    /// Build one NNUE variant.
    BuildVariant {
        #[arg(value_enum, default_value_t = BuildVariant::Full)]
        variant: BuildVariant,
    },
    /// NNUE network management.
    Nnue {
        #[command(subcommand)]
        command: NnueCommand,
    },
    /// Structured tools for AI agents.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Install built binaries locally.
    Install,
    /// Remove local Mujrim installation artifacts.
    Uninstall,
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Release { target } => ReleaseAction { target }.run(),
        Command::BuildVariant { variant } => BuildVariantAction { variant }.run(),
        Command::Nnue { command } => NnueAction { command }.run(),
        Command::Agent { command } => AgentAction { command }.run(),
        Command::Install => InstallAction.run(),
        Command::Uninstall => UninstallAction.run(),
    };

    if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
