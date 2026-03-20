# KishMat Chess Engine

# Default target
default: build

# Build in debug mode
build:
    cargo build --workspace

# Build optimized release binaries for ALL crates (native CPU)
release:
    cargo run --release -p kishmat-tooling -- release native

# Release build for macOS (aarch64 + x86_64)
release-darwin:
    cargo run --release -p kishmat-tooling -- release darwin

# Release build for Linux (x86_64 + aarch64)
release-linux:
    cargo run --release -p kishmat-tooling -- release linux

# Release build for Windows (x86_64, requires cargo-xwin or cross)
release-win:
    cargo run --release -p kishmat-tooling -- release win

# Release build for ALL platforms
release-full:
    cargo run --release -p kishmat-tooling -- release full

# Run all tests (16MB stack for deep search recursion)
test:
    RUST_MIN_STACK=16777216 cargo test --workspace

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
bench depth="20" threads="" hash="128" time="120":
    @echo "Running KishMat Benchmark Suite..."
    RUSTFLAGS="-C target-cpu=native" cargo run --release -p kishmat-benchmarker -- bench -d {{depth}} --hash {{hash}} --time {{time}}

# Benchmark an external UCI engine binary
bench-uci engine depth="16" hash="128" threads="1" time="120":
    @echo "Benchmarking external engine: {{engine}}"
    RUSTFLAGS="-C target-cpu=native" cargo run --release -p kishmat-benchmarker -- uci {{engine}} -d {{depth}} --hash {{hash}} -t {{threads}} --time {{time}}

# Benchmark an external XBoard engine binary
bench-xboard engine depth="16" hash="128" threads="1" time="120":
    @echo "Benchmarking external XBoard engine: {{engine}}"
    RUSTFLAGS="-C target-cpu=native" cargo run --release -p kishmat-benchmarker -- xboard {{engine}} -d {{depth}} --hash {{hash}} -t {{threads}} --time {{time}}

# Show engine info (NNUE, techniques, hardware)
engine-info:
    RUSTFLAGS="-C target-cpu=native" cargo run --release -p kishmat-benchmarker -- info

# Run a quick NPS benchmark at depth 16
nps:
    @echo "NPS Benchmark (depth 16, startpos)..."
    @printf 'uci\nisready\nposition startpos\ngo depth 16\nquit\n' | cargo run --release 2>&1 | tail -5

# Build and run the GUI application
ui:
    @echo "Building KishMat GUI..."
    cargo build --release -p kishmat-ui
    @echo "Launching KishMat Chess GUI..."
    cargo run --release -p kishmat-ui

# Build the updater
updater:
    cargo build --release -p kishmat-updater

# Run the updater (check for updates)
check-updates:
    cargo run --release -p kishmat-updater -- check

# Run the updater (update all components)
update:
    cargo run --release -p kishmat-updater -- update all

# Profile-guided optimization build (optional, requires nightly Rust)
pgo:
    @echo "Building with Profile-Guided Optimization (needs nightly)..."
    @echo "  Pass 1: Instrumented build..."
    RUSTFLAGS="-C target-cpu=native -Cprofile-generate=/tmp/kishmat-pgo" cargo +nightly build --release
    @echo "  Collecting profile data..."
    @printf 'uci\nisready\nposition startpos\ngo depth 14\nquit\n' | ./target/release/kishmat 2>&1 > /dev/null
    @echo "  Pass 2: Optimized build with PGO data..."
    RUSTFLAGS="-C target-cpu=native -Cprofile-use=/tmp/kishmat-pgo" cargo +nightly build --release
    @echo "  PGO build complete!"

# Clean build artifacts
clean:
    cargo clean

# Format all code
fmt:
    cargo fmt --all

# Run clippy lints
lint:
    cargo clippy --workspace -- -D warnings

# Check without building
check:
    cargo check --workspace

# ──────────────────────────────────────────────────────────────
# NNUE Network Adapter Variants
# ──────────────────────────────────────────────────────────────

# Build with ALL NNUE adapters (Akimbo + Stockfish) — default
build-full:
    cargo run --release -p kishmat-tooling -- build-variant full

# Build with Akimbo-family adapter only (no Stockfish .nnue support)
build-akimbo:
    cargo run --release -p kishmat-tooling -- build-variant akimbo

# Build with Stockfish adapter only (no external Akimbo loader)
build-stockfish:
    cargo run --release -p kishmat-tooling -- build-variant stockfish

# Build with embedded NNUE only (no external network loading)
build-embedded:
    cargo run --release -p kishmat-tooling -- build-variant embedded

# Build minimal engine (no book, no adapters, no GUI extras)
build-minimal:
    cargo run --release -p kishmat-tooling -- build-variant minimal

# List all available NNUE build variants
build-variants:
    cargo run --release -p kishmat-tooling -- build-variant list

# ──────────────────────────────────────────────────────────────
# NNUE Network Downloads
# Download latest networks from top open-source engines.
# All networks are saved to crates/kishmat-eval/resources/
# and excluded from Git via .gitignore.
# ──────────────────────────────────────────────────────────────

# Directory for all downloaded networks
nets_dir := "crates/kishmat-eval/resources"

# Download ALL latest NNUE networks from supported engines
nets:
    cargo run --release -p kishmat-tooling -- nnue all --dir "{{nets_dir}}"
    cargo run --release -p kishmat-tooling -- nnue status --dir "{{nets_dir}}"

# Download latest Akimbo NNUE network
net-akimbo:
    cargo run --release -p kishmat-tooling -- nnue engine akimbo --dir "{{nets_dir}}"

# Download latest Stockfish NNUE networks (big + small)
net-stockfish:
    cargo run --release -p kishmat-tooling -- nnue engine stockfish --dir "{{nets_dir}}"

# Download latest Viridithas NNUE network
net-viridithas:
    cargo run --release -p kishmat-tooling -- nnue engine viridithas --dir "{{nets_dir}}"

# Download latest Alexandria NNUE network
net-alexandria:
    cargo run --release -p kishmat-tooling -- nnue engine alexandria --dir "{{nets_dir}}"

# Show all downloaded NNUE networks
net-status:
    cargo run --release -p kishmat-tooling -- nnue status --dir "{{nets_dir}}"

# Benchmark with a specific external network file
bench-net net_path depth="16" hash="128" time="120":
    @echo "Benchmarking KishMat with runtime NNUE file: {{net_path}}"
    RUSTFLAGS="-C target-cpu=native" cargo run --release -p kishmat-benchmarker -- bench -d {{depth}} --hash {{hash}} --time {{time}} --eval-file "{{net_path}}" --eval-preset auto

# ──────────────────────────────────────────────────────────────
# Install KishMat — bundles everything into a single package
# Book + NNUE network are compiled into the binary (fastest: direct memory access)
# - macOS:   .app bundle in ~/Applications with CLI engine + updater
# - Linux:   ~/.local/bin + .desktop entry
# - Windows: %LOCALAPPDATA%/KishMat with Start Menu shortcut
# ──────────────────────────────────────────────────────────────
install:
    cargo run --release -p kishmat-tooling -- install

# Uninstall KishMat from all platforms
uninstall:
    cargo run --release -p kishmat-tooling -- uninstall
