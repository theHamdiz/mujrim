# Mujrim Agent Guide

Updated: 2026-04-08

## Repository Rules
- Always check latest dependency versions from the internet and read their docs before implementing updates.
- Every code change must include unit/integration tests.
- Keep code idiomatic and remove AI-sounding wording/comments.
- Do not ignore warnings/errors, even outside the immediate task.
- Stay DRY: reuse existing functionality before adding new code.
- Prefer modular, trait-based designs with no unnecessary runtime overhead.

## Current State
- A structured AI-agent tool surface is implemented in `mujrim-tooling`.
- Entry point: `mujrim-tooling agent`.
- Output contract: JSON (machine-readable for agent orchestration).
- Tool domains are split by responsibility:
  - `engine.*`
  - `gui.*`
  - `tooling.*`
  - `updater.*`

## Agent Tool Commands
- `mujrim-tooling agent list [--pretty]`
- `mujrim-tooling agent describe <tool> [--pretty]`
- `mujrim-tooling agent call <tool> --input '<json-object>' [--pretty]`

## Implemented Tool Set
- `engine.analyze`
- `engine.perft`
- `gui.settings.path`
- `gui.settings.read`
- `gui.piece_sets.list`
- `tooling.build_variants`
- `tooling.release_targets`
- `updater.nnue.catalog`
- `updater.nnue.status`
- `updater.tuning.read`

## Open UI Workstream
- Add multiple high-quality piece sets under separate folders (`default` + additional sets) and expose runtime switching in GUI settings.
- Replace emoji-based UI icons with professional vector iconography suitable for `iced`.

## CI/CD Baseline (Required)
- CI must run: format, clippy (`-D warnings`), workspace tests, and an engine smoke test.
- Release pipeline must produce artifacts for major platforms, including:
  - macOS `aarch64` + `x86_64` + universal bundle
  - Linux `x86_64` + `aarch64` (gnu and musl variants)
  - Windows `x86_64`
- Every release artifact should contain engine + UI (when supported) + updater + NNUE payload/metadata.
- Smoke validation in release jobs should verify `mujrim` responds correctly to UCI handshake input.

## Current Iteration Commands
- Quality gate:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `RUST_MIN_STACK=16777216 cargo test --workspace`
- Fast strength estimate:
  - `just bench-json depth=16 hash=128 time=5`
