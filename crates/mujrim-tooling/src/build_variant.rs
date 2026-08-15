use std::fs;
use std::path::Path;

use crate::action::ToolAction;
use crate::process::run;
use mujrim_protocols::catalog::{adapter_binary_stem, host_packaging_arch};

const PORTABLE_BUILD_ENV: &[(&str, &str)] = &[("CARGO_BUILD_JOBS", "1")];

#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq)]
pub enum BuildVariant {
    Full,
    Akimbo,
    Stockfish,
    Reckless,
    #[value(alias = "native-v60")]
    V60,
    #[value(alias = "native-v60-embedded", alias = "v60-embedded")]
    V60Embedded,
    Viridithas,
    Obsidian,
    #[value(alias = "plentychess", alias = "plenty")]
    PlentyChess,
    Ateed,
    Benchmark,
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
                println!(
                    "build variants: full, akimbo, stockfish, reckless, v60, v60-embedded, viridithas, obsidian, plentychess, ateed, benchmark, embedded, minimal"
                );
                Ok(())
            }
            _ => {
                let args = variant_args(&self.variant);
                run("cargo", &args, PORTABLE_BUILD_ENV)?;
                snapshot_variant_dist_name(&self.variant)
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
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "xboard,book,nnue,simd,akimbo-nnue,embedded-networks",
        ],
        BuildVariant::Stockfish => vec![
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "xboard,book,nnue,simd,stockfish-nnue,embedded-networks",
        ],
        BuildVariant::Reckless => vec![
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "xboard,book,nnue,simd,reckless-nnue",
        ],
        BuildVariant::V60 => vec![
            "build",
            "--release",
            "-p",
            "mujrim-native-v60",
            "--features",
            "syzygy",
        ],
        BuildVariant::V60Embedded => vec![
            "build",
            "--release",
            "-p",
            "mujrim-native-v60",
            "--features",
            "syzygy,embedded-network",
        ],
        BuildVariant::Viridithas => vec![
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "xboard,book,nnue,simd,viridithas-nnue",
        ],
        BuildVariant::Obsidian => vec![
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "xboard,book,nnue,simd,obsidian-nnue",
        ],
        BuildVariant::PlentyChess => vec![
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "xboard,book,nnue,simd,plentychess-nnue",
        ],
        BuildVariant::Ateed => vec![
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "xboard,book,nnue,simd,ateed-nnue",
        ],
        BuildVariant::Benchmark => vec![
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "nnue,simd,reckless-nnue",
        ],
        BuildVariant::Embedded => vec![
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "xboard,book,nnue,simd,akimbo-nnue,stockfish-nnue,reckless-nnue,embedded-networks",
        ],
        BuildVariant::Minimal => vec![
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "nnue,simd",
        ],
        BuildVariant::List => Vec::new(),
    }
}

fn variant_dist_mapping(variant: &BuildVariant) -> Option<(&'static str, &'static str)> {
    match variant {
        BuildVariant::V60 | BuildVariant::V60Embedded => Some(("mujrim-v60", "mujrim-v60")),
        BuildVariant::Stockfish | BuildVariant::Embedded => Some(("mujrim", "mujrim-elite")),
        BuildVariant::Akimbo => Some(("mujrim", "mujrim-ak")),
        BuildVariant::Viridithas => Some(("mujrim", "mujrim-viri")),
        BuildVariant::Obsidian => Some(("mujrim", "mujrim-obs")),
        BuildVariant::PlentyChess => Some(("mujrim", "mujrim-plenty")),
        BuildVariant::Ateed => Some(("mujrim", "mujrim-ateed")),
        _ => None,
    }
}

fn snapshot_variant_dist_name(variant: &BuildVariant) -> Result<(), String> {
    let Some((source_stem, adapter_id)) = variant_dist_mapping(variant) else {
        return Ok(());
    };
    let arch = host_packaging_arch();
    let suffix = std::env::consts::EXE_SUFFIX;
    let source = Path::new("target")
        .join("release")
        .join(format!("{source_stem}{suffix}"));
    let destination_stem = adapter_binary_stem(adapter_id, &arch);
    let destination = Path::new("target")
        .join("release")
        .join(format!("{destination_stem}{suffix}"));
    fs::copy(&source, &destination).map_err(|error| {
        format!(
            "failed to snapshot {} as {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
    if adapter_id != source_stem {
        let alias = Path::new("target")
            .join("release")
            .join(format!("{adapter_id}{suffix}"));
        fs::copy(&source, &alias).map_err(|error| {
            format!(
                "failed to snapshot {} as {}: {error}",
                source.display(),
                alias.display()
            )
        })?;
    }
    Ok(())
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
        assert!(args.contains(&"mujrim"));
    }

    #[test]
    fn benchmark_variant_keeps_only_required_engine_features() {
        let args = variant_args(&BuildVariant::Benchmark);
        assert!(args.windows(2).any(|pair| pair == ["-p", "mujrim"]));
        let features = args
            .windows(2)
            .find_map(|pair| (pair[0] == "--features").then_some(pair[1]))
            .unwrap();
        assert_eq!(features, "nnue,simd,reckless-nnue");
        assert!(!features.contains("trainer"));
        assert!(!features.contains("gpu"));
        assert!(!features.contains("book"));
        assert!(!features.contains("xboard"));
    }

    #[test]
    fn optimized_variants_do_not_force_the_slower_allocator() {
        for variant in [
            BuildVariant::Akimbo,
            BuildVariant::Stockfish,
            BuildVariant::Reckless,
            BuildVariant::V60,
            BuildVariant::V60Embedded,
            BuildVariant::Benchmark,
            BuildVariant::Embedded,
            BuildVariant::Viridithas,
            BuildVariant::Obsidian,
            BuildVariant::PlentyChess,
            BuildVariant::Ateed,
        ] {
            assert!(
                !variant_args(&variant)
                    .iter()
                    .any(|arg| arg.contains("mimalloc"))
            );
        }
    }

    #[test]
    fn network_variants_build_only_the_engine_feature_stack() {
        for variant in [
            BuildVariant::Akimbo,
            BuildVariant::Stockfish,
            BuildVariant::Reckless,
        ] {
            let args = variant_args(&variant);
            assert!(args.windows(2).any(|pair| pair == ["-p", "mujrim"]));
            let features = args
                .windows(2)
                .find_map(|pair| (pair[0] == "--features").then_some(pair[1]))
                .unwrap();
            assert!(!features.contains("trainer"));
            assert!(!features.contains("gpu"));
            assert!(features.contains("book"));
            assert!(features.contains("xboard"));
        }
    }

    #[test]
    fn release_variants_do_not_lock_to_the_build_host_cpu() {
        assert_eq!(PORTABLE_BUILD_ENV, &[("CARGO_BUILD_JOBS", "1")]);
    }

    #[test]
    fn v60_targets_the_static_search_adapter() {
        assert_eq!(
            variant_args(&BuildVariant::V60),
            [
                "build",
                "--release",
                "-p",
                "mujrim-native-v60",
                "--features",
                "syzygy"
            ]
        );
    }

    #[test]
    fn embedded_variants_carry_their_network_payloads() {
        let engine = variant_args(&BuildVariant::Embedded);
        let engine_features = engine
            .windows(2)
            .find_map(|pair| (pair[0] == "--features").then_some(pair[1]))
            .unwrap();
        assert!(engine_features.contains("embedded-networks"));
        assert!(engine_features.contains("akimbo-nnue"));
        assert!(engine_features.contains("stockfish-nnue"));
        assert!(engine_features.contains("reckless-nnue"));

        assert_eq!(
            variant_args(&BuildVariant::V60Embedded),
            [
                "build",
                "--release",
                "-p",
                "mujrim-native-v60",
                "--features",
                "syzygy,embedded-network"
            ]
        );
    }

    #[test]
    fn obsidian_variant_enables_only_obsidian_nnue() {
        let features = variant_args(&BuildVariant::Obsidian)
            .windows(2)
            .find_map(|pair| (pair[0] == "--features").then_some(pair[1]))
            .unwrap();
        assert_eq!(features, "xboard,book,nnue,simd,obsidian-nnue");
        assert!(!features.contains("stockfish-nnue"));
        assert!(!features.contains("reckless-nnue"));
        assert!(!features.contains("akimbo-nnue"));
    }

    #[test]
    fn plentychess_and_ateed_variants_do_not_share_a_foreign_net() {
        let plenty = variant_args(&BuildVariant::PlentyChess)
            .windows(2)
            .find_map(|pair| (pair[0] == "--features").then_some(pair[1]))
            .unwrap();
        assert_eq!(plenty, "xboard,book,nnue,simd,plentychess-nnue");
        assert!(!plenty.contains("reckless-nnue"));
        assert!(!plenty.contains("obsidian-nnue"));

        let ateed = variant_args(&BuildVariant::Ateed)
            .windows(2)
            .find_map(|pair| (pair[0] == "--features").then_some(pair[1]))
            .unwrap();
        assert_eq!(ateed, "xboard,book,nnue,simd,ateed-nnue");
        assert!(!ateed.contains("reckless-nnue"));
        assert!(!ateed.contains("plentychess-nnue"));
    }

    #[test]
    fn dist_mapping_uses_product_engine_names() {
        assert_eq!(
            variant_dist_mapping(&BuildVariant::V60),
            Some(("mujrim-v60", "mujrim-v60"))
        );
        assert_eq!(
            variant_dist_mapping(&BuildVariant::Stockfish),
            Some(("mujrim", "mujrim-elite"))
        );
        assert_eq!(
            variant_dist_mapping(&BuildVariant::Akimbo),
            Some(("mujrim", "mujrim-ak"))
        );
        assert_eq!(
            variant_dist_mapping(&BuildVariant::Viridithas),
            Some(("mujrim", "mujrim-viri"))
        );
        assert_eq!(
            variant_dist_mapping(&BuildVariant::Obsidian),
            Some(("mujrim", "mujrim-obs"))
        );
        assert_eq!(
            variant_dist_mapping(&BuildVariant::PlentyChess),
            Some(("mujrim", "mujrim-plenty"))
        );
        assert_eq!(
            variant_dist_mapping(&BuildVariant::Ateed),
            Some(("mujrim", "mujrim-ateed"))
        );
        assert_eq!(adapter_binary_stem("mujrim-v10", "x86_64"), "mujrim-elite");
        assert_eq!(adapter_binary_stem("mujrim-akimbo", "aarch64"), "mujrim-ak");
        assert!(!adapter_binary_stem("mujrim-v60", "x86_64").contains("native"));
    }

    #[test]
    fn product_embedded_variants_do_not_unify_every_net() {
        let elite = variant_args(&BuildVariant::Stockfish)
            .windows(2)
            .find_map(|pair| (pair[0] == "--features").then_some(pair[1]))
            .unwrap();
        assert!(elite.contains("stockfish-nnue"));
        assert!(elite.contains("embedded-networks"));
        assert!(!elite.contains("akimbo-nnue"));
        assert!(!elite.contains("reckless-nnue"));

        let ak = variant_args(&BuildVariant::Akimbo)
            .windows(2)
            .find_map(|pair| (pair[0] == "--features").then_some(pair[1]))
            .unwrap();
        assert!(ak.contains("akimbo-nnue"));
        assert!(ak.contains("embedded-networks"));
        assert!(!ak.contains("stockfish-nnue"));
        assert!(!ak.contains("reckless-nnue"));
    }
}
