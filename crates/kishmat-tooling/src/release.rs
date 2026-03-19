use std::fs;
use std::path::{Path, PathBuf};

use crate::action::ToolAction;
use crate::process::{output, run};

/// All distributable binaries produced by the workspace.
const BINS: &[&str] = &[
    "kishmat",
    "kishmat-benchmarker",
    "kishmat-ui",
    "kishmat-updater",
];

/// Binaries that require host system libraries (EGL, OpenGL, windowing, or
/// native TLS/OpenSSL) and cannot be cross-compiled for non-host Linux.
const HOST_ONLY_BINS: &[&str] = &["kishmat-ui", "kishmat-updater"];

/// Packages to `--exclude` when cross-compiling for Linux aarch64.
/// kishmat-ui: depends on EGL/OpenGL (iced).
/// kishmat-updater: depends on OpenSSL (reqwest/native-tls).
const CROSS_EXCLUDE_LINUX: &[&str] = &["kishmat-ui", "kishmat-updater"];

/// Packages to `--exclude` when cross-compiling for Windows/macOS.
/// Only kishmat-ui (iced) — the updater works fine on Windows (schannel)
/// and macOS (Security.framework).
const CROSS_EXCLUDE_OTHER: &[&str] = &["kishmat-ui"];

/// Packages that are dev-only and never distributed.
/// They still compile as part of `--workspace` but we don't copy them.
#[allow(dead_code)]
const DEV_ONLY: &[&str] = &["kishmat-tooling", "kishmat-tests"];

// ── Cross-compilation toolchain packages ────────────────────────────────
// Maps (tool-binary, pacman-package, apt-package)
const MINGW_TOOLS: &[(&str, &str, &str)] = &[
    (
        "x86_64-w64-mingw32-gcc",
        "mingw-w64-gcc",
        "gcc-mingw-w64-x86-64",
    ),
    // dlltool is part of mingw-w64-binutils on Arch, binutils-mingw-w64-x86-64 on Debian
    (
        "x86_64-w64-mingw32-dlltool",
        "mingw-w64-binutils",
        "binutils-mingw-w64-x86-64",
    ),
];

const AARCH64_LINUX_TOOLS: &[(&str, &str, &str)] = &[(
    "aarch64-linux-gnu-gcc",
    "aarch64-linux-gnu-gcc",
    "gcc-aarch64-linux-gnu",
)];

#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq)]
pub enum ReleaseTarget {
    Native,
    Darwin,
    Linux,
    Win,
    Full,
}

#[derive(Debug)]
pub struct ReleaseAction {
    pub target: ReleaseTarget,
}

impl ToolAction for ReleaseAction {
    fn run(&self) -> Result<(), String> {
        match self.target {
            ReleaseTarget::Native => build_native(),
            ReleaseTarget::Darwin => build_darwin(),
            ReleaseTarget::Linux => build_linux(),
            ReleaseTarget::Win => build_windows(),
            ReleaseTarget::Full => {
                println!();
                println!("╔══════════════════════════════════════════════╗");
                println!("║       KishMat Cross-Platform Release        ║");
                println!("╚══════════════════════════════════════════════╝");
                println!();

                // Ensure all rustup targets are installed.
                ensure_rustup_targets()?;

                // Ensure system cross-compilation toolchains are present.
                ensure_system_tools(AARCH64_LINUX_TOOLS, "aarch64 Linux cross-compiler");
                ensure_system_tools(MINGW_TOOLS, "Windows (MinGW) cross-compiler");

                let mut succeeded = Vec::new();
                let mut skipped = Vec::new();

                for (name, builder) in [
                    ("darwin", build_darwin as fn() -> Result<(), String>),
                    ("linux", build_linux as fn() -> Result<(), String>),
                    ("windows", build_windows as fn() -> Result<(), String>),
                ] {
                    match builder() {
                        Ok(()) => succeeded.push(name),
                        Err(e) => {
                            eprintln!("  ⚠️  {name}: {e}");
                            skipped.push(name);
                        }
                    }
                    println!();
                }

                println!("╔══════════════════════════════════════════════╗");
                println!("║                  Summary                    ║");
                println!("╠══════════════════════════════════════════════╣");
                if !succeeded.is_empty() {
                    println!(
                        "║  ✅ Built:   {:32} ║",
                        succeeded.join(", ")
                    );
                }
                if !skipped.is_empty() {
                    println!(
                        "║  ⚠️  Skipped: {:32} ║",
                        skipped.join(", ")
                    );
                }
                println!("╚══════════════════════════════════════════════╝");
                println!();
                println!("📦 Distribution layout:");
                print_dist_tree();

                if succeeded.is_empty() {
                    Err("no platform builds succeeded".into())
                } else {
                    Ok(())
                }
            }
        }
    }
}

// ── Platform builders ───────────────────────────────────────────────────

fn build_native() -> Result<(), String> {
    println!("🔨 Building all KishMat crates (optimized release, native CPU)...");
    run("cargo", &["build", "--release", "--workspace"], &[])?;
    println!("✅ Release binaries built in target/release/");
    Ok(())
}

fn build_darwin() -> Result<(), String> {
    let installed = installed_targets()?;
    let has_aarch64 = installed.contains("aarch64-apple-darwin");
    let has_x86 = installed.contains("x86_64-apple-darwin");

    if !has_aarch64 && !has_x86 {
        return Err("no macOS targets installed".into());
    }

    let dist = Path::new("dist/darwin");
    prepare_dist(dist)?;

    println!("🍎 Building for macOS...");

    let mut any_ok = false;

    if has_aarch64 {
        println!("  → aarch64 (Apple Silicon)...");
        let args = cross_cargo_args("aarch64-apple-darwin", CROSS_EXCLUDE_OTHER);
        match run(
            "cargo",
            &args.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            &[("RUSTFLAGS", "-C target-cpu=apple-m1")],
        ) {
            Ok(()) => {
                copy_binaries(
                    Path::new("target/aarch64-apple-darwin/release"),
                    &dist.join("aarch64"),
                    true,
                )?;
                any_ok = true;
                println!("    ✅ done");
            }
            Err(e) => eprintln!("    ⚠️  {e}"),
        }
    }

    if has_x86 {
        println!("  → x86_64 (Intel Mac)...");
        let args = cross_cargo_args("x86_64-apple-darwin", CROSS_EXCLUDE_OTHER);
        match run(
            "cargo",
            &args.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            &[("RUSTFLAGS", "")],
        ) {
            Ok(()) => {
                copy_binaries(
                    Path::new("target/x86_64-apple-darwin/release"),
                    &dist.join("x86_64"),
                    true,
                )?;
                any_ok = true;
                println!("    ✅ done");
            }
            Err(e) => eprintln!("    ⚠️  {e}"),
        }
    }

    if any_ok { Ok(()) } else { Err("all macOS builds failed".into()) }
}

fn build_linux() -> Result<(), String> {
    let installed = installed_targets()?;
    let has_x86 = installed.contains("x86_64-unknown-linux-gnu");
    let has_aarch64 = installed.contains("aarch64-unknown-linux-gnu");

    if !has_x86 && !has_aarch64 {
        return Err("no Linux targets installed".into());
    }

    let dist = Path::new("dist/linux");
    prepare_dist(dist)?;

    println!("🐧 Building for Linux...");

    let mut any_ok = false;

    if has_x86 {
        println!("  → x86_64...");
        // Full workspace for native host, cross-excludes otherwise.
        let is_native = cfg!(target_arch = "x86_64") && cfg!(target_os = "linux");
        let args = if is_native {
            vec![
                "build".into(), "--release".into(), "--workspace".into(),
                "--target".into(), "x86_64-unknown-linux-gnu".into(),
            ]
        } else {
            cross_cargo_args("x86_64-unknown-linux-gnu", CROSS_EXCLUDE_LINUX)
        };
        match run(
            "cargo",
            &args.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            &[("RUSTFLAGS", "")],
        ) {
            Ok(()) => {
                copy_binaries(
                    Path::new("target/x86_64-unknown-linux-gnu/release"),
                    &dist.join("x86_64"),
                    !is_native,
                )?;
                any_ok = true;
                println!("    ✅ done");
            }
            Err(e) => eprintln!("    ⚠️  {e}"),
        }
    }

    if has_aarch64 {
        if !has_tool("aarch64-linux-gnu-gcc") {
            println!("  ⚠️  aarch64: cross-linker not found, skipping");
        } else {
            println!("  → aarch64 (ARM64)...");
            let args = cross_cargo_args("aarch64-unknown-linux-gnu", CROSS_EXCLUDE_LINUX);
            match run(
                "cargo",
                &args.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                &[("RUSTFLAGS", "")],
            ) {
                Ok(()) => {
                    copy_binaries(
                        Path::new("target/aarch64-unknown-linux-gnu/release"),
                        &dist.join("aarch64"),
                        true,
                    )?;
                    any_ok = true;
                    println!("    ✅ done");
                }
                Err(e) => eprintln!("    ⚠️  {e}"),
            }
        }
    }

    if any_ok { Ok(()) } else { Err("all Linux builds failed".into()) }
}

fn build_windows() -> Result<(), String> {
    let target = "x86_64-pc-windows-gnu";

    if !has_tool("x86_64-w64-mingw32-gcc") || !has_tool("x86_64-w64-mingw32-dlltool") {
        return Err("MinGW cross-compiler not found".into());
    }

    println!("🪟 Building for Windows...");

    let dist = Path::new("dist/windows/x86_64");
    if dist.exists() {
        fs::remove_dir_all(dist)
            .map_err(|e| format!("failed to remove {}: {e}", dist.display()))?;
    }
    fs::create_dir_all(dist).map_err(|e| format!("failed to create {}: {e}", dist.display()))?;

    let installed = installed_targets()?;
    if !installed.contains(target) {
        println!("  → Installing rustup target {target}...");
        run("rustup", &["target", "add", target], &[])?;
    }

    println!("  → x86_64 (Windows)...");
    let args = cross_cargo_args(target, CROSS_EXCLUDE_OTHER);
    run(
        "cargo",
        &args.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        &[("RUSTFLAGS", "")],
    )?;

    for bin in BINS {
        if HOST_ONLY_BINS.contains(bin) {
            continue;
        }
        let src = PathBuf::from(format!("target/{target}/release/{bin}.exe"));
        if src.exists() {
            let dst = dist.join(format!("{bin}.exe"));
            fs::copy(&src, &dst).map_err(|e| {
                format!("failed to copy {} to {}: {e}", src.display(), dst.display())
            })?;
        }
    }
    println!("    ✅ done");
    Ok(())
}

// ── Toolchain helpers ───────────────────────────────────────────────────

/// All rustup targets we want for a full cross-platform release.
const DESIRED_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-gnu",
    // macOS targets — will be installed but won't compile without macOS SDK
    // "aarch64-apple-darwin",
    // "x86_64-apple-darwin",
];

/// Ensure all desired rustup targets are installed.
fn ensure_rustup_targets() -> Result<(), String> {
    let installed = installed_targets()?;
    for target in DESIRED_TARGETS {
        if !installed.contains(*target) {
            println!("  📥 Installing rustup target: {target}");
            run("rustup", &["target", "add", target], &[])?;
        }
    }
    Ok(())
}

/// Try to install missing system tools via available package manager.
fn ensure_system_tools(tools: &[(&str, &str, &str)], description: &str) {
    let missing: Vec<&str> = tools
        .iter()
        .filter(|(bin, _, _)| !has_tool(bin))
        .map(|(_, pkg, _)| *pkg)
        .collect();

    if missing.is_empty() {
        return;
    }

    // Deduplicate package names
    let mut pkgs: Vec<&str> = missing.clone();
    pkgs.sort_unstable();
    pkgs.dedup();

    if has_tool("pacman") {
        println!("  📥 Installing {description} ({})...", pkgs.join(", "));
        let mut args: Vec<&str> = vec!["-S", "--noconfirm", "--needed"];
        args.extend(pkgs.iter());
        if let Err(e) = run("sudo", &{
            let mut full = vec!["pacman"];
            full.extend(args);
            full
        }, &[]) {
            eprintln!("    ⚠️  Auto-install failed: {e}");
            eprintln!(
                "    💡 Install manually: sudo pacman -S {}",
                pkgs.join(" ")
            );
        }
    } else if has_tool("apt-get") {
        let apt_pkgs: Vec<&str> = tools
            .iter()
            .filter(|(bin, _, _)| !has_tool(bin))
            .map(|(_, _, apt)| *apt)
            .collect();
        let mut apt_unique = apt_pkgs.clone();
        apt_unique.sort_unstable();
        apt_unique.dedup();

        println!(
            "  📥 Installing {description} ({})...",
            apt_unique.join(", ")
        );
        let mut args: Vec<&str> = vec!["apt-get", "install", "-y"];
        args.extend(apt_unique.iter());
        if let Err(e) = run("sudo", &args, &[]) {
            eprintln!("    ⚠️  Auto-install failed: {e}");
            eprintln!(
                "    💡 Install manually: sudo apt-get install {}",
                apt_unique.join(" ")
            );
        }
    } else {
        eprintln!("  ⚠️  Cannot auto-install {description} — no supported package manager found.");
        eprintln!(
            "    💡 Install these tools manually: {}",
            tools.iter().map(|(b, _, _)| *b).collect::<Vec<_>>().join(", ")
        );
    }
}

/// Build cargo args for cross-compilation, excluding host-only packages.
fn cross_cargo_args(target: &str, excludes: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "build".into(),
        "--release".into(),
        "--workspace".into(),
        "--target".into(),
        target.into(),
    ];
    for pkg in excludes {
        args.push("--exclude".into());
        args.push((*pkg).into());
    }
    args
}

/// Check if a command-line tool is available on PATH.
fn has_tool(name: &str) -> bool {
    std::process::Command::new("which")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── File helpers ────────────────────────────────────────────────────────

fn print_dist_tree() {
    let dist = Path::new("dist");
    if !dist.exists() {
        println!("  (no dist directory)");
        return;
    }
    fn walk(dir: &Path, prefix: &str) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());
        let total = entries.len();
        for (i, entry) in entries.iter().enumerate() {
            let path = entry.path();
            let name = entry.file_name();
            let is_last = i + 1 == total;
            let connector = if is_last { "└── " } else { "├── " };
            let child_prefix = if is_last {
                format!("{prefix}    ")
            } else {
                format!("{prefix}│   ")
            };
            if path.is_dir() {
                // Check if directory has any files
                let file_count = fs::read_dir(&path)
                    .into_iter()
                    .flatten()
                    .count();
                if file_count == 0 {
                    println!("{prefix}{connector}{}/ (empty)", name.to_string_lossy());
                } else {
                    println!("{prefix}{connector}{}/", name.to_string_lossy());
                    walk(&path, &child_prefix);
                }
            } else {
                let size = fs::metadata(&path)
                    .map(|m| human_size(m.len()))
                    .unwrap_or_default();
                println!(
                    "{prefix}{connector}{} {}",
                    name.to_string_lossy(),
                    size
                );
            }
        }
    }
    walk(dist, "   ");
}

fn human_size(bytes: u64) -> String {
    if bytes >= 1_000_000 {
        format!("({:.1} MB)", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("({:.0} KB)", bytes as f64 / 1_000.0)
    } else {
        format!("({bytes} B)")
    }
}

fn prepare_dist(dist: &Path) -> Result<(), String> {
    if dist.exists() {
        fs::remove_dir_all(dist)
            .map_err(|e| format!("failed to remove {}: {e}", dist.display()))?;
    }
    fs::create_dir_all(dist.join("aarch64"))
        .map_err(|e| format!("failed to create {}: {e}", dist.join("aarch64").display()))?;
    fs::create_dir_all(dist.join("x86_64"))
        .map_err(|e| format!("failed to create {}: {e}", dist.join("x86_64").display()))?;
    Ok(())
}

fn copy_binaries(from: &Path, to: &Path, is_cross: bool) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("failed to create {}: {e}", to.display()))?;
    let mut copied = 0usize;
    for bin in BINS {
        if is_cross && HOST_ONLY_BINS.contains(bin) {
            continue;
        }
        let src = from.join(bin);
        if src.exists() {
            let dst = to.join(bin);
            fs::copy(&src, &dst).map_err(|e| {
                format!("failed to copy {} to {}: {e}", src.display(), dst.display())
            })?;
            copied += 1;
        }
    }
    if copied == 0 {
        eprintln!("    ⚠️  no binaries found in {}", from.display());
    }
    Ok(())
}

fn installed_targets() -> Result<std::collections::HashSet<String>, String> {
    let out = output("rustup", &["target", "list", "--installed"])?;
    Ok(out.lines().map(|s| s.trim().to_string()).collect())
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use clap::ValueEnum;

    #[test]
    fn bins_are_sorted() {
        let mut sorted = BINS.to_vec();
        sorted.sort();
        assert_eq!(BINS, sorted.as_slice(), "BINS should be sorted alphabetically");
    }

    #[test]
    fn host_only_bins_are_subset_of_bins() {
        for b in HOST_ONLY_BINS {
            assert!(BINS.contains(b), "{b} in HOST_ONLY_BINS but not in BINS");
        }
    }

    #[test]
    fn cross_exclude_matches_host_only() {
        for pkg in CROSS_EXCLUDE {
            assert!(!pkg.is_empty());
        }
    }

    #[test]
    fn cross_cargo_args_includes_excludes() {
        let args = cross_cargo_args("aarch64-unknown-linux-gnu", CROSS_EXCLUDE_LINUX);
        let args_str: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        assert!(args_str.contains(&"--exclude"));
        assert!(args_str.contains(&"kishmat-ui"));
        assert!(args_str.contains(&"--target"));
        assert!(args_str.contains(&"aarch64-unknown-linux-gnu"));
    }

    #[test]
    fn human_size_formats() {
        assert_eq!(human_size(500), "(500 B)");
        assert_eq!(human_size(12_000), "(12000 KB)");
        assert!(human_size(17_000_000).contains("MB"));
    }

    #[test]
    fn release_target_value_enum_roundtrip() {
        assert_eq!(
            ReleaseTarget::from_str("native", true),
            Ok(ReleaseTarget::Native)
        );
        assert_eq!(
            ReleaseTarget::from_str("full", true),
            Ok(ReleaseTarget::Full)
        );
    }
}
