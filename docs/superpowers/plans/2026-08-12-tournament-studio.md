# Tournament Studio Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a CuteChess-organized Tournament Studio with hybrid live N-boards, host-arch-only engines, version 1.0.0, and rebuilt Windows dist.

**Architecture:** Ply callbacks from `play_game` feed `TournamentEvent`s into an extended `LiveTournamentSnapshot`; UI modules (`tournament_setup`, `tournament_arena`, `tournament_results`) render Setup/Arena/Results under the existing Mujrim iced theme. Engine discovery rejects non-host PE/ELF/Mach-O machines.

**Tech Stack:** Rust 2024, iced 0.14, mujrim-benchmarker, mujrim-protocols, mujrim-ui.

## Global Constraints

- Product version strings and crate versions: **1.0.0**
- Tournament UI concurrency: **1..=4**
- Auto-detect engines: **host architecture only** (no emulated)
- Keep Mujrim theme (no CuteChess visual clone)
- Every change ships with unit/integration tests

---

## Task 1: Version bump 2.0.0 → 1.0.0

- [ ] Update workspace/`Cargo.toml` and all crate `version` fields to `1.0.0`
- [ ] Update UCI/XBoard/`id name`/banner/user-agent string literals
- [ ] Fix assertions that hardcode `2.0.0`
- [ ] `cargo test -p mujrim --lib` / targeted string tests
- [ ] Commit

## Task 2: Host-arch binary filter

- [ ] Add `binary_arch` helper (PE/ELF/Mach-O) in `mujrim-protocols` (or small shared util)
- [ ] Filter bundled discovery to native only; skip Emulated targets
- [ ] Filter `probe_adjacent_engines` / executable collection
- [ ] Unit tests with synthetic PE headers
- [ ] Commit

## Task 3: Ply-stream events

- [ ] Add `GameStarted` / `PlyPlayed` / `GameFinished` to `TournamentEvent` (or match-level progress callback)
- [ ] Thread optional progress from `play_game` after each move
- [ ] Extend `LiveTournamentSnapshot` with `live_games: Vec<LiveGameBoard>`
- [ ] Tests for snapshot ply append
- [ ] Commit

## Task 4: Setup + Arena + Results UI

- [ ] `tournament_setup.rs` form wired to `TournamentConfig`/`MatchConfig`
- [ ] `tournament_arena.rs` N-board grid + finished strip + stats panels
- [ ] `tournament_results.rs` modal
- [ ] Wire `main.rs` panes/msgs; native-only roster
- [ ] Tests for setup validation helpers
- [ ] Commit

## Task 5: Dist rebuild

- [ ] `scripts/package-dist-windows.ps1 -Clean`
- [ ] Verify PE arches + UCI smoke on aarch64
- [ ] Commit docs only if needed (dist is gitignored)
