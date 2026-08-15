# Mujrim Agent Guide

Updated: 2026-08-15

## Repository Rules
- Always check latest dependency versions from the internet and read their docs before implementing updates.
- Every code change must include unit/integration tests.
- Keep code idiomatic and remove AI-sounding wording/comments.
- Do not ignore warnings/errors, even outside the immediate task.
- Stay DRY: reuse existing functionality before adding new code.
- Prefer modular, trait-based designs with no unnecessary runtime overhead.

## Current State
- Multi-adapter in-process engine: Stockfish / Reckless / Akimbo / Viridithas / Obsidian / PlentyChess / Ateed / Mujrim HCE each bind eval + matching search via `EvalSearchAdapter`.
- Ateed is the Phase 2 MoE NNUE (`ATEED001`, disk `ateed_default.bin`, adapter id `ateed`, product binary `mujrim-ateed`). Default eval remains Reckless/`auto`.
- Phase 3: Ateed WDL variance widens futility/RFP and relieves LMR; NNUE downloads resume via HTTP Range; `mujrim train emit-ateed` writes a zero net.
- Phase 4: CPU `TrainCompute` matvec; `mujrim train datagen` / `mujrim train ateed` parse `FEN|score|wdl` and SGD Ateed heads (or expert 0 + FT).
- Phase 5: `--scope moe` trains the routed expert’s heads; `mujrim train fetch` resumes remote datasets via HTTP Range.
- Floem title-bar **Ateed** tab is password-gated (secret stored only as Base64 `SkFIQU5BTQ==`). The studio plans multi-source fetch, dry-runs train ticks, and evaluates an in-memory zero net — it does not download datasets or run a full train.
- Product surfaces: `--backend universal` (selectable), `--backend mujrim-hce` (classical HCE), `--backend v60`/`v10`/`akimbo` (packaged adapters), `--backend ateed` (in-process MoE), external upstream passthrough.
- Do not use “native” as an engine/backend product name (`RuntimeCompatibility::Native` is host-ISA packaging only).
- A structured AI-agent tool surface is implemented in `mujrim-tooling`.
- Entry point: `mujrim-tooling agent`.
- Output contract: JSON (machine-readable for agent orchestration).
- Tool domains are split by responsibility:
  - `engine.*`
  - `gui.*`
  - `tooling.*`
  - `updater.*`
- Default desktop GUI is Floem (`cargo run --release -p mujrim-ui`); Iced is `--no-default-features --features iced-ui,book,nnue`.
- Title-bar icons are embedded Lucide SVGs on the Floem path.
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
- Additional piece sets live under `crates/mujrim-ui/assets/pieces/` and switch at runtime in Options.
- Ateed studio lives behind the title-bar Ateed pill (`Screen::Ateed`); unlock compares against the decoded Base64 gate.

## CI/CD Baseline (Required)
- CI must run: format, clippy (`-D warnings`), workspace tests, and an engine smoke test (`uciok` + `Mujrim 1.0.0`).
- Native CI matrix: Linux x86_64 + Linux aarch64 (`ubuntu-24.04-arm`) + macOS ARM64 + Windows x86_64 + Windows ARM64 (`windows-11-arm`).
- Cross CI/release matrix: Linux `armv7`, `x86_64-musl`, `aarch64-musl` (engine + updater; UCI smoke via `cross run`).
- Release pipeline must produce artifacts for major platforms, including:
  - macOS `aarch64` + `x86_64` + universal bundle
  - Linux `x86_64` + `aarch64` (gnu full with UI; musl engine-only) + `armv7` engine-only
  - Windows `x86_64` + Windows `aarch64`
- Every release artifact should contain engine + UI (when supported) + updater + NNUE payload/metadata.
- Smoke validation in release jobs should verify `mujrim` responds correctly to UCI handshake input.

## Current Iteration Commands
- Quality gate:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `RUST_MIN_STACK=16777216 cargo test --workspace`
- Fast strength estimate:
  - `just bench-json depth=16 hash=128 time=5`
