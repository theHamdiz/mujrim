use crate::action::ToolAction;
use crate::process::run;

#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq)]
pub enum BuildVariant {
    Full,
    Akimbo,
    Stockfish,
    Embedded,
    Minimal,
    List,
}

#[derive(Debug)]
pub struct BuildVariantAction {
    pub variant: BuildVariant,
}

impl ToolAction for BuildVariantAction {
    fn run(&self) -> Result<(), String> {
        match self.variant {
            BuildVariant::List => {
                println!("build variants: full, akimbo, stockfish, embedded, minimal");
                Ok(())
            }
            _ => {
                let args = variant_args(&self.variant);
                run("cargo", &args, &[("RUSTFLAGS", "-C target-cpu=native")])
            }
        }
    }
}

fn variant_args(variant: &BuildVariant) -> Vec<&'static str> {
    match variant {
        BuildVariant::Full => vec!["build", "--release"],
        BuildVariant::Akimbo => vec![
            "build",
            "--release",
            "--no-default-features",
            "--features",
            "mimalloc,trainer,gpu,xboard,book,nnue,simd,akimbo-nnue",
        ],
        BuildVariant::Stockfish => vec![
            "build",
            "--release",
            "--no-default-features",
            "--features",
            "mimalloc,trainer,gpu,xboard,book,nnue,simd,stockfish-nnue",
        ],
        BuildVariant::Embedded => vec![
            "build",
            "--release",
            "--no-default-features",
            "--features",
            "mimalloc,trainer,gpu,xboard,book,nnue,simd",
        ],
        BuildVariant::Minimal => vec![
            "build",
            "--release",
            "-p",
            "kishmat",
            "--no-default-features",
            "--features",
            "nnue,simd",
        ],
        BuildVariant::List => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_list_has_no_args() {
        assert!(variant_args(&BuildVariant::List).is_empty());
    }

    #[test]
    fn minimal_targets_main_binary() {
        let args = variant_args(&BuildVariant::Minimal);
        assert!(args.contains(&"-p"));
        assert!(args.contains(&"kishmat"));
    }
}
