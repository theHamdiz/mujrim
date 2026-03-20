# KishMat Agent Guide

Updated: 2026-03-19

## Repository Rules
- Always check latest dependency versions from the internet and read their docs before implementing updates.
- Every code change must include unit/integration tests.
- Keep code idiomatic and remove AI-sounding wording/comments.
- Do not ignore warnings/errors, even outside the immediate task.
- Stay DRY: reuse existing functionality before adding new code.
- Prefer modular, trait-based designs with no unnecessary runtime overhead.

## Current State
- A structured AI-agent tool surface is implemented in `kishmat-tooling`.
- Entry point: `kishmat-tooling agent`.
- Output contract: JSON (machine-readable for agent orchestration).
- Tool domains are split by responsibility:
  - `engine.*`
  - `gui.*`
  - `tooling.*`
  - `updater.*`

## Agent Tool Commands
- `kishmat-tooling agent list [--pretty]`
- `kishmat-tooling agent describe <tool> [--pretty]`
- `kishmat-tooling agent call <tool> --input '<json-object>' [--pretty]`

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
