# Tournament Studio Design (CuteChess-organized, Mujrim theme)

Approved: 2026-08-12

## Goal

Replace the thin tournament hub with a CuteChess-organized **Tournament Studio** that supports:

- Hybrid live viewing: up to **N concurrent boards** (N = concurrency, capped 1–4) updating **move-by-move**, plus a finished-game strip
- Dedicated **Setup**, **Arena**, and **Results** surfaces (Results opens via button)
- Host-architecture-only engine auto-detection (no emulated binaries)
- Product version **1.0.0** for engine + GUI

Keep the existing Mujrim iced theme / palette / cards — not CuteChess chrome.

## Architecture

### Modules (UI)

| Module | Responsibility |
|--------|----------------|
| `tournament_setup.rs` | CuteChess-like config form + player list |
| `tournament_arena.rs` | Live N-board grid + finished strip + stats panels |
| `tournament_results.rs` | Standings / game table modal opened by button |
| `tournament_live.rs` | Shared snapshot + ply/game state (existing, extended) |
| `main.rs` | Shell: `TournamentPane::{Setup, Arena}` + wire msgs |

### Backend

| Area | Change |
|------|--------|
| `play_game` / `run_match` | Optional ply callback after each legal move |
| `TournamentEvent` | Add `GameStarted`, `PlyPlayed`, `GameFinished` (keep match events) |
| `TournamentConfig` / setup DTO | Expose knobs already in `MatchConfig` + event metadata |
| Engine catalog | Native PE/ELF/Mach-O machine filter; drop Emulated from auto-detect |

## Engine discovery (host-arch only)

1. Read PE `Machine` (Windows), ELF `e_machine`, or Mach-O CPU type for candidate binaries.
2. Accept only host ISA (`aarch64` / `x86_64` as appropriate).
3. Bundled discovery: skip `RuntimeCompatibility::Emulated` candidates.
4. External probe: reject wrong-arch paths before spawn.
5. Unit tests with fixture PE headers / synthetic bytes.

## Live ply stream

Events (conceptual):

```text
Planned → MatchStarted → GameStarted → PlyPlayed* → GameFinished*
         → MatchFinished → … → Cancelled?
```

`PlyPlayed` carries: game id, white/black names, ply index, UCI move, optional score/nps/depth, fen or full move list so far.

Concurrency: `MatchConfig.concurrency` clamped to 1..=4 for UI tournaments. Arena renders one board tile per **in-progress** game (≤ N) and a scrollable finished strip.

## Setup knobs (v1)

- Event name, site (optional)
- Format: Round Robin, Double Round Robin, Swiss, Knockout
- Rounds / Swiss rounds, games per encounter (`pairs`), swap sides (default on)
- Time control mode: nodes / move-time / depth (map to existing `MatchConfig`)
- Concurrency 1–4, hash, threads, memory caps, max plies
- Player list: select from native-only catalog; add/remove/reorder; basic configure (name, path display, hash/threads overrides later if cheap)
- PGN output path optional (write after finish if set)

Deferred (not v1): Gauntlet/Pyramid, Polyglot book UI, EPD suite browser, full UCI option editor, tablebase adjudication UI.

## Arena layout (Mujrim theme)

```
[ Results ] [ Setup ] [ Cancel ]
┌─ Live boards (1..N) ─────────────────────────────┐
│ board tile │ board tile │ …                      │
│ names, last info, mini move list                 │
└──────────────────────────────────────────────────┘
┌─ Focused stats ──────────────────────────────────┐
│ White eval panel │ Black eval panel │ Eval hist  │
└──────────────────────────────────────────────────┘
┌─ Finished games strip (click → focus/replay) ────┐
└──────────────────────────────────────────────────┘
```

## Results panel

Modal/card opened by **Results** button: standings table + game results list; selecting a game focuses replay on Arena (or embedded board).

## Version

Workspace crates and user-facing `id name` / banners: **1.0.0**.

## Testing

- PE machine filter unit tests
- Ply event emission unit test (mock/minimal game or recorded moves)
- Tournament live snapshot append/update tests
- Setup DTO validation tests
- Dist rebuild after merge of feature work

## Non-goals (v1)

- Separate OS windows per panel
- Exact CuteChess pixel clone
- Emulated x86_64 engines on Arm64 hosts
