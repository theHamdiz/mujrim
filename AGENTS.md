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
- Training data: `mujrim train catalog|fetch|decode|merge|datagen|ateed` download Stockfish/Lc0/self-play dumps. Stockfish sets are `.plain` or `.plain.gz` only. `fetch` decompresses gzip/zstd and decodes Mujrim text / Stockfish `.plain` / MJBP / PGN / Lc0 v3–v6 into train-ready `FEN|score|wdl` in the same step. `--mix` weighted-interleaves sources instead of concatenating them.
- Floem title-bar **Ateed** tab is password-gated (secret stored only as Base64 `SkFIQU5BTQ==`). Fetch / decode / merge / train / datagen call the discovered `mujrim` CLI and stream `progress` lines; those actions stay disabled when the binary is missing. Evaluate and latency probes stay in-process.
- Play games, tournaments, and Ateed CLI jobs write crash-safe sidecars (`active-game.toml`, `active-tournament.toml`, `active-ateed-job.toml`, plus `{output}.job` / `{output}.partial`). After a power cut the UI offers resume; finished tournament games are kept and the interrupted pairing is replayed from the start. Train/datagen/fetch continue from the last completed epoch, game, or HTTP Range part.
- Concurrent (arena) tournaments keep the right sidebar but hide the move list; they render a stable equal-cell live board grid (one tile per concurrent game) instead of a single switching board. Single-board tournaments and other game screens still show the move list.
- GUI tournaments handshake each engine's advertised UCI options and only set Hash/Threads/EvalFile/WeightsFile when that engine implements them. `mujrim-*` wrappers get no Lc0 CLI argv; raw `lc0` uses `plan_launch`. Event Elo is a field rating (not CCRL 40/15); Stockfish is the only hard CCRL pin.
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
- Ateed studio lives behind the title-bar Ateed pill (`Screen::Ateed`); unlock compares against the decoded Base64 gate. Fetch/decode/merge/train/datagen stay disabled until a `mujrim` CLI is discovered.

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
