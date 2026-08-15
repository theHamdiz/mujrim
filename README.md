<p align="center">
  <img src="assets/branding/mujrim-icon.png" alt="Mujrim Logo" width="220" />
</p>

<h1 align="center">Mujrim</h1>

<p align="center">
  <b>High-performance Rust chess suite</b><br/>
  Engine, desktop game, study workspace, tournaments, and reproducible benchmarks.
</p>

## Overview

Mujrim is a modular chess workspace with independent crates for board logic,
evaluation, search, protocols, benchmarking, study tools, desktop interfaces,
updates, and packaging. It supports UCI and XBoard, hybrid evaluation, bundled
engine discovery, architecture-aware launching, and strictly bounded match
workers.

## Quick Start

```bash
git clone https://github.com/theHamdiz/mujrim.git
cd mujrim

cargo build --release -p mujrim
./target/release/mujrim uci
cargo run --release -p mujrim-ui
```

## Workspace

- `mujrim-types`: board state, legal move generation, bitboards, and hashing.
- `mujrim-eval`: classical and efficiently updatable neural evaluation.
- `mujrim-search`: iterative deepening, PVS, transpositions, pruning, and SMP.
- `mujrim-comms`: UCI and XBoard protocol handling.
- `mujrim-protocols`: bounded external-process adapters and discovery.
- `mujrim-benchmarker`: tactical suites, paired matches, and tournaments.
- `mujrim-study`: PGN, local game database, openings, ratings, and training.
- `mujrim-ui`: native desktop game, analysis, coaching, and study interface.
- `mujrim-updater`, `mujrim-tooling`, `mujrim-installer`: release operations.

## Desktop Features

- Human, computer, and computer-versus-computer play.
- Architecture-aware bundled-engine discovery and safe process limits.
- Paired round-robin tournaments with reproducible openings and Elo estimates.
- Full-game review with move-quality annotations and coaching vocabulary.
- Searchable, deduplicated local PGN library with multi-game import.
- Legal replay from standard or custom starting positions.
- Opening, repertoire, puzzle, and spaced-repetition domain support.
- Animated pieces, configurable themes, premoves, and multiple board arrows.
- PGN, GIF, screenshot, and recording workflows.

## Benchmarks

`mujrim-benchmarker` runs deterministic tactical suites, paired fixed-node
matches, and resumable round-robin tournaments. Color-swapped opening pairs,
pentanomial statistics, confidence intervals, and sequential stopping are
reported as machine-readable JSON.

```bash
just bench-json 16 128 5
just duel ./target/release/mujrim ./path/to/reference 5000 64 1 duel.jsonl
```

The safe duel preset uses one worker, one engine thread, 384 MiB per process,
and a 768 MiB aggregate ceiling. Checkpoints persist completed pairs so an
interrupted session resumes without replaying finished games. Match estimates
are relative measurements under their stated conditions, not official ratings.

## Release Profiles

The engine `release` profile uses optimization level 3, fat link-time
optimization, one code-generation unit, abort-on-panic, and stripped symbols.
Large desktop binaries use thin link-time optimization with one code-generation
unit to keep link memory below the documented workstation safety ceiling while
retaining cross-crate optimization. Tests use the optimized
`release-test` profile; debug builds are not part of the supported workflow.

## Quality Gate

```bash
cargo fmt --all -- --check
cargo clippy --release --workspace --all-targets -- -D warnings
CARGO_BUILD_JOBS=1 RUST_MIN_STACK=16777216 cargo test --profile release-test --workspace
```

Release binaries use runtime ISA dispatch rather than build-host CPU flags, so
one binary per target architecture can select the fastest supported kernel on
the execution host. CI also performs a UCI handshake smoke test and validates
release targets for supported Windows, Linux, and macOS architectures.

## Installation

```bash
just install
just uninstall
```

Release packages include the engine, desktop interface where supported,
updater, installer, and neural-network metadata or payloads.

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

- [Ko-fi](https://ko-fi.com/thehamdiz)
- [contact@hamdiz.me](mailto:contact@hamdiz.me)

## License

MIT — see `License.md`.
