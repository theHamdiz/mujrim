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

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").expect("target architecture");
    let compiler: std::ffi::OsString = std::env::var_os("WINDRES").map_or_else(
        || match arch.as_str() {
            "aarch64" => "aarch64-w64-mingw32-windres".into(),
            "x86_64" => "x86_64-w64-mingw32-windres".into(),
            _ => "windres".into(),
        },
        Into::into,
    );
    let output = std::path::Path::new(
        &std::env::var("OUT_DIR").expect("build output directory"),
    )
    .join("as-invoker-manifest.o");
    let status = std::process::Command::new(&compiler)
        .current_dir(resource_dir)
        .args(["--input-format=rc", "--output-format=coff", "app.rc", "-o"])
        .arg(&output)
        .status()
        .unwrap_or_else(|error| panic!("failed to run {compiler:?}: {error}"));
    assert!(status.success(), "Windows manifest compilation failed");
    println!("cargo:rustc-link-arg={}", output.display());
}
