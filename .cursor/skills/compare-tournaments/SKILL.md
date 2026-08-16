---
name: compare-tournaments
description: Compare two Mujrim GUI tournaments for adapter vs native strength, NPS, depth, and time forfeits. Use when the user asks to compare tournaments, evaluate adapter regression/enhancement, review Top N follow-up results, or invoke compare-tournaments.
---

# Compare Mujrim tournaments

Reproduce the same evaluation used after `Mujrim Tournament · Top 14`: reconstruct both events, restrict the baseline to the later field, separate raw scores from time-forfeit inflation, and judge each Mujrim adapter against its upstream native.

## How the user calls this

Any of:

- `/compare-tournaments`
- `use compare-tournaments`
- `compare the latest tournament with the previous one`
- `compare <newer event> with <older event>`

Event arguments may be the UI name (`Mujrim Tournament · Top 14`) or the id (`t-1786846735`).

## Do this every time

1. Run the project script (do not re-invent the parser):

```bash
python3 .cursor/skills/compare-tournaments/scripts/compare_tournaments.py --self-test
python3 .cursor/skills/compare-tournaments/scripts/compare_tournaments.py \
  "<after-name-or-id>" "<before-name-or-id>" \
  --json /tmp/tournament_compare.json
```

Omit both names to default to the two most recent finished events in `~/.local/share/Mujrim/library/mujrim.sqlite3`.

2. Prefer checkpoint jsonl under `~/.local/share/Mujrim/tournaments/<format>/<id>/` when that source has at least as many games as SQLite. SQLite `tournament_games` is often incomplete for the newest event.

3. Map product stems with the script table. Do not guess pairs:

| Adapter | Native |
|---|---|
| Mujrim Elite | Stockfish |
| Mujrim v60 | Reckless |
| Mujrim Akimbo | Akimbo |
| Mujrim Viridithas | Viridithas |
| Mujrim Obsidian | Obsidian |
| Mujrim PlentyChess | PlentyChess |
| Mujrim Lc0 | Lc0 |

4. Always publish **three** tables before a verdict:

- Full later-event standings (raw, flags included)
- Baseline standings **restricted to the later field**
- Later-event standings **excluding `termination == forfeit`**

5. For each adapter report: score-percentage delta, vs-native-field score, direct H2H, NPS, average depth, flag count. A raw rank gain that disappears after stripping flags is not an enhancement.

6. Read binary `modified_ms` from checkpoint headers. Only Mujrim adapters rebuilt after the baseline count as “our” change. Unchanged vendor natives are the control.

7. Event Elo is a field rating, not CCRL 40/15.

8. Write a canvas at `~/.cursor/projects/home-hamdiz-projects-RustroverProjects-mujrim/canvases/<kebab-name>.canvas.tsx` following the Cursor canvas skill: verdict first, then score-percentage chart, adapter delta table, full standings, flag-stripped table, NPS gap, and a short “why” tied to code that actually shipped between the two binary mtimes. Link the canvas in the chat reply.

9. Lead the chat answer with the verdict (enhanced / flat / regressed / mixed) and the one-sentence reason. Do not dump raw jsonl.

## Verdict rules

- **Enhanced**: vs-native score and/or H2H improved after flags are removed, and NPS/depth moved toward the native.
- **Flat**: vs-native score within a few points; headline movement is flags or sample noise.
- **Regressed**: vs-native score dropped, draw rate collapsed, or the adapter started flagging while finished-game quality did not rise.
- **Broken**: near-zero draws plus a finished-game score far below the baseline (search/eval correctness), regardless of flag-inflated wins.

Sample-size warning: a 14-engine single RR at 4 games/pairing is 52 games/engine; a V2-style subset may have ~100. Treat H2H of 4 games as directional only.

## Paths

- Study DB: `~/.local/share/Mujrim/library/mujrim.sqlite3` (open read-only; WAL may be active)
- Checkpoints: `~/.local/share/Mujrim/tournaments/{round-robin,double-round-robin,swiss,knockout}/<id>/*.jsonl`
- Script: `.cursor/skills/compare-tournaments/scripts/compare_tournaments.py`
