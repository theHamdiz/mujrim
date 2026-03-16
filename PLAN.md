# KishMat → 3000+ Elo: Master Plan

> Combining the best of **Stockfish 18**, **Viridithas 15**, and **Akimbo 1.0** to transform KishMat from ~1775 Elo into a 3000+ superhuman engine.

---

## 1. Engine Overview & Ratings

| Engine | Language | CCRL 40/15 | CCRL Blitz | NNUE | Key Strength |
|--------|----------|-----------|-----------|------|-------------|
| **Stockfish 18** | C++ | ~3700+ | ~3800+ | SFNNv10 (HalfKAv2_hm + FullThreats) | Gold standard — deepest search, best NNUE, correction history |
| **Viridithas 15** | Rust | 3572 | 3690 | Custom NNUE via `bullet` trainer | Strongest Rust engine — 5 correction histories, hindsight ext/red |
| **Akimbo 1.0** | Rust | 3477 | 3579 | Custom NNUE via `bullet` trainer | Lean codebase — compact but deadly, node-based time management |
| **KishMat 2.0** | Rust | ~1775* | — | NNCorrL (768→32x2→1 correction only) | Fast NPS (19.79M), good infra, but weak eval & wasted search |

*\*Estimated from Bratko-Kopec 10/24 (41.7%).*

---

## 2. Feature-by-Feature Comparison

### 2.1 Evaluation

| Feature | Stockfish 18 | Viridithas 15 | Akimbo 1.0 | KishMat 2.0 | Gap |
|---------|-------------|--------------|-----------|------------|-----|
| **NNUE Architecture** | SFNNv10: 768→1536→8→256→32→1 + Threat Inputs | Custom large NNUE (~10MB), perspective nets | Custom NNUE via bullet, embedded binary | **None** — tiny 768→32x2→1 correction layer only | 🔴 **CRITICAL** |
| **NNUE Training Data** | Billions of self-play positions | Self-play only (original datagen) | Self-play + Leela data | N/A — no standalone NNUE | 🔴 **CRITICAL** |
| **NNUE Trainer** | Custom PyTorch (nnue-pytorch) | `bullet` (Rust+CUDA) | `bullet` (Rust+CUDA) | N/A | 🔴 **CRITICAL** |
| **Classic HCE** | Removed (pure NNUE) | Removed (pure NNUE) | Removed (pure NNUE) | PeSTO PSQT + mobility + king safety + threats | 🟡 Good baseline |
| **Tapered Eval** | N/A (NNUE) | N/A (NNUE) | N/A (NNUE) | ✅ MG/EG interpolation | ✅ OK for HCE |

**Verdict:** KishMat's NNCorrL is a ±300cp correction on top of a classical eval. All three reference engines use **standalone NNUE** as their *sole* evaluation — this is the single biggest Elo differentiator (easily **200–500 Elo**).

---

### 2.2 Search: Pruning & Reductions

| Technique | Stockfish 18 | Viridithas 15 | Akimbo 1.0 | KishMat 2.0 | Gap |
|-----------|-------------|--------------|-----------|------------|-----|
| **Null Move Pruning** | R=4+d/3+eval/174, TT-aware, verified | R=4+d/3+eval/174, ban mechanism, TT guards | R=5+d/5+eval/198, verified at d≥17 | R=5+d/5+eval/200, verified d>12 | 🟢 Close |
| **Reverse Futility** | Margin with margin/3 above beta return | 73d, depth<9, complex guards (ttpv, tt_cap) | 94d/improving, depth≤8 | 77d-74*improving, depth≤8 | 🟡 Missing guards |
| **Razoring** | Quadratic with qsearch fallback | 123+295d, alpha<2000 | 407d, depth≤razor_depth | 507+312d², depth≤3 | 🟢 Close |
| **Late Move Reductions** | Base-tuned ln(d)*ln(m)/div + 8 adjustments | 99/260 + history/17017 + 9 multipliers (see/corr/check/cut/ttpv/refutation) | 48/248 + 6 adjustments (pv/check/cutoffs/history) | 0.77+ln(d)*ln(m)/2.36 + 4 adjustments | 🔴 Missing 5+ adjustments |
| **Late Move Pruning** | Complex formula | `(3+d²)/(2-improving)` | `2+d²/(1 or 2)` | `(3+d²)/(2-improving)`, d≤8 | 🟢 Close |
| **Futility Pruning** | Multi-layered | `86+70d` at d<9 | `188+35d²`, d<6 | `77d-46*improving`, d≤6 | 🟢 Close |
| **ProbCut** | Refined with precomputed table | `176+78*improving`, eval/289 guard | `beta+256`, captures only, verified | `beta+200`, captures only | 🟡 Missing eval guard |
| **Singular Extensions** | β = ttScore-3d, double/triple ext, margins | β = ttScore, double ext (margin 13), triple ext (201) | β = ttScore-d, double ext (<5 per line), negative ext | β = ttScore-3d, but **broken implementation** | 🔴 **BROKEN** |
| **Internal Iterative** | Reduction (IIR): depth-1 if no tt_move | IIR: depth -= 1 at depth ≥ iir_depth | IIR: depth -= 1 at depth ≥ 4 | IID: full re-search at PV, depth-2 | 🟡 IID→IIR switch needed |
| **SEE Pruning** | Separate margins for quiet/tactical | -62 quiet, -28 tactical | -64 quiet, -148 capture, depth<7 | -60d quiet, -20d² capture, d≤4 | 🟡 Suboptimal margins |
| **History Pruning** | Complex multi-table | -3186 margin per depth | 1682 margin, depth<6 | ❌ **Missing** | 🔴 **MISSING** |
| **Hindsight Ext/Red** | Not explicitly named | Based on prev reduction & eval sum | ❌ | ❌ | 🟡 Moderate gain |

---

### 2.3 Search: Move Ordering

| Feature | Stockfish 18 | Viridithas 15 | Akimbo 1.0 | KishMat 2.0 | Gap |
|---------|-------------|--------------|-----------|------------|-----|
| **Move Picker** | Staged generation (TT→captures→killers→quiets) | Staged `MovePicker` with SEE-filtering per stage | Incremental pick-best from scored list | Generate-all, score-all, incremental pick | 🔴 No staged generation |
| **History Table** | piece-to + from-to + continuation(×4) + pawn + tactical + capture | piece-to + from-to + cont(×4) + pawn + tactical | main quiet history + cont(×2) with threats | `history[color][from][to]` only | 🔴 **1 table vs 6+** |
| **Continuation History** | 4 plies deep (cont1, cont2, cont4) | 4 plies deep | 2 plies deep, with threat indexing | ❌ **None** | 🔴 **MISSING** |
| **Capture History** | By piece-type and captured piece-type | Tactical history by piece/capture/threats | Implicit via MVV-LVA | None (MVV-LVA only) | 🟡 |
| **Killer Moves** | 1 per ply | 1 per ply | 1 per ply | 2 per ply | 🟢 Fine |
| **Countermove** | Full table | Full table | ❌ | `countermoves[from][to]` | 🟡 Has it, unused in scoring? |

---

### 2.4 Correction History (CorrHist)

| Feature | Stockfish 18 | Viridithas 15 | Akimbo 1.0 | KishMat 2.0 | Gap |
|---------|-------------|--------------|-----------|------------|-----|
| **Pawn CorrHist** | ✅ Weighted | ✅ Weight 1890 | ✅ | ❌ | 🔴 **MISSING** |
| **Major Piece CorrHist** | ✅ | ✅ Weight 1461 | ✅ | ❌ | 🔴 **MISSING** |
| **Minor Piece CorrHist** | ✅ | ✅ Weight 1292 | ❌ | ❌ | 🔴 **MISSING** |
| **NonPawn CorrHist** | ✅ | ✅ Weight 1887 | ❌ | ❌ | 🟡 |
| **Continuation CorrHist** | ✅ | ✅ Weight 1942 | ❌ | ❌ | 🟡 |
| **Eval Adjustment** | Sum of weighted corrections | Sum of weighted corrections | Simple correction | ❌ | 🔴 **MISSING** |
| **Used to adjust LMR** | ✅ (corrplexity) | ✅ (LMR_CORR_MUL) | ❌ | ❌ | 🟡 |

**Verdict:** Correction history is worth **30–50 Elo** and KishMat has zero implementation. This is one of the highest-ROI additions.

---

### 2.5 Time Management

| Feature | Stockfish 18 | Viridithas 15 | Akimbo 1.0 | KishMat 2.0 | Gap |
|---------|-------------|--------------|-----------|------------|-----|
| **Node-Based TM** | ✅ Best-move node fraction | ✅ Best-move stability | ✅ Node fraction × multiplier | ❌ | 🔴 **MISSING** |
| **Best-Move Stability** | Scale time by move changes | Scale time by move changes | `(1.5-frac)*1.35` if d>8 | Binary: 0.5x / 0.75x | 🔴 Very primitive |
| **Score Trend** | ✅ Drop → time up, rise → time down | ✅ | ❌ | ❌ | 🔴 **MISSING** |
| **Corrplexity TM** | ✅ Spend more time on complex positions | ❌ | ❌ | ❌ | 🟡 Advanced |

---

### 2.6 Infrastructure

| Feature | Stockfish 18 | Viridithas 15 | Akimbo 1.0 | KishMat 2.0 | Gap |
|---------|-------------|--------------|-----------|------------|-----|
| **Lazy SMP** | Mature, best-thread selection | Scoped threads, best-thread selection by score/depth | Simple multi-thread | Fixed depth-offset per thread, no best-thread | 🔴 No best-thread |
| **TT Design** | Multi-entry buckets, generation aging | Multi-entry buckets, generation aging | Standard bucket | 4-entry buckets, generation aging | 🟢 OK |
| **Syzygy Tablebases** | ✅ (Fathom) | ✅ (Pyrrhic) | ❌ | ❌ | 🟡 +20-30 Elo |
| **Cuckoo Repetition** | ✅ Upcoming repetition detection | ✅ | ❌ | ❌ | 🟡 +5-10 Elo |
| **Draw Score Randomization** | ❌ | ✅ Avoids draw blindness | ❌ | ❌ | 🟢 Minor |
| **PV Line Tracking** | Full PV via triangular table | Full PV via `PVariation` struct | Full PV line per ply | TT-based (only best move) | 🔴 No PV line |
| **SPRT Tuning** | OpenBench | OpenBench (SweHosting) | OpenBench (SweHosting) | None | 🔴 Missing methodology |
| **Chess960** | ✅ | ✅ | ✅ | Flag only, not tested | 🟡 |

---

## 3. Root Cause Analysis: Why KishMat is ~1775 Elo

| # | Problem | Elo Cost (Est.) | Engines That Solve It |
|---|---------|-----------------|----------------------|
| 1 | **No standalone NNUE** — using a tiny correction layer instead of a proper NNUE eval | 400–600 | All three |
| 2 | **No correction history** — static eval never learns from search results | 30–50 | SF, Viri, Akimbo |
| 3 | **Only 1 history table** — missing continuation, capture, pawn history | 50–80 | SF, Viri, Akimbo |
| 4 | **Broken singular extensions** — searches alternatives one by one instead of excluded-move approach | 20–40 | All three |
| 5 | **No PV line** — can't report or use principal variation for search guidance | 10–20 | All three |
| 6 | **No staged move generation** — generates all moves upfront, wasting cycles on pruned nodes | 30–50 | SF, Viri |
| 7 | **Primitive time management** — binary soft/hard, no node TM or score trends | 20–40 | All three |
| 8 | **LMR too simplistic** — misses 5+ proven adjustment factors | 20–40 | SF, Viri, Akimbo |
| 9 | **No history pruning** — searches pointless quiet moves with terrible history | 10–20 | SF, Viri, Akimbo |
| 10 | **No best-thread selection** in Lazy SMP | 10–15 | SF, Viri |
| 11 | **IID instead of IIR** — expensive re-search instead of simple depth reduction | 5–10 | All three |
| 12 | **No Syzygy tablebases** | 20–30 | SF, Viri |
| 13 | **Qsearch doesn't use NNUE** — falls back to classical eval in qsearch stand-pat | 15–25 | N/A (but major inconsistency) |

**Total theoretical gap: ~640–1020 Elo** — consistent with the observed ~1800 Elo deficit.

---

## 4. The Plan: KishMat → 3000+ Elo

### Phase 1: Foundation Fixes (Target: +200–300 Elo → ~2000–2075)
> Timeline: 1–2 weeks

These are correctness and low-hanging-fruit fixes that remove bugs and wasted search effort.

| # | Task | Source Inspiration | Est. Gain |
|---|------|--------------------|-----------|
| 1.1 | **Fix singular extensions** — implement excluded-move approach: pass `excluded_move` to `search_ab`, skip it during move loop, search alternatives at `(depth-1)/2` with `β = ttScore - margin*depth` | Akimbo, Viridithas | +20–40 |
| 1.2 | **Switch IID → IIR** — replace expensive re-search with `depth -= 1` when no TT move at `depth ≥ 4` | All three | +5–10 |
| 1.3 | **Add PV line tracking** — triangular PV table (`pv[MAX_PLY][MAX_PLY]`), propagate PV moves back up | Akimbo (simple), Viridithas | +10–20 |
| 1.4 | **Use NNUE eval in qsearch** — call the same hybrid eval for stand-pat, not raw classical | Consistency fix | +15–25 |
| 1.5 | **Fix qsearch alpha/beta** — return `best_score` not `alpha`/`beta`, use fail-soft consistently | Akimbo's clean qs | +5–10 |
| 1.6 | **Add mate distance pruning** — `alpha = max(alpha, -MATE + ply)`, `beta = min(beta, MATE - ply - 1)` | All three | +5 |
| 1.7 | **Fix countermove table** — index by `(previous_from, previous_to)`, use in move ordering | Viridithas | +5–10 |

---

### Phase 2: History Revolution (Target: +100–200 Elo → ~2175–2275)
> Timeline: 2–3 weeks

Massively improve move ordering quality to cut the search tree dramatically.

| # | Task | Source Inspiration | Est. Gain |
|---|------|--------------------|-----------|
| 2.1 | **Implement continuation history** — `cont_hist[piece][to_square]` indexed by previous moves (1-ply and 2-ply back minimum) | Akimbo (2-ply), Viridithas (4-ply) | +30–40 |
| 2.2 | **Implement capture/tactical history** — `cap_hist[moved_piece][to_square][captured_piece_type]` | Viridithas, Stockfish | +15–20 |
| 2.3 | **Compute stat_score** — sum of main history + continuation histories, use for LMR adjustments | All three | +10–15 |
| 2.4 | **Add history pruning** — skip quiet moves with terrible history score at low depths (`depth < 6, score < -margin * depth`) | Akimbo: `-1682*d`, Viridithas: `-3186` | +10–20 |
| 2.5 | **Add SEE stat-score integration** — adjust SEE pruning margins based on stat_score | Viridithas | +5–10 |
| 2.6 | **Staged move generation** — generate captures first, then quiets, picking best at each stage without generating all moves upfront | Viridithas's `MovePicker` | +30–50 |
| 2.7 | **Enhance LMR** — add adjustments for: PV node, cut node, improving, gives check, stat_score/history_divisor, TT capture, TT PV | Viridithas (9 factors) | +20–30 |

---

### Phase 3: Correction History (Target: +30–50 Elo → ~2225–2325)
> Timeline: 1–2 weeks

Learn from search results to correct static eval errors dynamically.

| # | Task | Source Inspiration | Est. Gain |
|---|------|--------------------|-----------|
| 3.1 | **Pawn correction history** — hash pawn structure, record `(search_score - static_eval)` diff, apply weighted correction | Stockfish, Viridithas (weight 1890) | +15–20 |
| 3.2 | **Material/major correction history** — hash material configuration | Viridithas (weight 1461) | +8–12 |
| 3.3 | **Minor piece correction history** — hash minor piece positions | Viridithas (weight 1292) | +5–8 |
| 3.4 | **Apply corrections to eval** — `corrected_eval = static_eval + sum(weight_i * corrhist_i) / divisor` | All three | Included above |
| 3.5 | **Use correction in LMR** — higher correction magnitude → reduce less (position is complex) | Viridithas: `LMR_CORR_MUL = 448` | +5–10 |

---

### Phase 4: Advanced Search (Target: +50–100 Elo → ~2325–2425)
> Timeline: 2–3 weeks

Sophisticated pruning, extensions, and reductions that top engines use.

| # | Task | Source Inspiration | Est. Gain |
|---|------|--------------------|-----------|
| 4.1 | **Double extensions** — if singular search score is far below sBeta, extend by 2 (capped per line) | Akimbo: `s_beta - 25`, cap 5 per line | +10–15 |
| 4.2 | **Negative extensions** — if TT move fails singular test AND ttScore ≥ beta, reduce depth by 1 | Akimbo, Viridithas | +5–8 |
| 4.3 | **Hindsight extensions/reductions** — use previous ply's reduction magnitude + eval sum | Viridithas: unique technique | +5–10 |
| 4.4 | **Improve RFP** — add guards: not when ttpv, tt_move is capture, depth considerations | Viridithas: complex conditions | +5–10 |
| 4.5 | **Improve NMP** — add TT-awareness (don't NMP when TT fail-low at beta, or TT fail-high on obvious capture) | Viridithas: 2 TT guards | +5–8 |
| 4.6 | **ProbCut with eval guard** — condition on `can_probcut` from TT depth/score | Akimbo: `can_probcut` flag | +3–5 |
| 4.7 | **Do-deeper search** — if TT entry depth is much lower than current depth, search +1 deeper | Viridithas: `do_deeper_base = 32` | +3–5 |
| 4.8 | **Aspiration narrowing** — use eval/divisor to narrow initial aspiration window: `delta = initial + |eval|/30155` | Viridithas | +3–5 |

---

### Phase 5: Time Management (Target: +20–40 Elo → ~2365–2465)
> Timeline: 1 week

Smart time management = stronger play under time controls.

| # | Task | Source Inspiration | Est. Gain |
|---|------|--------------------|-----------|
| 5.1 | **Node-based TM** — track `nodes_per_root_move[best_move]`, scale soft time by `(1.5 - frac) * 1.35` | Akimbo: `ntable.get(best_move)` | +10–15 |
| 5.2 | **Best-move stability** — increase time when best move changes between iterations, decrease when stable | All three engines | +5–10 |
| 5.3 | **Score trend** — increase time when score drops significantly, decrease when stable or rising | Viridithas | +5–10 |
| 5.4 | **Implement `SearchLimit` enum** — `Infinite`, `Time { soft, hard }`, `Depth(i32)`, `Nodes(u64)` | Viridithas | +0 (cleanup) |

---

### Phase 6: NNUE Evaluation (Target: +400–600 Elo → ~2765–3065)
> Timeline: 4–8 weeks (this is the hardest and highest-impact phase)

Replace the classical eval entirely with a proper NNUE.

| # | Task | Source Inspiration | Est. Gain |
|---|------|--------------------|-----------|
| 6.1 | **NNUE Architecture** — implement HalfKP/HalfKAv2 feature extraction (768 or 768×2 inputs), perspective nets, efficient incremental update via accumulator stack | Viridithas, Stockfish | — |
| 6.2 | **Quantized inference** — i16/i8 operations using SIMD (AVX2/SSE/NEON), ClippedReLU activations | All three | — |
| 6.3 | **Network size** — start with 768→256×2→32→32→1 (similar to modern engines), ~5–10MB binary | Viridithas, Akimbo | — |
| 6.4 | **Data generation** — self-play with current engine (Phase 1–5 improved version), generate millions of positions with search scores | Viridithas: original datagen code | — |
| 6.5 | **Train with `bullet`** — use `jw1912/bullet` (Rust+CUDA trainer), train iteratively with self-play data | Akimbo, Viridithas (since v11) | — |
| 6.6 | **Embed network** — compile NNUE weights into the binary via `include_bytes!` or `.zst` + decompression at startup | Both Rust engines | — |
| 6.7 | **Remove classical eval dependency** — once NNUE is trained, remove HCE and use pure NNUE + correction histories | All three | — |
| 6.8 | **Iterative improvement** — generate data → train → test → repeat. Each generation should be ~10–20 Elo stronger | Standard practice | — |
| | **Combined NNUE gain** | | **+400–600** |

---

### Phase 7: Infrastructure & Polish (Target: +30–50 Elo → ~2800–3115)
> Timeline: 2–3 weeks

| # | Task | Source Inspiration | Est. Gain |
|---|------|--------------------|-----------|
| 7.1 | **Best-thread selection** — after Lazy SMP completes, pick the thread with best score at highest completed depth | Viridithas: `select_best()` | +10–15 |
| 7.2 | **Syzygy tablebase probing** — integrate Pyrrhic (Rust bindings to Fathom) for ≤6/7 piece endgames | Viridithas: `pyrrhic` dep | +20–30 |
| 7.3 | **Cuckoo filter** — upcoming repetition detection to avoid walking into draws | Viridithas: `cuckoo.rs` | +5–10 |
| 7.4 | **TT refinement** — store `was_pv` flag, use ttpv in pruning decisions and LMR | Viridithas, Stockfish | +5–10 |
| 7.5 | **SPRT testing framework** — set up automated self-play testing (cutechess-cli + OpenBench style) | Industry standard | Methodology |
| 7.6 | **Parameter tuning** — use SPRT to tune all search parameters (80+ constants like Viridithas) | Viridithas: all params are tunable | +20–40 |

---

## 5. Prioritized Ordering (By ROI)

| Priority | Phase | Elo Gain | Effort | ROI |
|----------|-------|----------|--------|-----|
| 🥇 | **Phase 1: Foundation Fixes** | +200–300 | Low (1–2 weeks) | ⭐⭐⭐⭐⭐ |
| 🥈 | **Phase 2: History Revolution** | +100–200 | Medium (2–3 weeks) | ⭐⭐⭐⭐ |
| 🥉 | **Phase 3: Correction History** | +30–50 | Low (1–2 weeks) | ⭐⭐⭐⭐ |
| 4 | **Phase 4: Advanced Search** | +50–100 | Medium (2–3 weeks) | ⭐⭐⭐ |
| 5 | **Phase 5: Time Management** | +20–40 | Low (1 week) | ⭐⭐⭐ |
| 6 | **Phase 6: NNUE** | +400–600 | High (4–8 weeks) | ⭐⭐⭐⭐⭐ |
| 7 | **Phase 7: Infrastructure** | +30–50 | Medium (2–3 weeks) | ⭐⭐⭐ |

> **Total projected gain: ~830–1340 Elo → KishMat at 2600–3115 Elo**
>
> With NNUE and iterative refinement, **3000+ is achievable**.

---

## 6. Key Code Patterns to Adopt

### From Viridithas (the #1 Rust engine):

```rust
// Correction history: 5 parallel tables, weighted combination
let correction = (pawn_corr * PAWN_WEIGHT
    + major_corr * MAJOR_WEIGHT
    + minor_corr * MINOR_WEIGHT
    + nonpawn_corr * NONPAWN_WEIGHT
    + cont_corr * CONT_WEIGHT) / DIVISOR;

// LMR with 9+ adjustment factors
let mut r = lmr_table[depth][move_count];
r += i32::from(!is_pv) * LMR_NON_PV_MUL;
r -= i32::from(is_ttpv) * LMR_TTPV_MUL;
r += i32::from(is_cut_node) * LMR_CUT_NODE_MUL;
r -= i32::from(!improving) * LMR_NON_IMPROVING_MUL;
r -= stat_score / HISTORY_LMR_DIVISOR;
r -= i32::from(gives_check) * LMR_CHECK_MUL;
r -= correction.abs() * LMR_CORR_MUL;
// ... and more

// Hindsight extensions (unique to Viridithas)
if prev_reduction >= HINDSIGHT_EXT_DEPTH
    && static_eval + prev_static_eval < 0 {
    depth += 1;
}
```

### From Akimbo (lean but deadly):

```rust
// Correction history update
fn update_correction_history(&mut self, pos: &Position, depth: i32,
                              search_score: i32, static_eval: i32) {
    let error = search_score - static_eval;
    let weight = depth.min(16);
    // Exponential moving average
    entry = (entry * (256 - weight) + error * 256 * weight) / 256;
}

// Node-based time management
let frac = ntable.get(best_move) as f64 / total_nodes as f64;
let multiplier = if depth > 8 { (1.5 - frac) * 1.35 } else { 1.0 };
if time >= soft_bound * multiplier { stop(); }

// Double singular extensions (capped)
if s_score < s_beta - 25 && dbl_exts < 5 {
    extend += 1;
    dbl_exts += 1;
}
```

### From Stockfish (the absolute king):

```rust
// Multi-correction history combination
let corr = (pawn_corr_val * PAWN_WEIGHT
    + material_corr_val * MATERIAL_WEIGHT
    + minor_corr_val * MINOR_WEIGHT
    + nonpawn_corr_val * NONPAWN_WEIGHT
    + cont_corr_val * CONTINUATION_WEIGHT) / WEIGHT_SUM;
let eval = static_eval + corr;

// Corrplexity: uncertainty-aware time management
let complexity = correction.abs();
let time_multiplier = base + complexity / DIVISOR;

// NMP with TT guards
if !(tt_bound == UPPER && tt_value < beta)    // don't NMP if TT says pos is bad
    && !(tt_bound == LOWER && see(tt_move) > 2*PAWN)  // don't NMP if obvious capture
```

---

## 7. Verification Plan

| Milestone | Test | Target |
|-----------|------|--------|
| Phase 1 complete | BK test, 1000-game self-play vs v2.0.0 | BK ≥ 15/24, +50 Elo minimum |
| Phase 2 complete | 1000-game self-play vs Phase 1 | +80 Elo minimum, NPS within 10% |
| Phase 3 complete | 500-game self-play vs Phase 2 | +25 Elo minimum |
| Phase 4 complete | 1000-game self-play vs Phase 3 | +40 Elo minimum |
| Phase 5 complete | 500-game STC match vs Phase 4 | +15 Elo minimum under time control |
| Phase 6 complete | 1000-game self-play + CCRL submission | +300 Elo minimum, target 2800+ |
| Phase 7 complete | CCRL submission, tournament play | Target 3000+ |

**Testing tool:** `cutechess-cli` with SPRT bounds `[-5, 5]` at 95% confidence for minor patches, `[0, 10]` for major features.

---

## 8. References

| Resource | URL |
|----------|-----|
| Viridithas source | https://github.com/cosmobobak/viridithas |
| Akimbo source | https://github.com/jw1912/akimbo |
| Stockfish source | https://github.com/official-stockfish/Stockfish |
| bullet NNUE trainer | https://github.com/jw1912/bullet |
| Pyrrhic (Syzygy Rust) | https://github.com/jdart1/Fathom |
| Chess Programming Wiki | https://www.chessprogramming.org |
| OpenBench (SPRT testing) | https://github.com/AndyGrant/OpenBench |
| cutechess-cli | https://github.com/cutechess/cutechess |

---

*Plan authored: 2026-03-16. Based on source code analysis of all four engines.*

*KishMat: كش مات — The Arabian chess engine that will reach 3000+.*
