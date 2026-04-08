<p align="center">
  <img src="logo.png" alt="KishMat Logo" width="220" />
</p>

<h1 align="center">KishMat</h1>

<p align="center">
  <b>Rust chess engine and GUI project</b><br/>
  UCI and XBoard support, NNUE + classical evaluation, and cross-platform builds.
</p>

<p align="center">
  <a href="#overview"><img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust" /></a>
  <a href="License.md"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT" /></a>
  <a href="#engine-features"><img src="https://img.shields.io/badge/search-Lazy%20SMP-green?style=flat-square" alt="Lazy SMP" /></a>
  <a href="#engine-features"><img src="https://img.shields.io/badge/protocol-UCI%20%7C%20XBoard-lightgrey?style=flat-square" alt="UCI | XBoard" /></a>
  <a href="#engine-features"><img src="https://img.shields.io/badge/eval-Classical%20%2B%20NNUE-purple?style=flat-square" alt="Classical + NNUE" /></a>
  <a href="#gui-features"><img src="https://img.shields.io/badge/gui-iced%200.14-cyan?style=flat-square" alt="iced 0.14 GUI" /></a>
</p>

<p align="center">
  <a href="https://ko-fi.com/thehamdiz">Support on Ko-fi</a> ·
  <a href="License.md">MIT License</a>
</p>

---

## Overview

KishMat is a modular Rust chess workspace with separate crates for core board logic, evaluation, search, protocols, GUI, updater, tooling, benchmarks, and installer packaging.

Current release line: **v1**.

## Quick Start

```bash
git clone https://github.com/theHamdiz/kishmat.git
cd kishmat
```

### Build and run

```bash
# Optimized engine build
cargo build --release -p kishmat

# UCI mode
./target/release/kishmat uci

# XBoard mode
./target/release/kishmat xboard

# GUI
cargo run --release -p kishmat-ui
```

### Local quality gate

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
RUST_MIN_STACK=16777216 cargo test --workspace
```

## Architecture

Workspace highlights:

- `crates/kishmat-types`: board model, move generation, bitboards, zobrist.
- `crates/kishmat-eval`: classical evaluation and NNUE adapters.
- `crates/kishmat-search`: alpha-beta/PVS, pruning, move ordering, SMP.
- `crates/kishmat-comms`: UCI/XBoard protocol handling.
- `crates/kishmat-ui`: native GUI.
- `crates/kishmat-updater`: update and tuning parameter surface.
- `crates/kishmat-benchmarker`: benchmarking and iterative measurement tools.
- `crates/kishmat-tooling`: release/install/tool automation.

Release profile uses full optimization (`lto = "fat"`, `codegen-units = 1`, `opt-level = 3`, `panic = "abort"`).

## Engine Features

- Hybrid evaluation: NNUE-first with classical support.
- Search: iterative deepening, aspiration windows, PVS, TT, LMR/LMP, null-move, SEE pruning, singular extension handling.
- Opening support through embedded book paths.
- UCI and XBoard protocol support.

## GUI Features

- Human vs Human, Human vs Engine, Engine vs Engine modes.
- Configurable themes and persistent settings.
- Move list, engine info panel, PGN export.
- GIF export and recording utilities.

## Benchmarks (Latest)

### BK suite proxy (latest local run)

Settings: depth `16`, `30s` per position, release build, strongest NNUE from `nnue/`, Stockfish search preset.

- Accuracy: `16/24` (`66.67%`) — short runs can vary by about one position either way.
- Approx CCRL 40/15 proxy: `~2150` (piecewise mapping from accuracy; **100% on this suite → 3500** on the same proxy scale).
- Approx Lichess blitz proxy: `~2265`
- Start position NPS (5s sample): `~4.0M`

This proxy is for regression tracking only. It is **not** real CCRL or Lichess list Elo. Reaching **3500** on this scale requires essentially a perfect BK pass (or better suite coverage), which is far above today’s strength; `kishmat-benchmarker iterate` uses `--target-elo 3500` by default as that ceiling.

### Elo iterate (BK loop)

- Example: `cargo run --release -p kishmat-benchmarker -- iterate --target-elo 3500 --depth 16 --time 30`
- Stops when the proxy reaches the target, BK count hits `--min-bk`, or stagnation / max rounds. With current play strength, expect stagnation **below** 3500.

### Head-to-head baseline (latest local run)

- `kishmat` vs `stockfish` (12 games, short TC, alternating colors)
- Score: `0.5 / 12` (0W / 1D / 11L)
- Estimated delta in that setup: `~ -545 Elo`

This head-to-head baseline is the active optimization target.

## CI/CD and Releases

CI runs:

- format check
- clippy (`-D warnings`)
- workspace tests
- native smoke test (`uci` handshake)
- cross-target build checks

Release workflow publishes archives for:

- macOS: `aarch64`, `x86_64`, and universal
- Linux: `x86_64` + `aarch64` (gnu and musl variants)
- Windows: `x86_64`

Artifacts include engine, updater, and platform-appropriate GUI binaries, plus NNUE metadata/payload directories.

## Installation

```bash
just install
just uninstall
```

Platform install behavior:

- macOS: app bundle + CLI tooling.
- Linux: local binaries and desktop entry.
- Windows: local app directory and shortcut.

## Development Commands

```bash
just build
just release
just test
just lint
just fmt
just check
just ui
just bench
```

## Support

- Ko-fi: [https://ko-fi.com/thehamdiz](https://ko-fi.com/thehamdiz)
- Email: [contact@hamdiz.me](mailto:contact@hamdiz.me)

## License

MIT — see `License.md`.