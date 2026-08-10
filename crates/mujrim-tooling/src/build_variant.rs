use crate::action::ToolAction;
use crate::process::run;

const PORTABLE_BUILD_ENV: &[(&str, &str)] = &[("CARGO_BUILD_JOBS", "1")];

#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq)]
pub enum BuildVariant {
    Full,
    Akimbo,
    Stockfish,
    Reckless,
    NativeV60,
    NativeV60Embedded,
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
                    "build variants: full, akimbo, stockfish, reckless, native-v60, native-v60-embedded, benchmark, embedded, minimal"
                );
                Ok(())
            }
            _ => {
                let args = variant_args(&self.variant);
                run("cargo", &args, PORTABLE_BUILD_ENV)
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
            "xboard,book,nnue,simd,akimbo-nnue",
        ],
        BuildVariant::Stockfish => vec![
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "xboard,book,nnue,simd,stockfish-nnue",
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
        BuildVariant::NativeV60 => vec![
            "build",
            "--release",
            "-p",
            "mujrim-native-v60",
            "--features",
            "syzygy",
        ],
        BuildVariant::NativeV60Embedded => vec![
            "build",
            "--release",
            "-p",
            "mujrim-native-v60",
            "--features",
            "syzygy,embedded-network",
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
            BuildVariant::NativeV60,
            BuildVariant::NativeV60Embedded,
            BuildVariant::Benchmark,
            BuildVariant::Embedded,
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
    fn native_v60_targets_the_static_search_adapter() {
        assert_eq!(
            variant_args(&BuildVariant::NativeV60),
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
            variant_args(&BuildVariant::NativeV60Embedded),
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
}
