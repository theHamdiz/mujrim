//! Runtime discovery for bundled, architecture-specific chess engines.

use std::path::{Path, PathBuf};

pub const BUNDLED_ENGINES: &[(&str, &str)] = &[
    ("mujrim", "Mujrim Elite"),
    ("mujrim-v60", "Mujrim v60"),
    ("mujrim-v10", "Mujrim v10"),
    ("mujrim-akimbo", "Mujrim Akimbo"),
    ("stockfish", "Stockfish"),
    ("plentychess", "PlentyChess"),
    ("obsidian", "Obsidian"),
    ("reckless", "Reckless"),
    ("akimbo", "Akimbo"),
    ("ethereal", "Ethereal"),
];

/// In-process classical evaluator + HCE search stack (not a separate binary).
pub const MUJRIM_HCE_DISPLAY_NAME: &str = "Mujrim HCE";

/// Mujrim adapter ids that ship as `mujrim-*-{arch}` under `bin/{os-arch}/`.
pub const ARCH_SUFFIXED_ADAPTERS: &[&str] = &["mujrim-v60", "mujrim-v10", "mujrim-akimbo"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeCompatibility {
    Native,
    Emulated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchLimitSupport {
    pub fixed_nodes: bool,
    pub move_time: bool,
}

impl SearchLimitSupport {
    pub const STANDARD: Self = Self {
        fixed_nodes: true,
        move_time: true,
    };

    pub const DEPTH_ONLY: Self = Self {
        fixed_nodes: false,
        move_time: false,
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineCandidate {
    pub path: PathBuf,
    pub target_directory: String,
    pub compatibility: RuntimeCompatibility,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredEngine {
    pub id: &'static str,
    pub display_name: &'static str,
    pub path: PathBuf,
    pub target_directory: String,
    pub compatibility: RuntimeCompatibility,
    pub search_limits: SearchLimitSupport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimePlatform {
    pub os: &'static str,
    pub architecture: &'static str,
}

impl RuntimePlatform {
    pub const fn current() -> Self {
        Self {
            os: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
        }
    }

    pub fn directory_name(self) -> String {
        format!("{}-{}", self.os, self.architecture)
    }
}

/// True for Mujrim adapter binaries that include an arch token in dist names.
pub fn is_arch_suffixed_adapter(engine_id: &str) -> bool {
    ARCH_SUFFIXED_ADAPTERS.contains(&engine_id)
}

/// Strip the OS prefix from a runtime target directory (`windows-x86_64-avx2` → `x86_64-avx2`).
pub fn arch_token_from_target_directory(target_directory: &str) -> String {
    let stripped = target_directory
        .strip_prefix("windows-")
        .or_else(|| target_directory.strip_prefix("linux-"))
        .or_else(|| target_directory.strip_prefix("macos-"))
        .or_else(|| target_directory.strip_prefix("darwin-"))
        .or_else(|| target_directory.strip_prefix("android-"))
        .unwrap_or(target_directory);
    normalize_arch_token(stripped)
}

/// Derive an arch token from a rustc target triple (`x86_64-pc-windows-msvc` → `x86_64`).
pub fn arch_token_from_rustc_target(triple: &str) -> String {
    let arch = triple.split('-').next().unwrap_or(triple);
    normalize_arch_token(arch)
}

/// Derive an arch token from the current [`RuntimePlatform`].
pub fn arch_token_from_platform(platform: RuntimePlatform) -> String {
    arch_token_from_target_directory(&platform.directory_name())
}

/// Packaging arch token, optionally including an ISA flavor (`avx2` → `x86_64-avx2`).
pub fn packaging_arch_token(platform: RuntimePlatform, isa_flavor: Option<&str>) -> String {
    let base = arch_token_from_platform(platform);
    match isa_flavor {
        Some(flavor) if !flavor.is_empty() && !base.ends_with(flavor) => {
            format!("{base}-{flavor}")
        }
        _ => base,
    }
}

/// Preferred host packaging arch, using `x86_64-avx2` on AVX2 Windows hosts.
pub fn host_packaging_arch() -> String {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    if std::arch::is_x86_feature_detected!("avx2") {
        return "x86_64-avx2".to_owned();
    }
    arch_token_from_platform(RuntimePlatform::current())
}

/// Dist / display stem for a Mujrim adapter: `mujrim-v60-x86_64-avx2`.
pub fn adapter_binary_stem(adapter_id: &str, arch: &str) -> String {
    format!("{adapter_id}-{arch}")
}

fn normalize_arch_token(token: &str) -> String {
    if token == "arm64" {
        return "aarch64".to_owned();
    }
    if let Some(rest) = token.strip_prefix("arm64-") {
        return format!("aarch64-{rest}");
    }
    token.to_owned()
}

fn with_exe_suffix(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

fn executable_filename(engine_id: &str) -> String {
    with_exe_suffix(engine_id)
}

fn packaged_executable_filename(engine_id: &str, target_directory: &str) -> String {
    if is_arch_suffixed_adapter(engine_id) {
        let arch = arch_token_from_target_directory(target_directory);
        with_exe_suffix(&adapter_binary_stem(engine_id, &arch))
    } else {
        executable_filename(engine_id)
    }
}

fn package_directory(engine_id: &str) -> &str {
    match engine_id {
        "mujrim-v60" | "mujrim-v10" | "mujrim-akimbo" => "mujrim",
        other => other,
    }
}

fn search_limit_support(engine_id: &str) -> SearchLimitSupport {
    if engine_id == "ethereal" {
        SearchLimitSupport::DEPTH_ONLY
    } else {
        SearchLimitSupport::STANDARD
    }
}

fn runtime_targets() -> Vec<(String, RuntimeCompatibility)> {
    let current = RuntimePlatform::current();
    let mut targets = Vec::with_capacity(4);

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    if std::arch::is_x86_feature_detected!("avx2") {
        targets.push((
            "windows-x86_64-avx2".to_owned(),
            RuntimeCompatibility::Native,
        ));
    }

    targets.push((current.directory_name(), RuntimeCompatibility::Native));

    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        // Host-arch only: never auto-select emulated x86_64 binaries on Arm64.
        targets.push(("windows-arm64".to_owned(), RuntimeCompatibility::Native));
    }

    targets
}

fn engine_roots(executable: &Path, current_dir: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::with_capacity(4);
    if let Some(executable_dir) = executable.parent() {
        roots.push(executable_dir.join("engines"));
        if let Some(parent) = executable_dir.parent() {
            roots.push(parent.join("engines"));
        }
        if let Some(packaged_root) = executable_dir
            .ancestors()
            .find(|path| path.file_name().is_some_and(|name| name == "engines"))
        {
            roots.push(packaged_root.to_path_buf());
        }
    }
    roots.push(current_dir.join("dist").join("engines"));
    roots.push(current_dir.join("engines"));
    roots
}

fn push_candidate(candidates: &mut Vec<EngineCandidate>, candidate: EngineCandidate) {
    if !candidates
        .iter()
        .any(|existing| existing.path == candidate.path)
    {
        candidates.push(candidate);
    }
}

/// Candidate locations in priority order. An explicit path always wins,
/// followed by host-native packaged builds (never emulated ISA folders).
pub fn engine_candidate_details(
    engine_id: &str,
    executable: &Path,
    current_dir: &Path,
    explicit: Option<&Path>,
) -> Vec<EngineCandidate> {
    let flat_filename = executable_filename(engine_id);
    let mut candidates = Vec::with_capacity(20);
    if let Some(path) = explicit {
        push_candidate(
            &mut candidates,
            EngineCandidate {
                path: path.to_path_buf(),
                target_directory: "explicit".to_owned(),
                compatibility: RuntimeCompatibility::Native,
            },
        );
    }
    if let Some(executable_dir) = executable.parent() {
        push_candidate(
            &mut candidates,
            EngineCandidate {
                path: executable_dir.join(&flat_filename),
                target_directory: "adjacent".to_owned(),
                compatibility: RuntimeCompatibility::Native,
            },
        );
        if is_arch_suffixed_adapter(engine_id) {
            let host_arch = host_packaging_arch();
            push_candidate(
                &mut candidates,
                EngineCandidate {
                    path: executable_dir
                        .join(with_exe_suffix(&adapter_binary_stem(engine_id, &host_arch))),
                    target_directory: "adjacent".to_owned(),
                    compatibility: RuntimeCompatibility::Native,
                },
            );
        }
    }
    for root in engine_roots(executable, current_dir) {
        for (target_directory, compatibility) in runtime_targets() {
            let filename = packaged_executable_filename(engine_id, &target_directory);
            let path = root
                .join(package_directory(engine_id))
                .join("bin")
                .join(&target_directory)
                .join(filename);
            push_candidate(
                &mut candidates,
                EngineCandidate {
                    path,
                    target_directory,
                    compatibility,
                },
            );
        }
    }
    candidates
}

pub fn engine_candidates(
    engine_id: &str,
    executable: &Path,
    current_dir: &Path,
    explicit: Option<&Path>,
) -> Vec<PathBuf> {
    engine_candidate_details(engine_id, executable, current_dir, explicit)
        .into_iter()
        .map(|candidate| candidate.path)
        .collect()
}

pub fn discover_engine_details(
    engine_id: &str,
    executable: &Path,
    current_dir: &Path,
    explicit: Option<&Path>,
) -> Result<EngineCandidate, String> {
    let candidates = engine_candidate_details(engine_id, executable, current_dir, explicit);
    candidates
        .iter()
        .find(|candidate| {
            candidate.compatibility == RuntimeCompatibility::Native
                && candidate.path.is_file()
                && crate::binary_arch::is_host_native_binary(&candidate.path)
        })
        .cloned()
        .ok_or_else(|| {
            let searched = candidates
                .iter()
                .map(|candidate| candidate.path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "could not find host-native {engine_id} for {} (searched: {searched})",
                RuntimePlatform::current().directory_name()
            )
        })
}

pub fn discover_engine(
    engine_id: &str,
    executable: &Path,
    current_dir: &Path,
    explicit: Option<&Path>,
) -> Result<PathBuf, String> {
    discover_engine_details(engine_id, executable, current_dir, explicit)
        .map(|candidate| candidate.path)
}

pub fn discover_bundled_engines(executable: &Path, current_dir: &Path) -> Vec<DiscoveredEngine> {
    BUNDLED_ENGINES
        .iter()
        .filter_map(|&(id, display_name)| {
            discover_engine_details(id, executable, current_dir, None)
                .ok()
                .map(|candidate| DiscoveredEngine {
                    id,
                    display_name,
                    path: candidate.path,
                    target_directory: candidate.target_directory,
                    compatibility: candidate.compatibility,
                    search_limits: search_limit_support(id),
                })
        })
        .collect()
}

pub fn discover_bundled_engines_from_environment() -> Result<Vec<DiscoveredEngine>, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to locate current executable: {error}"))?;
    let current_dir = std::env::current_dir()
        .map_err(|error| format!("failed to locate current directory: {error}"))?;
    Ok(discover_bundled_engines(&executable, &current_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Use forward slashes so Path parent/join semantics work on Linux/macOS CI hosts too.
    #[test]
    fn explicit_engine_path_has_priority() {
        let candidates = engine_candidates(
            "reckless",
            Path::new("C:/Program Files/Mujrim/mujrim.exe"),
            Path::new("D:/src/mujrim"),
            Some(Path::new("E:/engines/reckless-custom.exe")),
        );
        assert_eq!(
            candidates.first().unwrap(),
            Path::new("E:/engines/reckless-custom.exe")
        );
    }

    #[test]
    fn flat_installer_layout_finds_adjacent_engine() {
        let candidates = engine_candidate_details(
            "mujrim-v60",
            Path::new("C:/Program Files/Mujrim/bin/mujrim.exe"),
            Path::new("D:/unrelated"),
            None,
        );
        assert_eq!(
            candidates.first().unwrap(),
            &EngineCandidate {
                path: Path::new("C:/Program Files/Mujrim/bin")
                    .join(executable_filename("mujrim-v60")),
                target_directory: "adjacent".to_owned(),
                compatibility: RuntimeCompatibility::Native,
            }
        );
    }

    #[test]
    fn ethereal_uses_its_verified_depth_only_control() {
        assert_eq!(
            search_limit_support("ethereal"),
            SearchLimitSupport::DEPTH_ONLY
        );
        assert_eq!(
            search_limit_support("stockfish"),
            SearchLimitSupport::STANDARD
        );
    }

    #[test]
    fn packaged_candidates_include_runtime_architecture() {
        let candidates = engine_candidates(
            "reckless",
            Path::new("C:/Mujrim/mujrim.exe"),
            Path::new("D:/src/mujrim"),
            None,
        );
        let expected = Path::new("C:/Mujrim")
            .join("engines")
            .join("reckless")
            .join("bin")
            .join(RuntimePlatform::current().directory_name())
            .join(executable_filename("reckless"));
        assert!(
            candidates.contains(&expected),
            "missing {expected:?} in {candidates:?}"
        );
    }

    #[test]
    fn mujrim_adapters_use_arch_suffixed_packaged_filenames() {
        assert_eq!(BUNDLED_ENGINES[1], ("mujrim-v60", "Mujrim v60"));
        assert_eq!(BUNDLED_ENGINES[2], ("mujrim-v10", "Mujrim v10"));
        assert_eq!(BUNDLED_ENGINES[3], ("mujrim-akimbo", "Mujrim Akimbo"));

        let target = RuntimePlatform::current().directory_name();
        let arch = arch_token_from_target_directory(&target);
        for adapter in ["mujrim-v60", "mujrim-v10", "mujrim-akimbo"] {
            let candidates = engine_candidates(
                adapter,
                Path::new("C:/Mujrim/mujrim-ui.exe"),
                Path::new("D:/src/mujrim"),
                None,
            );
            let expected = Path::new("C:/Mujrim")
                .join("engines")
                .join("mujrim")
                .join("bin")
                .join(&target)
                .join(with_exe_suffix(&adapter_binary_stem(adapter, &arch)));
            assert!(
                candidates.contains(&expected),
                "missing {expected:?} in {candidates:?}"
            );
        }
    }

    #[test]
    fn upstream_comparison_engines_remain_unsuffixed() {
        for upstream in ["stockfish", "akimbo", "reckless"] {
            assert_eq!(
                packaged_executable_filename(upstream, "windows-x86_64-avx2"),
                executable_filename(upstream)
            );
        }
    }

    #[test]
    fn arch_token_strips_os_prefix_and_normalizes_arm64() {
        assert_eq!(
            arch_token_from_target_directory("windows-x86_64-avx2"),
            "x86_64-avx2"
        );
        assert_eq!(arch_token_from_target_directory("linux-x86_64"), "x86_64");
        assert_eq!(
            arch_token_from_target_directory("darwin-aarch64"),
            "aarch64"
        );
        assert_eq!(arch_token_from_target_directory("windows-arm64"), "aarch64");
        assert_eq!(
            arch_token_from_target_directory("windows-arm64-neon"),
            "aarch64-neon"
        );
    }

    #[test]
    fn arch_token_from_rustc_target_uses_triple_arch() {
        assert_eq!(
            arch_token_from_rustc_target("x86_64-pc-windows-msvc"),
            "x86_64"
        );
        assert_eq!(
            arch_token_from_rustc_target("aarch64-unknown-linux-gnu"),
            "aarch64"
        );
        assert_eq!(
            arch_token_from_rustc_target("aarch64-apple-darwin"),
            "aarch64"
        );
    }

    #[test]
    fn packaging_arch_token_appends_isa_flavor() {
        let platform = RuntimePlatform {
            os: "windows",
            architecture: "x86_64",
        };
        assert_eq!(packaging_arch_token(platform, Some("avx2")), "x86_64-avx2");
        assert_eq!(packaging_arch_token(platform, None), "x86_64");
    }

    #[test]
    fn adapter_binary_stem_matches_dist_scheme() {
        assert_eq!(
            adapter_binary_stem("mujrim-v60", "x86_64-avx2"),
            "mujrim-v60-x86_64-avx2"
        );
        assert_eq!(
            adapter_binary_stem("mujrim-v10", "aarch64"),
            "mujrim-v10-aarch64"
        );
        assert_eq!(
            adapter_binary_stem("mujrim-akimbo", "x86_64"),
            "mujrim-akimbo-x86_64"
        );
    }

    #[test]
    fn filename_mapping_for_packaged_adapters() {
        assert_eq!(
            packaged_executable_filename("mujrim-v60", "windows-x86_64-avx2"),
            with_exe_suffix("mujrim-v60-x86_64-avx2")
        );
        assert_eq!(
            packaged_executable_filename("mujrim-v10", "linux-aarch64"),
            with_exe_suffix("mujrim-v10-aarch64")
        );
        assert_eq!(
            packaged_executable_filename("mujrim-akimbo", "darwin-x86_64"),
            with_exe_suffix("mujrim-akimbo-x86_64")
        );
    }

    #[test]
    fn engine_inside_packaged_tree_finds_sibling_backends() {
        let candidates = engine_candidates(
            "stockfish",
            Path::new("C:/Mujrim/dist/engines/mujrim/bin/windows-aarch64/mujrim-elite.exe"),
            Path::new("D:/unrelated"),
            None,
        );
        let expected = Path::new("C:/Mujrim/dist/engines")
            .join("stockfish")
            .join("bin")
            .join(RuntimePlatform::current().directory_name())
            .join(executable_filename("stockfish"));
        assert!(
            candidates.iter().any(|candidate| candidate == &expected),
            "missing {expected:?} in {candidates:?}"
        );
    }

    #[test]
    fn catalog_keeps_upstream_engines_for_native_comparison() {
        let ids: Vec<&str> = BUNDLED_ENGINES.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&"stockfish"));
        assert!(ids.contains(&"akimbo"));
        assert!(ids.contains(&"reckless"));
        assert!(ids.contains(&"mujrim-v60"));
        assert!(ids.contains(&"mujrim-v10"));
        assert!(ids.contains(&"mujrim-akimbo"));
        assert!(!ids.iter().any(|id| id.contains("native")));
        assert!(
            !BUNDLED_ENGINES
                .iter()
                .any(|(_, display)| display.to_ascii_lowercase().contains("native"))
        );
        assert_eq!(MUJRIM_HCE_DISPLAY_NAME, "Mujrim HCE");
    }

    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    #[test]
    fn windows_arm64_auto_detect_skips_emulated_x64_folders() {
        let candidates = engine_candidate_details(
            "obsidian",
            Path::new("C:/Mujrim/mujrim.exe"),
            Path::new("D:/src/mujrim"),
            None,
        );
        assert!(candidates.iter().all(|candidate| {
            candidate.compatibility == RuntimeCompatibility::Native
                && !candidate.target_directory.contains("x86_64")
        }));
    }
}
