<p align="center">
  <img src="logo.png" alt="KishMat Logo" width="220" />
</p>

<h1 align="center">KishMat</h1>

<p align="center">
  <b>The world's first Egyptian Arab chess engine.</b><br/>
  Written entirely in Rust. NNUE-enhanced, multi-protocol, tournament-ready.
</p>

<p align="center">
  <a href="#features"><img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust" /></a>
  <a href="License.md"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT" /></a>
  <a href="#search"><img src="https://img.shields.io/badge/search-Lazy%20SMP-green?style=flat-square" alt="Lazy SMP" /></a>
  <a href="#protocol"><img src="https://img.shields.io/badge/protocol-UCI%20%7C%20XBoard-lightgrey?style=flat-square" alt="UCI | XBoard" /></a>
  <a href="#evaluation"><img src="https://img.shields.io/badge/eval-NNCorrL%20Hybrid-purple?style=flat-square" alt="NNCorrL" /></a>
  <a href="#gui"><img src="https://img.shields.io/badge/gui-iced-cyan?style=flat-square" alt="iced GUI" /></a>
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> ·
  <a href="#features">Features</a> ·
  <a href="#gui">GUI</a> ·
  <a href="#architecture">Architecture</a> ·
  <a href="#benchmarks">Benchmarks</a> ·
  <a href="#uci-options">UCI Options</a> ·
  <a href="#development">Development</a>
</p>

---

## What's New in v2.0.0

- **Fully Modular Workspace** — Each logical component is its own crate under `crates/`, enabling partial updates and LEGO-like swappability
- **GUI Application** — Native chess GUI built with [iced](https://iced.rs), featuring macOS-style grain textures, high-fidelity image pieces, and support for Human vs Human, Human vs Engine, and Engine vs Engine play
- **Batch Updater** — GitHub-release-based updater (`kishmat-updater`) with per-component updates, progress bars, and SHA256 verification
- **NNCorrL Hybrid Evaluation** — Neural Network Correction Layer (768→64×2→1) with SCReLU activation adds pattern-based corrections to the classical eval
- **Stockfish-Inspired Search Tuning** — Razoring, futility, LMR, LMP, null-move, and correction history calibrated against top engine research
- **XBoard/CECP Protocol** — Full support for WinBoard and XBoard GUIs
- **Opening Book** — Embedded Polyglot gambit-focused opening book
- **Zero-Cost Modularity** — `lto = "fat"` + `codegen-units = 1` ensures full cross-crate inlining in release builds

---

## Quick Start

```bash
git clone https://github.com/theHamdiz/kishmat.git
cd kishmat
```

### Using [just](https://just.systems) (recommended)

```bash
just release     # Build optimized engine binary
just ui          # Build and launch the GUI
just test        # Run all tests
just bench       # Run ELO benchmark suite
```

### Using cargo directly

```bash
# Build the engine
RUSTFLAGS="-C target-cpu=native" cargo build --release

# Run in UCI mode (connects to any UCI-compatible GUI)
./target/release/kishmat uci

# Run in XBoard/CECP mode
./target/release/kishmat xboard

# Play interactively from the terminal
./target/release/kishmat play -d 8

# Launch the GUI
cargo run --release -p kishmat-ui

# Run the benchmark suite
./target/release/kishmat bench
```

---

## GUI

KishMat includes a native chess GUI built with [iced](https://iced.rs):

```bash
just ui
```

### Features

- **Three game modes**: Human vs Human, Human vs Engine, Engine vs Engine
- **Load any UCI engine**: Use the built-in KishMat engine or load external UCI engines via file picker
- **Premium design**: macOS-style subtle grain texture backgrounds, high-fidelity colored Staunton pieces
- **Interactive board**: Click to select pieces and make moves, with legal move highlighting and last-move indicators
- **Move history**: Scrollable algebraic notation panel
- **Engine info**: Real-time search depth and evaluation display

### Chess Pieces

The GUI uses high-fidelity colored Staunton chess pieces. The piece set is CC-BY-SA 3.0 compatible. You can swap pieces by replacing the PNG spritesheets in `crates/kishmat-ui/assets/`.

---

## Features

### Evaluation

| Feature | Description |
|---|---|
| **NNCorrL** | Neural correction: 768→64×2→1 with SCReLU, standalone + hybrid blending |
| **Tapered Eval** | Game-phase interpolation between middlegame and endgame scores |
| **PeSTO PSQT** | Piece-square tables for all pieces across both game phases |
| **Material** | Separate MG/EG piece values |
| **Mobility** | Per-piece mobility bonuses/penalties |
| **King Safety** | Pawn shield quality, pawn storm detection, attacker weights |
| **Threats** | Hanging pieces, pieces attacked by lower-value pieces |
| **Passed Pawns** | Rank-based scaling with king proximity bonuses |
| **Bishop Pair** | +30/+50 MG/EG bonus |
| **Rook on Open File** | Open/semi-open file bonuses |
| **Space Control** | Central square control evaluation |
| **Connected Rooks** | Bonus for connected rooks on rank or file |

### Search

| Technique | Description |
|---|---|
| **Lazy SMP** | Multi-threaded search (default 32 threads) |
| **Iterative Deepening** | Progressive depth with aspiration windows (10cp initial) |
| **PVS** | Principal Variation Search with null-window re-search |
| **Alpha-Beta** | Full-width with fail-soft |
| **Null Move Pruning** | R = 5 + depth/5 + eval correction, verification at depth>12 |
| **Late Move Reductions** | `0.77 + ln(d)·ln(m)/2.36` + history-based stat-score adjustments |
| **Late Move Pruning** | `(3 + depth²) / (2 - improving)` threshold formula |
| **Reverse Futility** | `77·depth - 74·improving` at depth ≤ 8 |
| **Razoring** | Drops to qsearch when `eval ≤ α - 507 - 312·d²` |
| **Futility Pruning** | `77·depth - 46·improving` at depth ≤ 6 |
| **Singular Extensions** | TT move singularity with double/negative extensions |
| **Check Extension** | +1 ply when in check |
| **ProbCut** | Reduced-depth verification for positions way above beta |
| **IIR** | Internal Iterative Reduction at PV nodes |
| **SEE Pruning** | Prune losing captures and quiet moves by SEE score |
| **History Gravity** | `bonus - entry·|bonus|/16384` (capped at ±16384) |
| **Continuation History** | 1-ply and 2-ply back move-piece tracking |
| **Capture History** | Piece-to-square-to-captured-piece scoring |
| **Correction History** | Pawn, material, minor piece, and non-pawn correction tables |
| **Killer Moves** | 2 per ply |
| **Countermove Heuristic** | Refutation move table |
| **Delta Pruning** | In quiescence search |

### Infrastructure

| Component | Description |
|---|---|
| **Transposition Table** | Lock-free, 4-entry buckets, generation-based aging |
| **TT Prefetching** | Cache-line prefetch for next position |
| **Magic Bitboards** | Sliding piece attack generation |
| **Zobrist Hashing** | Thread-safe, incremental |
| **Opening Book** | Embedded Polyglot format, gambit-focused |
| **mimalloc** | Global allocator for reduced fragmentation |
| **target-cpu=native** | AVX2 and other CPU extensions auto-enabled |
| **LTO + codegen-units=1** | Aggressive release profile optimization |

---

## Architecture

KishMat is designed as a **fully modular workspace** — each logical component is its own crate, allowing independent updates and swappability (like LEGO bricks):

```
kishmat/                          # Workspace root + main engine binary
├── src/                          # CLI entry point (UCI, XBoard, play, bench)
├── crates/
│   ├── kishmat-types/            # Board, bitboards, move gen, Zobrist, attack tables
│   ├── kishmat-eval/             # Tapered eval, PeSTO PSQT, mobility, king safety
│   ├── kishmat-search/           # Alpha-beta, Lazy SMP, TT, SEE, NNUE, opening book
│   │   └── src/nnue/             # Neural Network (768→64×2→1, SCReLU)
│   ├── kishmat-comms/            # UCI + XBoard protocol handlers, time management
│   ├── kishmat-tests/            # Integration tests across all engine crates
│   ├── kishmat-ui/               # Native GUI (iced) — chess board, game modes
│   │   └── assets/               # Chess piece spritesheets
│   └── kishmat-updater/          # GitHub-based batch updater binary
├── justfile                      # Build recipes (just ui, just test, etc.)
└── Cargo.toml                    # Workspace definition + release profile
```

**Zero-cost modularity**: All crates use `path` dependencies with `package` renames for ergonomic imports (`use types::*`). Release builds with `lto = "fat"` and `codegen-units = 1` ensure full cross-crate inlining — no performance penalty from the modular design.

---

## Benchmarks

### Bratko-Kopec Test (v2.0.0)

```
╔══════════════════════════════════════════════╗
║                  RESULTS                    ║
╠══════════════════════════════════════════════╣
║  Accuracy:    10/24 ( 41.7%)                ║
║  Est. ELO:    ~1775                          ║
║  NPS:         19.79M (depth 18, startpos)    ║
║  Total nodes: 694.25M                        ║
║  Total time:  25007ms                        ║
╚══════════════════════════════════════════════╝
```

Run the benchmark:

```bash
just bench
```

---

## Protocol Support

### UCI (Universal Chess Interface)

Full compliance. All standard commands implemented:
- `uci`, `isready`, `ucinewgame`, `position`, `go`, `stop`, `quit`
- `setoption`, `debug`, `register`, `ponderhit`
- `go` parameters: `depth`, `movetime`, `wtime`, `btime`, `winc`, `binc`, `movestogo`, `infinite`, `ponder`, `perft`, `nodes`, `mate`
- Non-standard extensions: `d`/`display`, `eval`, `perft`

### XBoard/CECP (Chess Engine Communication Protocol)

Full support for XBoard and WinBoard GUIs:
- `xboard`, `protover`, `new`, `go`, `force`, `quit`
- `usermove`, `setboard`, `time`, `otim`, `level`, `sd`
- `ping`/`pong`, `post`/`nopost`, `result`

---

## UCI Options

| Option | Type | Default | Range | Description |
|---|---|---|---|---|
| `Hash` | spin | 4096 | 1–65536 | TT size in MB |
| `Threads` | spin | 32 | 1–256 | Search threads (Lazy SMP) |
| `MoveOverhead` | spin | 10 | 0–5000 | Time management safety margin (ms) |
| `OwnBook` | check | true | — | Use embedded opening book |
| `UseNNUE` | check | true | — | Enable NNCorrL neural correction |
| `Ponder` | check | false | — | Pondering (accepted, not active) |
| `UCI_AnalyseMode` | check | false | — | Analysis mode flag |
| `UCI_Chess960` | check | false | — | Chess960 support flag |

---

## Updater

KishMat includes a self-updater that downloads updates from GitHub releases:

```bash
just check-updates   # Check for new releases
just update          # Update all components
```

Or run the updater binary directly:

```bash
kishmat-updater check              # Check for updates
kishmat-updater update all         # Update everything
kishmat-updater update kishmat-ui  # Update only the GUI
kishmat-updater list               # List installed components
```

---

## Development

Requires [Rust](https://rustup.rs) and optionally [just](https://just.systems).

```bash
just build      # Debug build (all crates)
just release    # Optimized release build
just test       # Run all tests (16MB stack for deep search)
just lint       # Run clippy
just fmt        # Format code
just check      # Check without building
just ui         # Build and launch the GUI
```

### Project Recipes

| Command | Description |
|---|---|
| `just build` | Debug build for all workspace crates |
| `just release` | Optimized release build (`-C target-cpu=native`) |
| `just test` | Run all tests with 16MB stack |
| `just ui` | Build and launch the chess GUI |
| `just run` | Run engine in UCI mode |
| `just play` | Interactive terminal play |
| `just bench` | Run ELO benchmark suite |
| `just nps` | Quick NPS benchmark (depth 16) |
| `just updater` | Build the updater binary |
| `just check-updates` | Check for GitHub updates |
| `just update` | Update all components |
| `just lint` | Run clippy lints |
| `just fmt` | Format all code |

---

## License

MIT — see [License.md](License.md).

Chess piece assets: CC-BY-SA 3.0 (based on cburnett Staunton set).

## Author

**Ahmad Hamdi Emara** — [contact@hamdiz.me](mailto:contact@hamdiz.me)

<p align="center">
  <sub>كش مات — KishMat.</sub>
</p>