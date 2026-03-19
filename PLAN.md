# KishMat → 3000+ Elo: Master Plan

> Combining the best of **Stockfish 18**, **Viridithas 15**, and **Akimbo 1.0** to transform KishMat from ~1963 Elo into a 3000+ superhuman engine.

---

## 1. Engine Overview & Ratings

| Engine            | Language | CCRL 40/15 | CCRL Blitz | NNUE                                                | Key Strength                                                           |
| ----------------- | -------- | ---------- | ---------- | --------------------------------------------------- | ---------------------------------------------------------------------- |
| **Stockfish 18**  | C++      | ~3700+     | ~3800+     | SFNNv10 (HalfKAv2_hm + FullThreats)                 | Gold standard — deepest search, best NNUE, correction history          |
| **Viridithas 15** | Rust     | 3572       | 3690       | Custom NNUE via `bullet` trainer                    | Strongest Rust engine — 5 correction histories, hindsight ext/red      |
| **Akimbo 1.0**    | Rust     | 3477       | 3579       | Custom NNUE via `bullet` trainer                    | Lean codebase — compact but deadly, node-based time management         |
| **KishMat 2.0**   | Rust     | ~1963\*    | —          | Akimbo-compatible 768→1024×2→1 NNUE + Classical HCE | Fast NPS, comprehensive search, needs NNUE retraining & staged movegen |

_\*Estimated from Bratko-Kopec 13/24 (54.2%) — depth 16, 120s/position._

---

## 2. Implemented Features (✅ Done)

These were previously listed as gaps but are now implemented in KishMat 2.0:

| Feature                                              | Status | Notes                                           |
| ---------------------------------------------------- | ------ | ----------------------------------------------- |
| NNUE (768→1024×2→1, SCReLU, king buckets)            | ✅     | Akimbo-compatible, embedded `net.bin`           |
| NNUE eval caching (board-state comparison)           | ✅     | Avoids full reinit every eval call              |
| Correction history (3 tables: pawn, material, minor) | ✅     | Weighted EMA updates                            |
| Continuation history (2-ply deep)                    | ✅     | `cont_hist[piece][to]`                          |
| Capture history                                      | ✅     | `cap_hist[piece][to][captured]`                 |
| IIR (Internal Iterative Reduction)                   | ✅     | `depth -= 1` when no TT move                    |
| PV line tracking (triangular table)                  | ✅     | Full PV propagation                             |
| Singular extensions (double + negative)              | ✅     | Per-path `dbl_exts < 5` counter (Akimbo-style)  |
| History pruning                                      | ✅     | Low-depth quiet move pruning                    |
| Stat_score-based LMR (7+ factors)                    | ✅     | PV/improving/killer/check/corr/cut/ttpv         |
| LMR for losing captures                              | ✅     | SEE < 0 captures get reduced                    |
| Mate distance pruning                                | ✅     | α/β bounds clamped                              |
| Countermove heuristic                                | ✅     | `countermoves[from][to]`                        |
| Best-thread selection (Lazy SMP)                     | ✅     | Score+depth based                               |
| Node-based time management                           | ✅     | Best-move node fraction                         |
| Score trend TM                                       | ✅     | Drop → more time                                |
| Best-move stability TM                               | ✅     | Changes → more time                             |
| Aspiration windows with eval/divisor                 | ✅     | Dynamic narrowing                               |
| TT was_pv flag                                       | ✅     | Used in pruning + LMR                           |
| ProbCut with eval guard                              | ✅     | Captures only                                   |
| NMP with anti-recursion                              | ✅     | `min_nmp_ply` guard (Akimbo-style)              |
| RFP with tt_was_pv and tt_capture guards             | ✅     | Complex conditions                              |
| TT-based static eval refinement                      | ✅     | Stockfish technique                             |
| Capture history in qsearch ordering                  | ✅     | MVV-LVA + cap_hist                              |
| Benchmarker crate                                    | ✅     | Standalone `kishmat-benchmarker` with CLI + TUI |

---

## 3. Remaining Gaps (Still Missing)

### 3.1 Search: Staged Move Generation — est. +30-50 Elo

**Reference**: Stockfish (staged MovePicker), Viridithas (staged `MovePicker` with SEE-filtering per stage)

| Feature         | Stockfish 18                        | Viridithas 15             | Akimbo 1.0            | KishMat 2.0                        |
| --------------- | ----------------------------------- | ------------------------- | --------------------- | ---------------------------------- |
| **Move Picker** | Staged (TT→captures→killers→quiets) | Staged with SEE-filtering | Incremental pick-best | Generate-all, score-all, pick-best |

**Recommendation**: Implement a 6-stage `MovePicker`:

1. TT move (no generation)
2. Generate captures → good captures (SEE ≥ 0) by MVV-LVA + cap_hist
3. Killer moves
4. Countermove
5. Generate quiets → by stat_score
6. Bad captures (SEE < 0)

---

### 3.2 Syzygy Tablebases — est. +20-30 Elo

**Reference**: Stockfish (Fathom), Viridithas (Pyrrhic)

**Recommendation**: Integrate `pyrrhic-rs` crate, implement `EngineAdapter` trait for KishMat's `Board`. Download 3-4-5 piece tables (~1GB). Probe WDL in search, DTZ at root.

---

### 3.3 GPU/NPU Auto-Detection — Infrastructure for Training

**Reference**: `bullet` trainer uses CUDA/HIP backends. KishMat runs on Apple M2 Pro with Metal 4.

| Platform            | GPU Backend | NPU              | Status    |
| ------------------- | ----------- | ---------------- | --------- |
| macOS Apple Silicon | Metal       | ANE (via CoreML) | Primary   |
| Linux NVIDIA        | CUDA        | —                | Secondary |
| Linux AMD           | HIP/ROCm    | —                | Stub      |
| Other               | CPU SIMD    | —                | Fallback  |

**Recommendation**: New `kishmat-gpu` crate with auto-detection (`detect.rs`), Metal compute backend (`metal_backend.rs`), CUDA stub, and CPU fallback.

---

### 3.4 Bullet NNUE Trainer — est. +200-400 Elo (with retraining)

**Reference**: Both Akimbo and Viridithas train with jw1912's `bullet` (Rust+CUDA).

**Recommendation**: New `kishmat-trainer` crate:

- `datagen.rs`: Self-play data generation in `bulletformat`
- `trainer.rs`: Training orchestration via `bullet_lib`
- CLI: `kishmat train datagen/train/bench` subcommands
- Architecture: Keep 768→1024×2→1 (Akimbo-compatible)

---

### 3.5 SPRT Parameter Tuning — est. +50-100 Elo

**Reference**: All three engines use OpenBench-style SPRT testing. Viridithas has 80+ tunable parameters.

**Recommendation**:

- Extract search constants into tunable `SearchParams` struct
- UCI `setoption` for runtime parameter overrides
- `sprt/tune.sh` script: builds base+candidate, runs `cutechess-cli` SPRT
- `sprt/params.toml`: parameter ranges and defaults

---

### 3.6 Additional Refinements from Reference Engines

| Feature                             | Source                                     | Est. Elo | Priority |
| ----------------------------------- | ------------------------------------------ | -------- | -------- |
| **Cuckoo repetition detection**     | Viridithas: `cuckoo.rs`                    | +5-10    | Medium   |
| **Hindsight extensions/reductions** | Viridithas: prev_reduction + eval_sum      | +5-10    | Medium   |
| **4-ply continuation history**      | Stockfish, Viridithas                      | +10-15   | Medium   |
| **NonPawn correction history**      | Stockfish, Viridithas (weight 1887)        | +5-8     | Low      |
| **Continuation correction history** | Stockfish, Viridithas (weight 1942)        | +3-5     | Low      |
| **Corrplexity time management**     | Stockfish: spend more on complex positions | +3-5     | Low      |
| **Draw score randomization**        | Viridithas: avoids draw blindness          | +2-3     | Low      |

---

## 4. Execution Priority (By ROI)

| Priority | Feature                    | Elo Gain | Effort                                |
| -------- | -------------------------- | -------- | ------------------------------------- |
| 🥇       | **Staged Move Generation** | +30-50   | Medium — pure code                    |
| 🥈       | **Syzygy Tablebases**      | +20-30   | Medium — crate integration + download |
| 🥉       | **GPU/NPU Auto-Detection** | Infra    | Medium — new crate                    |
| 4        | **Bullet NNUE Trainer**    | +200-400 | High — new crate + training pipeline  |
| 5        | **SPRT Parameter Tuning**  | +50-100  | Medium — scripts + param extraction   |
| 6        | **Additional Refinements** | +30-50   | Low each                              |

> **Total remaining gap: ~335-640 Elo** from code improvements.
> With NNUE retraining: **+535-1040 Elo → target 2500-3000+ Elo**.

---

## 5. References

| Resource                 | URL                                             |
| ------------------------ | ----------------------------------------------- |
| Viridithas source        | https://github.com/cosmobobak/viridithas        |
| Akimbo source            | https://github.com/jw1912/akimbo                |
| Stockfish source         | https://github.com/official-stockfish/Stockfish |
| bullet NNUE trainer      | https://github.com/jw1912/bullet                |
| pyrrhic-rs (Syzygy)      | https://crates.io/crates/pyrrhic-rs             |
| cutechess-cli            | https://github.com/cutechess/cutechess          |
| OpenBench (SPRT testing) | https://github.com/AndyGrant/OpenBench          |
| Chess Programming Wiki   | https://www.chessprogramming.org                |

---

_Plan updated: 2026-03-19. Based on source code analysis of all four engines._

_KishMat: كش مات — The Arabian chess engine that will reach 3000+._
