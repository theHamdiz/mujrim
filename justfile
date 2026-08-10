# Mujrim Chess Engine

# Default target
default: build

# Build the optimized engine
build:
    CARGO_BUILD_JOBS=1 cargo build --release -p mujrim

# Build optimized release binaries for all crates with runtime ISA dispatch.
release:
    cargo run --release -p mujrim-tooling -- release native

# Release build for macOS (aarch64 + x86_64)
release-darwin:
    cargo run --release -p mujrim-tooling -- release darwin

# Release build for Linux (x86_64 + aarch64)
release-linux:
    cargo run --release -p mujrim-tooling -- release linux

# Release build for Windows (x86_64, requires cargo-xwin or cross)
release-win:
    cargo run --release -p mujrim-tooling -- release win

# Release build for ALL platforms
release-full:
    cargo run --release -p mujrim-tooling -- release full

# Run optimized tests with a memory-safe release-derived profile.
test:
    CARGO_BUILD_JOBS=1 RUST_MIN_STACK=16777216 cargo test --profile release-test --workspace

# Run the engine in UCI mode
run:
    cargo run --release -- uci

# Run the engine in interactive play mode
play:
    cargo run --release -- play -d 8

# Analyze a FEN position
analyze fen depth="10":
    cargo run --release -- analyze -f "{{fen}}" -d {{depth}}

# Run perft test
perft depth="6":
    cargo run --release -- perft -d {{depth}}

# Run the ELO benchmark suite — auto-detects all hardware
bench depth="20" threads="" hash="256" time="30":
    @echo "Running Mujrim Benchmark Suite..."
    cargo run --release -p mujrim-benchmarker -- bench -d {{depth}} --hash {{hash}} --time {{time}} {{ if threads != "" { "--threads " + threads } else { "" } }}

# Run benchmark and emit machine-readable JSON summary
bench-json depth="20" threads="" hash="256" time="30":
    @echo "Running Mujrim Benchmark Suite (JSON)..."
    cargo run --release -p mujrim-benchmarker -- bench -d {{depth}} --hash {{hash}} --time {{time}} --json --quiet {{ if threads != "" { "--threads " + threads } else { "" } }}

# Fast paired strength match with fixed node budgets and sequential stopping
duel candidate reference nodes="5000" pairs="32" concurrency="1" checkpoint="" elo0="-30" elo1="10" reference_elo="":
    {{ if os() == "windows" { "powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/duel-preflight.ps1" } else { "true" } }}
    CARGO_BUILD_JOBS=1 RUST_MIN_STACK=16777216 cargo run --release -p mujrim-benchmarker -- duel "{{candidate}}" "{{reference}}" --nodes {{nodes}} --pairs {{pairs}} --concurrency {{concurrency}} --hash 16 --threads 1 --max-engine-memory 384 --max-match-memory 768 --session-pairs 1 --elo0 {{elo0}} --elo1 {{elo1}} --json {{ if checkpoint != "" { "--checkpoint \"" + checkpoint + "\"" } else { "" } }} {{ if reference_elo != "" { "--reference-elo " + reference_elo } else { "" } }}

# Benchmark an external UCI engine binary
bench-uci engine depth="16" hash="128" threads="1" time="30":
    @echo "Benchmarking external engine: {{engine}}"
    cargo run --release -p mujrim-benchmarker -- uci {{engine}} -d {{depth}} --hash {{hash}} -t {{threads}} --time {{time}}

# Benchmark an external XBoard engine binary
bench-xboard engine depth="16" hash="128" threads="1" time="30":
    @echo "Benchmarking external XBoard engine: {{engine}}"
    cargo run --release -p mujrim-benchmarker -- xboard {{engine}} -d {{depth}} --hash {{hash}} -t {{threads}} --time {{time}}

# Show engine info (NNUE, techniques, hardware)
engine-info:
    cargo run --release -p mujrim-benchmarker -- info

# Run a quick NPS benchmark at depth 16
nps:
    @echo "NPS Benchmark (depth 16, startpos, JSON summary)..."
    @cargo run --release -p mujrim-benchmarker -- bench -d 16 --time 5 --json --quiet

# Build and run the GUI application
ui:
    @echo "Building Mujrim GUI..."
    CARGO_BUILD_JOBS=1 cargo build --release -p mujrim-ui
    @echo "Launching Mujrim Chess GUI..."
    cargo run --release -p mujrim-ui

# Build and run the Bevy chess game (always release)
game:
    @echo "Building Mujrim Bevy Game..."
    CARGO_BUILD_JOBS=1 cargo run --release -p mujrim-game

# Build the updater
updater:
    cargo build --release -p mujrim-updater

# Run the updater (check for updates)
check-updates:
    cargo run --release -p mujrim-updater -- check

# Run the updater (update all components)
update:
    cargo run --release -p mujrim-updater -- update all

# Profile-guided optimization build (optional, requires nightly Rust)
pgo:
    @echo "Building with Profile-Guided Optimization (needs nightly)..."
    @echo "  Pass 1: Instrumented build..."
    RUSTFLAGS="-Cprofile-generate=/tmp/mujrim-pgo" cargo +nightly build --release
    @echo "  Collecting profile data..."
    @printf 'uci\nisready\nposition startpos\ngo depth 14\nquit\n' | ./target/release/mujrim 2>&1 > /dev/null
    @echo "  Pass 2: Optimized build with PGO data..."
    RUSTFLAGS="-Cprofile-use=/tmp/mujrim-pgo" cargo +nightly build --release
    @echo "  PGO build complete!"

# Clean build artifacts
clean:
    cargo clean

# Format all code
fmt:
    cargo fmt --all

# Run clippy lints
lint:
    CARGO_BUILD_JOBS=1 cargo clippy --release --workspace --all-targets -- -D warnings

# Check without building
check:
    CARGO_BUILD_JOBS=1 cargo check --release --workspace --all-targets

# ──────────────────────────────────────────────────────────────
# NNUE Network Adapter Variants
# ──────────────────────────────────────────────────────────────

# Build with every supported neural-network adapter
build-full:
    cargo run --release -p mujrim-tooling -- build-variant full

# Build with embedded NNUE only (no external network loading)
build-embedded:
    cargo run --release -p mujrim-tooling -- build-variant embedded

# Build the lean engine used for candidate benchmarks.
build-benchmark target_dir="target/benchmark":
    CARGO_BUILD_JOBS=1 RUST_MIN_STACK=16777216 cargo build --release -p mujrim --no-default-features --features nnue,simd,reckless-nnue --target-dir "{{target_dir}}"

# Build minimal engine (no book, no adapters, no GUI extras)
build-minimal:
    cargo run --release -p mujrim-tooling -- build-variant minimal

# List all available NNUE build variants
build-variants:
    cargo run --release -p mujrim-tooling -- build-variant list

# ──────────────────────────────────────────────────────────────
# NNUE Network Downloads
# Download latest networks from top open-source engines.
# All networks are saved to crates/mujrim-eval/resources/
# and excluded from Git via .gitignore.
# ──────────────────────────────────────────────────────────────

# Directory for all downloaded networks
nets_dir := "crates/mujrim-eval/resources"

# Download ALL latest NNUE networks from supported engines
nets:
    cargo run --release -p mujrim-tooling -- nnue all --dir "{{nets_dir}}"
    cargo run --release -p mujrim-tooling -- nnue status --dir "{{nets_dir}}"

# Show all downloaded NNUE networks
net-status:
    cargo run --release -p mujrim-tooling -- nnue status --dir "{{nets_dir}}"

# Benchmark with a specific external network file
bench-net net_path depth="16" hash="128" time="30":
    @echo "Benchmarking Mujrim with runtime NNUE file: {{net_path}}"
    cargo run --release -p mujrim-benchmarker -- bench -d {{depth}} --hash {{hash}} --time {{time}} --eval-file "{{net_path}}" --eval-preset auto

# ──────────────────────────────────────────────────────────────
# Install Mujrim — bundles everything into a single package
# Book + NNUE network are compiled into the binary (fastest: direct memory access)
# - macOS:   .app bundle in ~/Applications with CLI engine + updater
# - Linux:   ~/.local/bin + .desktop entry
# - Windows: %LOCALAPPDATA%/Mujrim with Start Menu shortcut
# ──────────────────────────────────────────────────────────────
install:
    cargo run --release -p mujrim-tooling -- install

# Uninstall Mujrim from all platforms
uninstall:
    cargo run --release -p mujrim-tooling -- uninstall

# ──────────────────────────────────────────────────────────────
# Installer — single binary that bundles all release artifacts
# ──────────────────────────────────────────────────────────────

# Build the installer (builds workspace first, then embeds binaries)
installer:
    @echo "Building engine and updater in maximal release mode..."
    CARGO_BUILD_JOBS=1 cargo build --release --workspace --exclude mujrim-ui --exclude mujrim-game --exclude mujrim-installer
    @echo "Building desktop clients with maximal fat LTO..."
    CARGO_BUILD_JOBS=1 cargo build --release -p mujrim-ui
    CARGO_BUILD_JOBS=1 cargo build --release -p mujrim-game
    @echo "Building installer with embedded binaries..."
    CARGO_BUILD_JOBS=1 cargo build --release -p mujrim-installer --features embed

# Run the installer GUI
run-installer:
    cargo run --release -p mujrim-installer --features embed
