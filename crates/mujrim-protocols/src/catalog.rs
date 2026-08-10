//! Runtime discovery for bundled, architecture-specific chess engines.

use std::path::{Path, PathBuf};

pub const BUNDLED_ENGINES: &[(&str, &str)] = &[
    ("mujrim", "Mujrim Elite"),
    ("mujrim-v60", "Mujrim Native v60"),
    ("stockfish", "Stockfish"),
    ("plentychess", "PlentyChess"),
    ("obsidian", "Obsidian"),
    ("reckless", "Reckless"),
    ("akimbo", "Akimbo"),
    ("ethereal", "Ethereal"),
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

fn executable_filename(engine_id: &str) -> String {
    if cfg!(windows) {
        format!("{engine_id}.exe")
    } else {
        engine_id.to_owned()
    }
}

fn package_directory(engine_id: &str) -> &str {
    if engine_id == "mujrim-v60" {
        "mujrim"
    } else {
        engine_id
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
        targets.push(("windows-arm64".to_owned(), RuntimeCompatibility::Native));
        targets.push((
            "windows-x86_64-avx2".to_owned(),
            RuntimeCompatibility::Emulated,
        ));
        targets.push(("windows-x86_64".to_owned(), RuntimeCompatibility::Emulated));
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

/// Candidate locations in priority order. An explicit path always wins,
/// followed by native packaged builds and supported emulation fallbacks.
pub fn engine_candidate_details(
    engine_id: &str,
    executable: &Path,
    current_dir: &Path,
    explicit: Option<&Path>,
) -> Vec<EngineCandidate> {
    let filename = executable_filename(engine_id);
    let mut candidates = Vec::with_capacity(17);
    if let Some(path) = explicit {
        candidates.push(EngineCandidate {
            path: path.to_path_buf(),
            target_directory: "explicit".to_owned(),
            compatibility: RuntimeCompatibility::Native,
        });
    }
    if let Some(executable_dir) = executable.parent() {
        candidates.push(EngineCandidate {
            path: executable_dir.join(&filename),
            target_directory: "adjacent".to_owned(),
            compatibility: RuntimeCompatibility::Native,
        });
    }
    for root in engine_roots(executable, current_dir) {
        for (target_directory, compatibility) in runtime_targets() {
            let path = root
                .join(package_directory(engine_id))
                .join("bin")
                .join(&target_directory)
                .join(&filename);
            if !candidates.iter().any(|candidate| candidate.path == path) {
                candidates.push(EngineCandidate {
                    path,
                    target_directory,
                    compatibility,
                });
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
    candidates
        .iter()
        .find(|candidate| candidate.path.is_file())
        .cloned()
        .ok_or_else(|| {
            let searched = candidates
                .iter()
                .map(|candidate| candidate.path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "could not find {engine_id} for {} (searched: {searched})",
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
            Path::new(r"C:\Program Files\Mujrim\mujrim.exe"),
            Path::new(r"D:\src\mujrim"),
            Some(Path::new(r"E:\engines\reckless-custom.exe")),
        );
        assert_eq!(
            candidates.first().unwrap(),
            Path::new(r"E:\engines\reckless-custom.exe")
        );
    }

    #[test]
    fn flat_installer_layout_finds_adjacent_engine() {
        let candidates = engine_candidate_details(
            "mujrim-v60",
            Path::new(r"C:\Program Files\Mujrim\bin\mujrim.exe"),
            Path::new(r"D:\unrelated"),
            None,
        );
        assert_eq!(
            candidates.first().unwrap(),
            &EngineCandidate {
                path: PathBuf::from(r"C:\Program Files\Mujrim\bin\mujrim-v60.exe"),
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
            Path::new(r"C:\Mujrim\mujrim.exe"),
            Path::new(r"D:\src\mujrim"),
            None,
        );
        let expected = Path::new(r"C:\Mujrim")
            .join("engines")
            .join("reckless")
            .join("bin")
            .join(RuntimePlatform::current().directory_name())
            .join(executable_filename("reckless"));
        assert!(candidates.contains(&expected));
    }

    #[test]
    fn native_mujrim_backend_is_part_of_the_runtime_catalog() {
        assert_eq!(BUNDLED_ENGINES.first(), Some(&("mujrim", "Mujrim Elite")));
        let candidates = engine_candidates(
            "mujrim-v60",
            Path::new(r"C:\Mujrim\mujrim-ui.exe"),
            Path::new(r"D:\src\mujrim"),
            None,
        );
        assert!(
            candidates.contains(
                &Path::new(r"C:\Mujrim")
                    .join("engines")
                    .join("mujrim")
                    .join("bin")
                    .join(RuntimePlatform::current().directory_name())
                    .join(executable_filename("mujrim-v60"))
            )
        );
    }

    #[test]
    fn engine_inside_packaged_tree_finds_sibling_backends() {
        let candidates = engine_candidates(
            "stockfish",
            Path::new(r"C:\Mujrim\dist\engines\mujrim\bin\windows-aarch64\mujrim-elite.exe"),
            Path::new(r"D:\unrelated"),
            None,
        );
        assert!(candidates.iter().any(|candidate| {
            candidate
                == &Path::new(r"C:\Mujrim\dist\engines")
                    .join("stockfish")
                    .join("bin")
                    .join(RuntimePlatform::current().directory_name())
                    .join(executable_filename("stockfish"))
        }));
    }

    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    #[test]
    fn windows_arm64_falls_back_to_x64_avx2_emulation() {
        let candidates = engine_candidate_details(
            "obsidian",
            Path::new(r"C:\Mujrim\mujrim.exe"),
            Path::new(r"D:\src\mujrim"),
            None,
        );
        let fallback = candidates
            .iter()
            .find(|candidate| candidate.target_directory == "windows-x86_64-avx2")
            .unwrap();
        assert_eq!(fallback.compatibility, RuntimeCompatibility::Emulated);
    }
}
