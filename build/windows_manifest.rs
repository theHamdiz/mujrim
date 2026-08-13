fn embed_as_invoker() {
    let manifest_dir = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("manifest directory"),
    );
    let workspace_dir = if manifest_dir.join("build/app.rc").is_file() {
        manifest_dir
    } else {
        manifest_dir
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace directory")
            .to_path_buf()
    };
    let resource_dir = workspace_dir.join("build");
    println!(
        "cargo:rerun-if-changed={}",
        resource_dir.join("app.manifest").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        resource_dir.join("app.rc").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        workspace_dir.join("assets/branding/mujrim.ico").display()
    );

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("build output directory"));
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").expect("target architecture");

    if let Some(compiler) = resolve_windres(&arch) {
        let output = out_dir.join("as-invoker-manifest.o");
        let status = std::process::Command::new(&compiler)
            .current_dir(&resource_dir)
            .args(["--input-format=rc", "--output-format=coff", "app.rc", "-o"])
            .arg(&output)
            .status()
            .unwrap_or_else(|error| panic!("failed to run {compiler:?}: {error}"));
        assert!(status.success(), "Windows manifest compilation failed via windres");
        println!("cargo:rustc-link-arg={}", output.display());
        return;
    }

    // GitHub-hosted Windows runners typically have MSVC `rc.exe`, not MinGW windres.
    let rc = resolve_rc_exe().unwrap_or_else(|| std::ffi::OsString::from("rc.exe"));
    let output = out_dir.join("as-invoker-manifest.res");
    let status = std::process::Command::new(&rc)
        .current_dir(&resource_dir)
        .args(["/nologo", "/fo"])
        .arg(&output)
        .arg("app.rc")
        .status()
        .unwrap_or_else(|error| {
            panic!(
                "failed to compile Windows manifest: neither MinGW windres nor rc.exe is available ({error})"
            )
        });
    assert!(status.success(), "Windows manifest compilation failed via rc.exe");
    println!("cargo:rustc-link-arg={}", output.display());
}

fn resolve_windres(arch: &str) -> Option<std::ffi::OsString> {
    if let Some(explicit) = std::env::var_os("WINDRES") {
        return Some(explicit);
    }
    let candidates: &[&str] = match arch {
        "aarch64" => &[
            "aarch64-w64-mingw32-windres",
            "llvm-windres",
            "windres",
        ],
        "x86_64" => &["x86_64-w64-mingw32-windres", "llvm-windres", "windres"],
        _ => &["windres"],
    };
    candidates
        .iter()
        .find(|candidate| command_exists(candidate))
        .map(|candidate| std::ffi::OsString::from(*candidate))
}

fn resolve_rc_exe() -> Option<std::ffi::OsString> {
    if command_exists("rc.exe") || command_exists("rc") {
        return Some(std::ffi::OsString::from("rc.exe"));
    }

    let kits = std::path::Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let Ok(entries) = std::fs::read_dir(kits) else {
        return None;
    };
    let mut versions: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    versions.sort();
    for version in versions.into_iter().rev() {
        for host in ["x64", "arm64", "x86"] {
            let candidate = version.join(host).join("rc.exe");
            if candidate.is_file() {
                return Some(candidate.into_os_string());
            }
        }
    }
    None
}

fn command_exists(name: &str) -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("where")
            .arg(name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("sh")
            .args(["-c", &format!("command -v {name}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}
