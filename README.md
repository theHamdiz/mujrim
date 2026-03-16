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
  <a href="#allocator"><img src="https://img.shields.io/badge/allocator-mimalloc-red?style=flat-square" alt="mimalloc" /></a>
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> ·
  <a href="#features">Features</a> ·
  <a href="#architecture">Architecture</a> ·
  <a href="#benchmarks">Benchmarks</a> ·
  <a href="#uci-options">UCI Options</a> ·
  <a href="#development">Development</a>
</p>

---

## What's New in v2.0.0

- **NNCorrL Hybrid Evaluation** — KishMat-original Neural Network Correction Layer (768→32x2→1) adds pattern-based corrections to the classical eval
- **Stockfish-Inspired Search Tuning** — Razoring, futility, LMR, LMP, and null-move parameters calibrated against top engine research
- **XBoard/CECP Protocol** — Full support for WinBoard and XBoard GUIs
- **100% UCI Compliance** — All standard UCI commands implemented: `debug`, `register`, `ponderhit`, `setoption`, plus standard options
- **Enhanced History Gravity** — Stockfish-style gravity formula: `entry += bonus - entry * |bonus| / 16384`
- **Opening Book** — Embedded Polyglot gambit-focused opening book

---

## Quick Start

```bash
git clone https://github.com/theHamdiz/kishmat.git
cd kishmat
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

Run in UCI mode (connects to any UCI-compatible GUI):

```bash
./target/release/kishmat uci
```

Run in XBoard/CECP mode (for WinBoard, XBoard):

```bash
./target/release/kishmat xboard
```

Play interactively from the terminal:

```bash
./target/release/kishmat play -d 8
```

Run the benchmark suite:

```bash
./target/release/kishmat bench
```

---

## Features

### Evaluation

| Feature | Description |
|---|---|
| **NNCorrL** | KishMat-original neural correction: 768→32x2→1 with clipped ReLU, bounded ±300cp |
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
| **Singular Extensions** | TT move singularity with `β = ttScore - 3·depth` |
| **Check Extension** | +1 ply when in check |
| **ProbCut** | Reduced-depth verification for positions way above beta |
| **IID** | Internal Iterative Deepening at PV nodes |
| **SEE Pruning** | Prune losing captures and quiet moves by SEE score |
| **History Gravity** | `bonus - entry·|bonus|/16384` (capped at ±16384) |
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

```
kishmat/           # Root crate: CLI, entry point, mimalloc
├── arbiter/       # Thin wrapper: engine → move API
├── comms/         # UCI + XBoard protocol handlers, time management
├── eval/          # Tapered eval, PeSTO PSQT, mobility, king safety, threats
├── search/        # Alpha-beta, Lazy SMP, TT, SEE, NNCorrL, opening book
│   └── nnue/      # Neural Network Correction Layer (768→32x2→1)
└── types/         # Board, bitboards, move gen, Zobrist, attack tables
```

All crates are workspace members. The search crate shares its TT
across threads via `Arc` for Lazy SMP.

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
kishmat bench -d 18
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

## Development

Requires [Rust](https://rustup.rs).

```bash
cargo build --release          # Optimized release build
cargo test --workspace         # Run all tests
cargo run --release -- bench   # Run ELO benchmark suite
cargo run --release -- play    # Interactive play
cargo run --release -- perft   # Perft test
```

### Running Tests

```bash
RUST_MIN_STACK=8388608 cargo test --workspace
```

The `RUST_MIN_STACK` is needed for the deep search recursion in debug mode.

---

## License

MIT — see [License.md](License.md).

## Author

**Ahmad Hamdi Emara** — [contact@hamdiz.me](mailto:contact@hamdiz.me)

<p align="center">
  <sub>كش مات — KishMat.</sub>
</p>