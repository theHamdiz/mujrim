//! Runtime discovery for bundled, architecture-specific chess engines.

use std::path::{Path, PathBuf};

/// Product engines shipped beside the UI under `engines/mujrim/bin/<os-arch>/`:
/// - `mujrim-elite` — Stockfish NNUE embedded
/// - `mujrim-external` — loads/discovers NNUE at runtime
/// - `mujrim-v60` — Reckless NNUE embedded
/// - `mujrim-ak` — Akimbo NNUE embedded
/// - `mujrim-viri` — Viridithas search + runtime net
/// - `mujrim-obs` — Obsidian search + runtime net
/// - `mujrim-plenty` — PlentyChess search profile
/// - `mujrim-lc0` — official Lc0 passthrough with GPU/CPU selection
pub const BUNDLED_ENGINES: &[(&str, &str)] = &[
    ("mujrim-elite", "Mujrim Elite"),
    ("mujrim-external", "Mujrim External"),
    ("mujrim-v60", "Mujrim v60"),
    ("mujrim-ak", "Mujrim Akimbo"),
    ("mujrim-viri", "Mujrim Viridithas"),
    ("mujrim-obs", "Mujrim Obsidian"),
    ("mujrim-plenty", "Mujrim PlentyChess"),
    ("mujrim-lc0", "Mujrim Lc0"),
    ("stockfish", "Stockfish"),
    ("plentychess", "PlentyChess"),
    ("obsidian", "Obsidian"),
    ("reckless", "Reckless"),
    ("akimbo", "Akimbo"),
    ("ethereal", "Ethereal"),
    ("lc0", "Lc0"),
    ("viridithas", "Viridithas"),
    ("hobbes", "Hobbes"),
    ("integral", "Integral"),
    ("velvet", "Velvet"),
];

/// In-process classical evaluator + HCE search stack (not a separate binary).
pub const MUJRIM_HCE_DISPLAY_NAME: &str = "Mujrim HCE";

/// Legacy / alternate ids that still resolve to a product binary.
const ENGINE_ID_ALIASES: &[(&str, &str)] = &[
    ("mujrim", "mujrim-elite"),
    ("mujrim-embedded", "mujrim-elite"),
    ("mujrim-v10", "mujrim-elite"),
    ("mujrim-akimbo", "mujrim-ak"),
    ("mujrim-viridithas", "mujrim-viri"),
    ("mujrim-obsidian", "mujrim-obs"),
    ("mujrim-plentychess", "mujrim-plenty"),
    ("mujrim-leela", "mujrim-lc0"),
];

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

/// Canonical product id for discovery (`mujrim` → `mujrim-elite`).
pub fn canonical_engine_id(engine_id: &str) -> &str {
    ENGINE_ID_ALIASES
        .iter()
        .find(|(alias, _)| *alias == engine_id)
        .map(|(_, canonical)| *canonical)
        .unwrap_or(engine_id)
}

/// True for Mujrim adapter binaries that historically included an arch token.
/// Product dist names are unsuffixed (`mujrim-v60.exe` inside `bin/<os-arch>/`).
pub fn is_arch_suffixed_adapter(engine_id: &str) -> bool {
    matches!(
        canonical_engine_id(engine_id),
        "mujrim-v60"
            | "mujrim-ak"
            | "mujrim-elite"
            | "mujrim-viri"
            | "mujrim-obs"
            | "mujrim-plenty"
            | "mujrim-lc0"
    )
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

/// Dist stem for a Mujrim product binary. Product names are fixed; arch lives in the folder.
pub fn adapter_binary_stem(adapter_id: &str, _arch: &str) -> String {
    canonical_engine_id(adapter_id).to_owned()
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
    with_exe_suffix(canonical_engine_id(engine_id))
}

fn packaged_executable_filename(engine_id: &str, _target_directory: &str) -> String {
    executable_filename(engine_id)
}

fn package_directory(engine_id: &str) -> &str {
    match canonical_engine_id(engine_id) {
        "mujrim-elite" | "mujrim-external" | "mujrim-v60" | "mujrim-ak" | "mujrim-viri"
        | "mujrim-obs" | "mujrim-plenty" | "mujrim-lc0" => "mujrim",
        other => other,
    }
}

fn search_limit_support(engine_id: &str) -> SearchLimitSupport {
    if canonical_engine_id(engine_id) == "ethereal" || engine_id == "ethereal" {
        SearchLimitSupport::DEPTH_ONLY
    } else {
        SearchLimitSupport::STANDARD
    }
}

fn runtime_targets() -> Vec<(String, RuntimeCompatibility)> {
    let current = RuntimePlatform::current();
    let mut targets = Vec::with_capacity(6);

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
        targets.push(("windows-arm64".to_owned(), RuntimeCompatibility::Native));
        // Prism/WoA can run x64 tournament engines; prefer native ARM builds first.
        targets.push((
            "windows-x86_64-avx2".to_owned(),
            RuntimeCompatibility::Emulated,
        ));
        targets.push(("windows-x86_64".to_owned(), RuntimeCompatibility::Emulated));
    }

    targets
}

fn push_unique_root(roots: &mut Vec<PathBuf>, root: PathBuf) {
    if !roots.iter().any(|existing| existing == &root) {
        roots.push(root);
    }
}

/// Engine roots searched for packaged binaries.
///
/// Order: `<exe_dir>/engines`, then a packaged `engines/` ancestor when the
/// executable itself lives under `engines/<id>/bin/<arch>/`, then
/// `<cwd>/engines` so `cargo run` from the repo finds the vendored tree.
/// Parent folders of the UI (for example `C:/Mujrim/engines` when the binary
/// is in `C:/Mujrim/windows-aarch64/`) are not walked — those duplicated
/// arch alias copies.
pub fn engine_search_roots(executable: &Path, current_dir: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::with_capacity(3);
    if let Some(executable_dir) = executable.parent() {
        push_unique_root(&mut roots, executable_dir.join("engines"));
        if let Some(packaged_root) = executable_dir
            .ancestors()
            .find(|path| path.file_name().is_some_and(|name| name == "engines"))
        {
            push_unique_root(&mut roots, packaged_root.to_path_buf());
        }
    }
    push_unique_root(&mut roots, current_dir.join("engines"));
    push_unique_root(
        &mut roots,
        current_dir
            .join("dist")
            .join(RuntimePlatform::current().directory_name())
            .join("engines"),
    );
    roots
}

/// Local engines directory beside an executable (`<exe_dir>/engines`), if any.
pub fn local_engines_root(executable: &Path) -> Option<PathBuf> {
    executable.parent().map(|dir| dir.join("engines"))
}

fn push_candidate(candidates: &mut Vec<EngineCandidate>, candidate: EngineCandidate) {
    if !candidates
        .iter()
        .any(|existing| existing.path == candidate.path)
    {
        candidates.push(candidate);
    }
}

fn legacy_filename_aliases(engine_id: &str) -> Vec<String> {
    match canonical_engine_id(engine_id) {
        "mujrim-elite" => vec![
            with_exe_suffix("mujrim"),
            with_exe_suffix("mujrim-embedded"),
            with_exe_suffix("mujrim-v10"),
        ],
        "mujrim-ak" => vec![
            with_exe_suffix("mujrim-akimbo"),
            with_exe_suffix("mujrim-akimbo-external"),
        ],
        "mujrim-v60" => vec![
            with_exe_suffix("mujrim-v60-embedded"),
            with_exe_suffix("mujrim-v60-external"),
        ],
        "mujrim-external" => vec![with_exe_suffix("mujrim-external")],
        _ => Vec::new(),
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
    let product_id = canonical_engine_id(engine_id);
    let flat_filename = executable_filename(product_id);
    let mut candidates = Vec::with_capacity(24);
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
        for alias in legacy_filename_aliases(product_id) {
            push_candidate(
                &mut candidates,
                EngineCandidate {
                    path: executable_dir.join(alias),
                    target_directory: "adjacent".to_owned(),
                    compatibility: RuntimeCompatibility::Native,
                },
            );
        }
    }
    for root in engine_search_roots(executable, current_dir) {
        for (target_directory, compatibility) in runtime_targets() {
            let filename = packaged_executable_filename(product_id, &target_directory);
            let path = root
                .join(package_directory(product_id))
                .join("bin")
                .join(&target_directory)
                .join(filename);
            push_candidate(
                &mut candidates,
                EngineCandidate {
                    path,
                    target_directory: target_directory.clone(),
                    compatibility,
                },
            );
            for alias in legacy_filename_aliases(product_id) {
                push_candidate(
                    &mut candidates,
                    EngineCandidate {
                        path: root
                            .join(package_directory(product_id))
                            .join("bin")
                            .join(&target_directory)
                            .join(alias),
                        target_directory: target_directory.clone(),
                        compatibility,
                    },
                );
            }
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
    let native = candidates.iter().find(|candidate| {
        candidate.compatibility == RuntimeCompatibility::Native
            && candidate.path.is_file()
            && crate::binary_arch::is_host_native_binary(&candidate.path)
    });
    let emulated = candidates.iter().find(|candidate| {
        candidate.compatibility == RuntimeCompatibility::Emulated && candidate.path.is_file()
    });
    native.or(emulated).cloned().ok_or_else(|| {
        let searched = candidates
            .iter()
            .map(|candidate| candidate.path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "could not find {} for {} (searched: {searched})",
            canonical_engine_id(engine_id),
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
    fn mujrim_product_binaries_use_fixed_names_in_arch_folders() {
        assert_eq!(BUNDLED_ENGINES[0], ("mujrim-elite", "Mujrim Elite"));
        assert_eq!(BUNDLED_ENGINES[1], ("mujrim-external", "Mujrim External"));
        assert_eq!(BUNDLED_ENGINES[2], ("mujrim-v60", "Mujrim v60"));
        assert_eq!(BUNDLED_ENGINES[3], ("mujrim-ak", "Mujrim Akimbo"));
        assert_eq!(BUNDLED_ENGINES[4], ("mujrim-viri", "Mujrim Viridithas"));
        assert_eq!(BUNDLED_ENGINES[5], ("mujrim-obs", "Mujrim Obsidian"));

        let target = RuntimePlatform::current().directory_name();
        for product in [
            "mujrim-elite",
            "mujrim-external",
            "mujrim-v60",
            "mujrim-ak",
            "mujrim-viri",
            "mujrim-obs",
            "mujrim-plenty",
            "mujrim-lc0",
        ] {
            let candidates = engine_candidates(
                product,
                Path::new("C:/Mujrim/mujrim-ui.exe"),
                Path::new("D:/src/mujrim"),
                None,
            );
            let expected = Path::new("C:/Mujrim")
                .join("engines")
                .join("mujrim")
                .join("bin")
                .join(&target)
                .join(with_exe_suffix(product));
            assert!(
                candidates.contains(&expected),
                "missing {expected:?} in {candidates:?}"
            );
        }
    }

    #[test]
    fn upstream_comparison_engines_remain_unsuffixed() {
        for upstream in [
            "stockfish",
            "akimbo",
            "reckless",
            "lc0",
            "viridithas",
            "hobbes",
            "integral",
            "velvet",
        ] {
            assert_eq!(
                packaged_executable_filename(upstream, "linux-x86_64"),
                executable_filename(upstream)
            );
        }
    }

    #[test]
    fn linux_x86_64_catalog_layout_resolves_new_engines() {
        let exe = Path::new("/opt/mujrim/mujrim-ui");
        for id in [
            "lc0",
            "viridithas",
            "hobbes",
            "integral",
            "velvet",
            "stockfish",
        ] {
            let candidates = engine_candidates(id, exe, Path::new("/tmp"), None);
            let expected = Path::new("/opt/mujrim")
                .join("engines")
                .join(id)
                .join("bin")
                .join("linux-x86_64")
                .join(id);
            let host = Path::new("/opt/mujrim")
                .join("engines")
                .join(id)
                .join("bin")
                .join(RuntimePlatform::current().directory_name())
                .join(id);
            assert!(
                candidates.contains(&expected) || candidates.contains(&host),
                "missing linux layout for {id}; got {candidates:?}"
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
    fn adapter_binary_stem_is_product_id() {
        assert_eq!(
            adapter_binary_stem("mujrim-v60", "x86_64-avx2"),
            "mujrim-v60"
        );
        assert_eq!(adapter_binary_stem("mujrim-akimbo", "aarch64"), "mujrim-ak");
        assert_eq!(
            adapter_binary_stem("mujrim-viridithas", "x86_64"),
            "mujrim-viri"
        );
        assert_eq!(
            adapter_binary_stem("mujrim-obsidian", "aarch64"),
            "mujrim-obs"
        );
        assert_eq!(adapter_binary_stem("mujrim", "x86_64"), "mujrim-elite");
        assert_eq!(package_directory("mujrim-viri"), "mujrim");
        assert_eq!(package_directory("mujrim-obs"), "mujrim");
        assert_eq!(
            adapter_binary_stem("mujrim-plentychess", "x86_64"),
            "mujrim-plenty"
        );
        assert_eq!(adapter_binary_stem("mujrim-leela", "aarch64"), "mujrim-lc0");
        assert_eq!(package_directory("mujrim-plenty"), "mujrim");
        assert_eq!(package_directory("mujrim-lc0"), "mujrim");
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
    fn engine_roots_include_exe_local_and_cwd_not_parent_trees() {
        let roots = engine_search_roots(
            Path::new("C:/Mujrim/windows-aarch64/mujrim-ui.exe"),
            Path::new("D:/src/mujrim"),
        );
        assert_eq!(
            roots,
            vec![
                PathBuf::from("C:/Mujrim/windows-aarch64/engines"),
                PathBuf::from("D:/src/mujrim/engines"),
                PathBuf::from("D:/src/mujrim/dist")
                    .join(RuntimePlatform::current().directory_name())
                    .join("engines"),
            ]
        );
        assert!(
            !roots.iter().any(
                |root| root.ends_with("dist/engines") || root == Path::new("C:/Mujrim/engines")
            ),
            "must not search parent or unscoped dist/engines trees: {roots:?}"
        );
    }

    #[test]
    fn discover_bundled_engines_finds_cwd_vendor_tree() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mujrim-catalog-cwd-{}-{}",
            std::process::id(),
            stamp
        ));
        let target = RuntimePlatform::current().directory_name();
        let bin_dir = root
            .join("engines")
            .join("velvet")
            .join("bin")
            .join(&target);
        std::fs::create_dir_all(&bin_dir).unwrap();
        let engine = bin_dir.join(executable_filename("velvet"));
        let native_machine = match crate::binary_arch::BinaryArch::host() {
            crate::binary_arch::BinaryArch::Aarch64 => 0xB7,
            _ => 0x3E,
        };
        std::fs::write(
            &engine,
            crate::binary_arch::synthetic_elf_bytes(native_machine),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&engine).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&engine, perms).unwrap();
        }
        let ui = root.join("elsewhere").join("mujrim-ui");
        std::fs::create_dir_all(ui.parent().unwrap()).unwrap();
        std::fs::write(&ui, []).unwrap();
        let found = discover_bundled_engines(&ui, &root);
        let ids: Vec<&str> = found.iter().map(|engine| engine.id).collect();
        assert!(
            ids.contains(&"velvet"),
            "cwd engines/ must be discovered when the UI binary has no sibling engines/: {ids:?}"
        );
        assert_eq!(found[0].path, engine);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn local_engines_root_is_beside_executable() {
        assert_eq!(
            local_engines_root(Path::new("C:/Mujrim/mujrim-ui.exe")),
            Some(PathBuf::from("C:/Mujrim/engines"))
        );
    }

    #[test]
    fn catalog_keeps_upstream_engines_for_native_comparison() {
        let ids: Vec<&str> = BUNDLED_ENGINES.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&"stockfish"));
        assert!(ids.contains(&"akimbo"));
        assert!(ids.contains(&"reckless"));
        assert!(ids.contains(&"mujrim-elite"));
        assert!(ids.contains(&"mujrim-external"));
        assert!(ids.contains(&"mujrim-v60"));
        assert!(ids.contains(&"mujrim-ak"));
        assert!(ids.contains(&"mujrim-viri"));
        assert!(ids.contains(&"mujrim-obs"));
        assert!(ids.contains(&"mujrim-plenty"));
        assert!(ids.contains(&"mujrim-lc0"));
        assert!(ids.contains(&"lc0"));
        assert!(ids.contains(&"viridithas"));
        assert!(ids.contains(&"hobbes"));
        assert!(ids.contains(&"integral"));
        assert!(ids.contains(&"velvet"));
        assert!(!ids.contains(&"mujrim-v10"));
        assert!(!ids.iter().any(|id| id.contains("native")));
        assert!(
            !BUNDLED_ENGINES
                .iter()
                .any(|(_, display)| display.to_ascii_lowercase().contains("native"))
        );
        assert_eq!(MUJRIM_HCE_DISPLAY_NAME, "Mujrim HCE");
    }

    #[test]
    fn host_arch_filter_rejects_wrong_elf_in_linux_layout() {
        let root = std::env::temp_dir().join(format!(
            "mujrim-catalog-elf-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let target = RuntimePlatform::current().directory_name();
        let bin_dir = root
            .join("engines")
            .join("velvet")
            .join("bin")
            .join(&target);
        std::fs::create_dir_all(&bin_dir).unwrap();
        let engine = bin_dir.join(executable_filename("velvet"));
        let foreign_machine =
            if crate::binary_arch::BinaryArch::host() == crate::binary_arch::BinaryArch::Aarch64 {
                0x3E
            } else {
                0xB7
            };
        std::fs::write(
            &engine,
            crate::binary_arch::synthetic_elf_bytes(foreign_machine),
        )
        .unwrap();
        let ui = root.join("mujrim-ui");
        std::fs::write(&ui, []).unwrap();
        let err = discover_engine_details("velvet", &ui, &root, None).unwrap_err();
        assert!(
            err.contains("could not find velvet"),
            "wrong ELF machine must not be selected: {err}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    #[test]
    fn windows_arm64_lists_x64_engine_folders_as_emulated() {
        let candidates = engine_candidate_details(
            "obsidian",
            Path::new("C:/Mujrim/mujrim.exe"),
            Path::new("D:/src/mujrim"),
            None,
        );
        assert!(
            candidates.iter().any(|candidate| {
                candidate.target_directory.contains("x86_64")
                    && candidate.compatibility == RuntimeCompatibility::Emulated
            }),
            "expected emulated x64 candidates, got {candidates:?}"
        );
        assert!(candidates.iter().all(|candidate| {
            !candidate.target_directory.contains("x86_64")
                || candidate.compatibility == RuntimeCompatibility::Emulated
        }));
    }

    #[test]
    fn dist_ui_discovers_local_native_mujrim_engines_when_present() {
        let ui = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dist/mujrim-ui.exe");
        let cwd = match ui.parent() {
            Some(parent) => parent.to_path_buf(),
            None => return,
        };
        if !ui.is_file() {
            return;
        }
        let found = discover_bundled_engines(&ui, &cwd);
        let ids: Vec<&str> = found.iter().map(|engine| engine.id).collect();
        for id in [
            "mujrim-elite",
            "mujrim-external",
            "mujrim-v60",
            "mujrim-ak",
            "stockfish",
        ] {
            assert!(
                ids.contains(&id),
                "missing {id} beside dist UI; discovered {ids:?}"
            );
        }
    }

    #[test]
    fn vendored_linux_x86_64_bins_are_elf_when_present() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../engines");
        if !root.is_dir() {
            return;
        }
        let mut found = 0usize;
        for id in [
            "stockfish",
            "akimbo",
            "reckless",
            "ethereal",
            "obsidian",
            "lc0",
            "viridithas",
            "hobbes",
            "integral",
            "velvet",
        ] {
            let path = root.join(id).join("bin").join("linux-x86_64").join(id);
            if !path.is_file() {
                continue;
            }
            found += 1;
            let magic = std::fs::read(&path).unwrap();
            assert!(
                magic.len() >= 4 && magic[..4] == [0x7F, b'E', b'L', b'F'],
                "{} must be ELF: {}",
                id,
                path.display()
            );
            assert!(
                crate::binary_arch::detect_binary_arch(&path)
                    == Some(crate::binary_arch::BinaryArch::X86_64),
                "{} must be ELF x86_64: {}",
                id,
                path.display()
            );
        }
        let _ = found;
    }
}
