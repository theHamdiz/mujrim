use std::{
    env,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    process::Command,
};

mod attacks;
mod magics;
mod maps;

fn main() {
    generate_attack_maps();
    generate_compiler_info();
    generate_engine_version();

    #[cfg(feature = "syzygy")]
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("wasm32") {
        generate_syzygy_binding();
    }

    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/logs/HEAD");
}

#[cfg(feature = "syzygy")]
fn generate_syzygy_binding() {
    cc::Build::new()
        .compiler("clang")
        .include("./deps/Fathom")
        .file("./deps/Fathom/tbprobe.c")
        .flag("-Wno-deprecated-declarations")
        .flag("-Wno-sign-compare")
        .flag("-Wno-macro-redefined")
        .flag("-O3")
        .compile("fathom");

    // `src/bindings.rs` is generated from this pinned Fathom revision and is
    // checked in. Release builds therefore do not depend on a host libclang
    // installation and remain reproducible on Windows ARM64 build hosts.
    println!("cargo:rerun-if-changed=deps/Fathom/tbprobe.c");
    println!("cargo:rerun-if-changed=deps/Fathom/tbprobe.h");
    println!("cargo:rerun-if-changed=src/bindings.rs");
}

fn generate_attack_maps() {
    let dir = env::var("OUT_DIR").unwrap();
    let path = Path::new(&dir).join("lookup.rs");
    let out = File::create(path).unwrap();
    write(BufWriter::new(out)).unwrap();
}

fn write(mut buf: BufWriter<File>) -> Result<(), std::io::Error> {
    macro_rules! write_map {
        ($name:tt, $type:tt, $items:expr) => {
            writeln!(buf, "static {}: [{}; {}] = {:?};", $name, $type, $items.len(), $items)?;
        };
    }

    write_map!("DIAGONALS", "[u64; 64]", maps::generate_diagonal_tables());

    write_map!("KING_MAP", "u64", maps::generate_king_map());
    write_map!("KNIGHT_MAP", "u64", maps::generate_knight_map());

    write_map!("PAWN_MAP", "[u64; 64]", maps::generate_pawn_map());

    write_map!("RAYPASS", "[u64; 64]", maps::generate_rays_map());
    write_map!("BETWEEN", "[u64; 64]", maps::generate_between_map());

    write_map!("ROOK_MAP", "u64", maps::generate_rook_map());
    write_map!("BISHOP_MAP", "u64", maps::generate_bishop_map());

    write_map!("ROOK_MAGICS", "MagicEntry", magics::ROOK_MAGICS);
    write_map!("BISHOP_MAGICS", "MagicEntry", magics::BISHOP_MAGICS);

    writeln!(buf, "struct MagicEntry {{ pub mask: u64, pub magic: u64, pub shift: u32, pub offset: u32 }}")
}

fn generate_compiler_info() {
    fn get_env(key: &str) -> String {
        env::var(key).unwrap_or("unknown".to_owned())
    }

    let version = Command::new("rustc")
        .arg("--version")
        .output()
        .map(|v| String::from_utf8_lossy(&v.stdout).to_string())
        .unwrap_or("unknown".to_owned());

    println!("cargo:rustc-env=COMPILER_VERSION={version}");
    println!("cargo:rustc-env=COMPILER_TARGET={}", get_env("TARGET"));
    println!("cargo:rustc-env=COMPILER_FEATURES={}", get_env("CARGO_CFG_TARGET_FEATURE"));
}

fn generate_engine_version() {
    let version = env!("CARGO_PKG_VERSION");

    let git_sha = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .filter(|v| v.status.success())
        .and_then(|v| String::from_utf8(v.stdout).ok())
        .map(|v| v.trim().to_string());

    if let Some(sha) = git_sha {
        println!("cargo:rustc-env=ENGINE_VERSION={version}-{sha}")
    } else {
        println!("cargo:rustc-env=ENGINE_VERSION={version}")
    }
}
