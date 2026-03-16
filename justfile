# KishMat Chess Engine

# Default target
default: build

# Build in debug mode
build:
    cargo build --workspace

# Build optimized release binary
release:
    RUSTFLAGS="-C target-cpu=native" cargo build --release

# Run all tests
test:
    RUST_MIN_STACK=8388608 cargo test --workspace

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

# Run the ELO benchmark suite
bench:
    @echo "Running KishMat Benchmark Suite..."
    RUSTFLAGS="-C target-cpu=native" cargo run --release -- bench

# Run a quick NPS benchmark at depth 16
nps:
    @echo "NPS Benchmark (depth 16, startpos)..."
    @printf 'uci\nisready\nposition startpos\ngo depth 16\nquit\n' | cargo run --release 2>&1 | tail -5

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
