fn main() {
    if std::env::var("CARGO_FEATURE_SYZYGY").is_err() {
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        return;
    }

    let fathom = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("mujrim-native-v60")
        .join("deps")
        .join("Fathom");
    cc::Build::new()
        .include(&fathom)
        .file(fathom.join("tbprobe.c"))
        .flag_if_supported("-Wno-deprecated-declarations")
        .flag_if_supported("-Wno-sign-compare")
        .flag_if_supported("-Wno-macro-redefined")
        .opt_level(3)
        .compile("fathom");
    println!(
        "cargo:rerun-if-changed={}",
        fathom.join("tbprobe.c").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        fathom.join("tbprobe.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        fathom.join("tbchess.c").display()
    );
}
