use std::fs;
use std::path::{Path, PathBuf};

use crate::action::ToolAction;
use crate::process::{output, run};
use mujrim_protocols::catalog::{RuntimePlatform, adapter_binary_stem, host_packaging_arch};

/// All distributable binaries produced by the workspace.
const BINS: &[&str] = &[
    "mujrim",
    "mujrim-benchmarker",
    "mujrim-ui",
    "mujrim-updater",
];

/// Dedicated product engines copied into `dist/<os-arch>/engines/mujrim/bin/<os-arch>/`.
const PRODUCT_ENGINE_STEMS: &[&str] = &[
    "mujrim-ak",
    "mujrim-ateed",
    "mujrim-elite",
    "mujrim-external",
    "mujrim-obs",
    "mujrim-plenty",
    "mujrim-v60",
    "mujrim-viri",
];

const EXTERNAL_ENGINE_FEATURES: &str =
    "xboard,book,nnue,simd,akimbo-nnue,stockfish-nnue,reckless-nnue,viridithas-nnue,obsidian-nnue";

/// Binaries that require host GPU/windowing libraries and cannot be
/// cross-compiled for non-host Linux (Floem/wgpu).
const HOST_ONLY_BINS: &[&str] = &["mujrim-ui"];

/// Packages to `--exclude` when cross-compiling for Linux.
/// mujrim-ui depends on Floem/wgpu. Updater uses rustls and can cross.
const CROSS_EXCLUDE_LINUX: &[&str] = &["mujrim-ui"];

/// Packages to `--exclude` when cross-compiling for Windows/macOS.
/// Only mujrim-ui (Floem/wgpu).
const CROSS_EXCLUDE_OTHER: &[&str] = &["mujrim-ui"];

/// Packages that are dev-only and never distributed.
/// They still compile as part of `--workspace` but we don't copy them.
#[allow(dead_code)]
const DEV_ONLY: &[&str] = &["mujrim-tooling", "mujrim-tests"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WindowsMsvcTarget {
    triple: &'static str,
    directory: &'static str,
}

const WINDOWS_MSVC_TARGETS: &[WindowsMsvcTarget] = &[
    WindowsMsvcTarget {
        triple: "aarch64-pc-windows-msvc",
        directory: "aarch64",
    },
    WindowsMsvcTarget {
        triple: "x86_64-pc-windows-msvc",
        directory: "x86_64",
    },
];

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
                println!("║       Mujrim Cross-Platform Release        ║");
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
                    println!("║  ✅ Built:   {:32} ║", succeeded.join(", "));
                }
                if !skipped.is_empty() {
                    println!("║  ⚠️  Skipped: {:32} ║", skipped.join(", "));
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
    println!("🔨 Building all Mujrim crates (optimized release, runtime ISA dispatch)...");
    let environment = [("CARGO_BUILD_JOBS", "1")];
    run(
        "cargo",
        &[
            "build",
            "--release",
            "--workspace",
            "--exclude",
            "mujrim-ui",
            "--exclude",
            "mujrim-installer",
        ],
        &environment,
    )?;
    run(
        "cargo",
        &[
            "build",
            "--release",
            "-p",
            "mujrim-native-v60",
            "--features",
            "syzygy",
        ],
        &environment,
    )?;
    snapshot_engine("mujrim-v60", "mujrim-v60-external")?;
    let arch = host_packaging_arch();
    // Product set: elite / external / v60 / ak / viri / obs
    // Never enable embedded-networks on top of default features — that embeds every net.
    run(
        "cargo",
        &[
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            EXTERNAL_ENGINE_FEATURES,
        ],
        &environment,
    )?;
    snapshot_engine("mujrim", "mujrim-external")?;
    run(
        "cargo",
        &[
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "xboard,book,nnue,simd,stockfish-nnue,embedded-networks",
        ],
        &environment,
    )?;
    snapshot_engine("mujrim", &adapter_binary_stem("mujrim-elite", &arch))?;
    snapshot_engine("mujrim", "mujrim-embedded")?;
    run(
        "cargo",
        &[
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "xboard,book,nnue,simd,akimbo-nnue,embedded-networks",
        ],
        &environment,
    )?;
    snapshot_engine("mujrim", &adapter_binary_stem("mujrim-ak", &arch))?;
    run(
        "cargo",
        &[
            "build",
            "--release",
            "-p",
            "mujrim-native-v60",
            "--features",
            "syzygy,embedded-network",
        ],
        &environment,
    )?;
    snapshot_engine("mujrim-v60", &adapter_binary_stem("mujrim-v60", &arch))?;
    snapshot_engine("mujrim-v60", "mujrim-v60-embedded")?;
    run(
        "cargo",
        &[
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "xboard,book,nnue,simd,viridithas-nnue",
        ],
        &environment,
    )?;
    snapshot_engine("mujrim", "mujrim-viri")?;
    run(
        "cargo",
        &[
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "xboard,book,nnue,simd,obsidian-nnue",
        ],
        &environment,
    )?;
    snapshot_engine("mujrim", "mujrim-obs")?;
    run(
        "cargo",
        &[
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "xboard,book,nnue,simd,plentychess-nnue",
        ],
        &environment,
    )?;
    snapshot_engine("mujrim", "mujrim-plenty")?;
    run(
        "cargo",
        &[
            "build",
            "--release",
            "-p",
            "mujrim",
            "--no-default-features",
            "--features",
            "xboard,book,nnue,simd,ateed-nnue",
        ],
        &environment,
    )?;
    snapshot_engine("mujrim", "mujrim-ateed")?;
    // Leave mujrim.exe as the lean external (no embedded net).
    snapshot_engine("mujrim-external", "mujrim")?;
    run(
        "cargo",
        &["build", "--release", "-p", "mujrim-ui"],
        &environment,
    )?;
    run(
        "cargo",
        &[
            "build",
            "--release",
            "-p",
            "mujrim-installer",
            "--features",
            "embed",
        ],
        &environment,
    )?;
    publish_native_dist()?;
    println!(
        "✅ Release binaries built in target/release/ and copied to {}",
        native_dist_root().display()
    );
    Ok(())
}

fn native_dist_root() -> PathBuf {
    Path::new("dist").join(RuntimePlatform::current().directory_name())
}

fn publish_native_dist() -> Result<(), String> {
    let suffix = std::env::consts::EXE_SUFFIX;
    let release = Path::new("target/release");
    let platform = RuntimePlatform::current();
    let dist = native_dist_root();
    fs::create_dir_all(&dist)
        .map_err(|error| format!("failed to create {}: {error}", dist.display()))?;

    let extras = [
        "mujrim",
        "mujrim-benchmarker",
        "mujrim-tooling",
        "mujrim-ui",
        "mujrim-updater",
    ];
    for stem in extras {
        let source = release.join(format!("{stem}{suffix}"));
        if source.is_file() {
            fs::copy(&source, dist.join(format!("{stem}{suffix}"))).map_err(|error| {
                format!(
                    "failed to copy {} into {}: {error}",
                    source.display(),
                    dist.display()
                )
            })?;
        }
    }

    let packaged = dist
        .join("engines")
        .join("mujrim")
        .join("bin")
        .join(platform.directory_name());
    fs::create_dir_all(&packaged)
        .map_err(|error| format!("failed to create {}: {error}", packaged.display()))?;
    for stem in PRODUCT_ENGINE_STEMS {
        let source = release.join(format!("{stem}{suffix}"));
        if !source.is_file() {
            continue;
        }
        let metadata = fs::metadata(&source)
            .map_err(|error| format!("failed to stat {}: {error}", source.display()))?;
        if metadata.len() == 0 {
            return Err(format!(
                "refusing to publish empty product binary {}",
                source.display()
            ));
        }
        fs::copy(&source, packaged.join(format!("{stem}{suffix}"))).map_err(|error| {
            format!(
                "failed to copy {} into {}: {error}",
                source.display(),
                packaged.display()
            )
        })?;
    }
    publish_nnue_into(&dist)?;
    Ok(())
}

fn publish_nnue_into(dist: &Path) -> Result<(), String> {
    let dest = dist.join("nnue");
    let legacy = Path::new("dist").join("nnue");
    let resources = Path::new("crates/mujrim-eval/resources");
    for source in [Path::new("nnue"), legacy.as_path(), resources] {
        if !source.is_dir() || source == dest.as_path() {
            continue;
        }
        fs::create_dir_all(&dest)
            .map_err(|error| format!("failed to create {}: {error}", dest.display()))?;
        for entry in fs::read_dir(source)
            .map_err(|error| format!("failed to read {}: {error}", source.display()))?
        {
            let entry =
                entry.map_err(|error| format!("failed to read {}: {error}", source.display()))?;
            let from = entry.path();
            if !from.is_file() {
                continue;
            }
            if source == resources {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !name.starts_with("lc0_") || !name.ends_with(".pb.gz") {
                    continue;
                }
            }
            let to = dest.join(entry.file_name());
            fs::copy(&from, &to).map_err(|error| {
                format!(
                    "failed to copy {} into {}: {error}",
                    from.display(),
                    to.display()
                )
            })?;
        }
    }
    Ok(())
}

fn snapshot_engine(source_stem: &str, destination_stem: &str) -> Result<(), String> {
    let suffix = std::env::consts::EXE_SUFFIX;
    let source = Path::new("target")
        .join("release")
        .join(format!("{source_stem}{suffix}"));
    let destination = Path::new("target")
        .join("release")
        .join(format!("{destination_stem}{suffix}"));
    if source == destination {
        return Ok(());
    }
    fs::copy(&source, &destination).map_err(|error| {
        format!(
            "failed to snapshot {} as {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
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
            &[("RUSTFLAGS", "")],
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

    if any_ok {
        Ok(())
    } else {
        Err("all macOS builds failed".into())
    }
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
                "build".into(),
                "--release".into(),
                "--workspace".into(),
                "--target".into(),
                "x86_64-unknown-linux-gnu".into(),
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

    if any_ok {
        Ok(())
    } else {
        Err("all Linux builds failed".into())
    }
}

fn build_windows() -> Result<(), String> {
    if cfg!(windows) {
        return build_windows_msvc();
    }

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

fn build_windows_msvc() -> Result<(), String> {
    let installed = installed_targets()?;
    let current_target = format!("{}-pc-windows-msvc", std::env::consts::ARCH);
    let dist = Path::new("dist/windows");
    prepare_dist(dist)?;
    let mut built = 0usize;

    for target in WINDOWS_MSVC_TARGETS {
        if !installed.contains(target.triple) {
            println!("  -> Installing rustup target {}...", target.triple);
            run("rustup", &["target", "add", target.triple], &[])?;
        }

        println!("  -> {}...", target.triple);
        let args = cross_cargo_args(target.triple, CROSS_EXCLUDE_OTHER);
        run(
            "cargo",
            &args.iter().map(|arg| arg.as_str()).collect::<Vec<_>>(),
            &[("CARGO_BUILD_JOBS", "1"), ("RUSTFLAGS", "")],
        )?;

        let source = PathBuf::from(format!("target/{}/release", target.triple));
        let destination = dist.join(target.directory);
        fs::create_dir_all(&destination)
            .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;
        for binary in BINS {
            if HOST_ONLY_BINS.contains(binary) && target.triple != current_target {
                continue;
            }
            let source = source.join(format!("{binary}.exe"));
            if source.is_file() {
                let destination = destination.join(format!("{binary}.exe"));
                fs::copy(&source, &destination).map_err(|error| {
                    format!(
                        "failed to copy {} to {}: {error}",
                        source.display(),
                        destination.display()
                    )
                })?;
                built += 1;
            }
        }
    }

    if built == 0 {
        Err("no Windows MSVC release binaries were produced".into())
    } else {
        Ok(())
    }
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
        if let Err(e) = run(
            "sudo",
            &{
                let mut full = vec!["pacman"];
                full.extend(args);
                full
            },
            &[],
        ) {
            eprintln!("    ⚠️  Auto-install failed: {e}");
            eprintln!("    💡 Install manually: sudo pacman -S {}", pkgs.join(" "));
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
            tools
                .iter()
                .map(|(b, _, _)| *b)
                .collect::<Vec<_>>()
                .join(", ")
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
                let file_count = fs::read_dir(&path).into_iter().flatten().count();
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
                println!("{prefix}{connector}{} {}", name.to_string_lossy(), size);
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
        assert_eq!(
            BINS,
            sorted.as_slice(),
            "BINS should be sorted alphabetically"
        );
    }

    #[test]
    fn host_only_bins_are_subset_of_bins() {
        for b in HOST_ONLY_BINS {
            assert!(BINS.contains(b), "{b} in HOST_ONLY_BINS but not in BINS");
        }
    }

    #[test]
    fn cross_excludes_are_non_empty() {
        for pkg in CROSS_EXCLUDE_LINUX.iter().chain(CROSS_EXCLUDE_OTHER.iter()) {
            assert!(!pkg.is_empty());
        }
    }

    #[test]
    fn cross_cargo_args_includes_excludes() {
        let args = cross_cargo_args("aarch64-unknown-linux-gnu", CROSS_EXCLUDE_LINUX);
        let args_str: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        assert!(args_str.contains(&"--exclude"));
        assert!(args_str.contains(&"mujrim-ui"));
        assert!(args_str.contains(&"--target"));
        assert!(args_str.contains(&"aarch64-unknown-linux-gnu"));
    }

    #[test]
    fn human_size_formats() {
        assert_eq!(human_size(500), "(500 B)");
        assert_eq!(human_size(12_000), "(12 KB)");
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

    #[test]
    fn windows_msvc_release_matrix_covers_arm64_and_x86_64() {
        assert_eq!(
            WINDOWS_MSVC_TARGETS,
            &[
                WindowsMsvcTarget {
                    triple: "aarch64-pc-windows-msvc",
                    directory: "aarch64",
                },
                WindowsMsvcTarget {
                    triple: "x86_64-pc-windows-msvc",
                    directory: "x86_64",
                },
            ]
        );
    }

    #[test]
    fn adapter_snapshot_stems_are_product_ids() {
        use mujrim_protocols::catalog::arch_token_from_rustc_target;
        let arch = arch_token_from_rustc_target("x86_64-pc-windows-msvc");
        for adapter in [
            "mujrim-v60",
            "mujrim-elite",
            "mujrim-ak",
            "mujrim-viri",
            "mujrim-obs",
            "mujrim-plenty",
            "mujrim-ateed",
        ] {
            let stem = adapter_binary_stem(adapter, &arch);
            assert!(!stem.contains("native"), "{stem}");
            assert_eq!(stem, adapter);
        }
        assert_eq!(
            adapter_binary_stem("mujrim-v10", "x86_64-avx2"),
            "mujrim-elite"
        );
        assert_eq!(adapter_binary_stem("mujrim-akimbo", "aarch64"), "mujrim-ak");
    }

    #[test]
    fn native_release_snapshots_installer_payload_stems() {
        let src = include_str!("release.rs");
        let native = src
            .split("fn build_native()")
            .nth(1)
            .and_then(|rest| rest.split("fn snapshot_engine(").next())
            .expect("native build");
        for stem in [
            "mujrim-external",
            "mujrim-embedded",
            "mujrim-v60-external",
            "mujrim-v60-embedded",
            "mujrim-viri",
            "mujrim-obs",
            "mujrim-plenty",
            "mujrim-ateed",
        ] {
            assert!(
                native.contains(&format!("\"{stem}\"")),
                "missing installer snapshot {stem}"
            );
        }
    }

    #[test]
    fn snapshot_engine_skips_copying_a_binary_onto_itself() {
        let src = include_str!("release.rs");
        let snapshot = src
            .split("fn snapshot_engine(")
            .nth(1)
            .and_then(|rest| rest.split("fn build_darwin(").next())
            .expect("snapshot_engine");
        assert!(
            snapshot.contains("if source == destination"),
            "copying mujrim-v60 onto itself truncates the file to 0 bytes"
        );
    }

    #[test]
    fn native_dist_publishes_under_the_host_platform_directory() {
        let src = include_str!("release.rs");
        let publish = src
            .split("fn publish_native_dist()")
            .nth(1)
            .and_then(|rest| rest.split("fn snapshot_engine(").next())
            .expect("publish_native_dist");
        assert!(
            publish.contains("native_dist_root()"),
            "native publish must target dist/<os-arch>"
        );
        assert!(
            publish.contains("publish_nnue_into"),
            "platform tree must receive the NNUE payload"
        );
        let publish_nnue = src
            .split("fn publish_nnue_into(")
            .nth(1)
            .and_then(|rest| rest.split("fn snapshot_engine(").next())
            .expect("publish_nnue_into");
        assert!(
            publish_nnue.contains("lc0_") && publish_nnue.contains(".pb.gz"),
            "dist nnue/ must receive the official Lc0 BT4 sidecar"
        );
        assert!(
            publish.contains("metadata.len() == 0"),
            "empty product binaries must not replace a good v60"
        );
        assert!(
            !publish.contains("PRODUCT_ENGINE_STEMS.iter().chain(extras"),
            "product engines must not be copied to the dist root"
        );
        let root = native_dist_root();
        assert_eq!(
            root,
            PathBuf::from("dist").join(RuntimePlatform::current().directory_name())
        );
        assert_ne!(root, PathBuf::from("dist"));
    }

    #[test]
    fn product_engine_stems_cover_the_dist_set() {
        assert_eq!(
            PRODUCT_ENGINE_STEMS,
            &[
                "mujrim-ak",
                "mujrim-ateed",
                "mujrim-elite",
                "mujrim-external",
                "mujrim-obs",
                "mujrim-plenty",
                "mujrim-v60",
                "mujrim-viri",
            ]
        );
        assert!(EXTERNAL_ENGINE_FEATURES.contains("viridithas-nnue"));
        assert!(EXTERNAL_ENGINE_FEATURES.contains("obsidian-nnue"));
        assert!(!EXTERNAL_ENGINE_FEATURES.contains("embedded-networks"));
        let src = include_str!("release.rs");
        let obs_features = src
            .lines()
            .find(|line| {
                line.contains("xboard,book,nnue,simd,obsidian-nnue") && !line.contains("//")
            })
            .expect("mujrim-obs feature list");
        assert!(
            !obs_features.contains("stockfish-nnue"),
            "mujrim-obs must not compile Stockfish NNUE: {obs_features}"
        );
        assert!(src.contains("\"xboard,book,nnue,simd,plentychess-nnue\""));
        assert!(
            src.contains("snapshot_engine(\"mujrim\", \"mujrim-plenty\")")
                && src.contains("\"xboard,book,nnue,simd,plentychess-nnue\""),
            "mujrim-plenty must be built from plentychess-nnue, not copied from another adapter"
        );
        assert!(src.contains("\"xboard,book,nnue,simd,ateed-nnue\""));
        assert!(
            src.contains("snapshot_engine(\"mujrim\", \"mujrim-ateed\")")
                && src.contains("\"xboard,book,nnue,simd,ateed-nnue\""),
            "mujrim-ateed must be built from ateed-nnue, not copied from another adapter"
        );
    }
}
