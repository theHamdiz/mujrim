//! Arena helpers for hybrid live tournament boards.

use super::layout::LIVE_BOARD_SLOTS;
use super::tournament_live::{LiveGameBoard, PlayedGame};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArenaSlotPhase {
    Waiting,
    Live,
    Settled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArenaSlot {
    pub phase: ArenaSlotPhase,
    pub game: Option<LiveGameBoard>,
}

impl ArenaSlot {
    pub fn waiting() -> Self {
        Self {
            phase: ArenaSlotPhase::Waiting,
            game: None,
        }
    }

    pub fn game_key(&self) -> Option<&str> {
        self.game.as_ref().map(|game| game.game_key.as_str())
    }

    pub fn paint_token(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.phase.hash(&mut hasher);
        if let Some(game) = &self.game {
            game.game_key.hash(&mut hasher);
            game.moves.len().hash(&mut hasher);
            game.last_uci.hash(&mut hasher);
            game.position_fen.hash(&mut hasher);
        }
        hasher.finish()
    }
}

/// Cap concurrent live boards shown in the arena grid.
pub fn visible_live_boards(live: &[LiveGameBoard], concurrency: usize) -> Vec<LiveGameBoard> {
    let limit = concurrency.clamp(1, LIVE_BOARD_SLOTS);
    live.iter()
        .filter(|game| !game.is_placeholder())
        .take(limit)
        .cloned()
        .collect()
}

pub fn arena_slot_count(concurrency: u32) -> usize {
    (concurrency as usize).clamp(1, LIVE_BOARD_SLOTS)
}

pub fn grid_columns(slot_count: usize) -> usize {
    match slot_count {
        0 | 1 => 1,
        2 => 2,
        3 | 4 => 2,
        5 | 6 => 3,
        7..=9 => 3,
        _ => 4,
    }
}

pub fn grid_rows(slot_count: usize) -> usize {
    let count = slot_count.max(1);
    count.div_ceil(grid_columns(count))
}

/// Keep each in-progress game on a fixed tile so the grid never remounts
/// or collapses when pairings finish and the next ones start.
pub fn stable_arena_slots(
    live: &[LiveGameBoard],
    concurrency: usize,
    previous: &[ArenaSlot],
) -> Vec<ArenaSlot> {
    let count = concurrency.clamp(1, LIVE_BOARD_SLOTS);
    let live: Vec<&LiveGameBoard> = live.iter().filter(|game| !game.is_placeholder()).collect();
    let mut slots: Vec<ArenaSlot> = previous.iter().take(count).cloned().collect();
    while slots.len() < count {
        slots.push(ArenaSlot::waiting());
    }

    let mut assigned = std::collections::HashSet::new();
    for slot in &mut slots {
        let Some(key) = slot.game_key().map(str::to_owned) else {
            continue;
        };
        if let Some(fresh) = live.iter().copied().find(|game| game.game_key == key) {
            slot.phase = ArenaSlotPhase::Live;
            slot.game = Some(fresh.clone());
            assigned.insert(key);
        } else if slot.phase == ArenaSlotPhase::Live {
            slot.phase = ArenaSlotPhase::Settled;
        }
    }

    for game in live {
        if assigned.contains(&game.game_key) {
            continue;
        }
        let target = slots
            .iter()
            .position(|slot| slot.phase == ArenaSlotPhase::Waiting)
            .or_else(|| {
                slots
                    .iter()
                    .enumerate()
                    .filter(|(_, slot)| slot.phase == ArenaSlotPhase::Settled)
                    .min_by_key(|(_, slot)| slot.game.as_ref().map(|game| game.moves.len()))
                    .map(|(index, _)| index)
            });
        let Some(index) = target else {
            continue;
        };
        slots[index] = ArenaSlot {
            phase: ArenaSlotPhase::Live,
            game: Some(game.clone()),
        };
        assigned.insert(game.game_key.clone());
    }
    slots
}

pub fn finished_strip(games: &[PlayedGame], limit: usize) -> Vec<PlayedGame> {
    games.iter().rev().take(limit).cloned().collect()
}

pub fn score_text(score_cp: i32) -> String {
    format!("{:+.2}", score_cp as f32 / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(key: &str) -> LiveGameBoard {
        LiveGameBoard {
            game_key: key.into(),
            match_index: 1,
            round: 1,
            white: "A".into(),
            black: "B".into(),
            initial_fen: String::new(),
            position_fen: String::new(),
            moves: Vec::new(),
            last_uci: String::new(),
            score_cp: 0,
            depth: 0,
            nodes: 0,
            white_clock_ms: None,
            black_clock_ms: None,
            ..LiveGameBoard::default()
        }
    }

    #[test]
    fn visible_live_boards_keeps_insertion_order() {
        let boards = (0..6)
            .map(|index| board(&format!("g{index}")))
            .collect::<Vec<_>>();
        let visible = visible_live_boards(&boards, 2);
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].game_key, "g0");
        assert_eq!(visible[1].game_key, "g1");
    }

    #[test]
    fn visible_live_boards_skips_placeholders() {
        let boards = vec![
            LiveGameBoard {
                game_key: "pending-0".into(),
                ..LiveGameBoard::default()
            },
            board("g1"),
        ];
        let visible = visible_live_boards(&boards, 8);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].game_key, "g1");
    }

    #[test]
    fn grid_shape_covers_common_concurrency() {
        assert_eq!(grid_columns(1), 1);
        assert_eq!(grid_rows(1), 1);
        assert_eq!(grid_columns(2), 2);
        assert_eq!(grid_rows(2), 1);
        assert_eq!((grid_columns(4), grid_rows(4)), (2, 2));
        assert_eq!((grid_columns(6), grid_rows(6)), (3, 2));
        assert_eq!((grid_columns(8), grid_rows(8)), (3, 3));
        assert_eq!((grid_columns(16), grid_rows(16)), (4, 4));
        assert_eq!(arena_slot_count(0), 1);
        assert_eq!(arena_slot_count(32), LIVE_BOARD_SLOTS);
    }

    #[test]
    fn stable_slots_keep_game_keys_when_a_pairing_finishes() {
        let first = stable_arena_slots(&[board("g-a"), board("g-b"), board("g-c")], 3, &[]);
        assert_eq!(
            first
                .iter()
                .filter_map(ArenaSlot::game_key)
                .collect::<Vec<_>>(),
            ["g-a", "g-b", "g-c"]
        );
        let after = stable_arena_slots(&[board("g-b"), board("g-c")], 3, &first);
        assert_eq!(after[0].phase, ArenaSlotPhase::Settled);
        assert_eq!(after[0].game_key(), Some("g-a"));
        assert_eq!(after[1].phase, ArenaSlotPhase::Live);
        assert_eq!(after[1].game_key(), Some("g-b"));
        assert_eq!(after[2].game_key(), Some("g-c"));
    }

    #[test]
    fn stable_slots_reuse_waiting_then_settled_for_new_games() {
        let held = stable_arena_slots(&[board("g-a")], 2, &[]);
        assert_eq!(held[1].phase, ArenaSlotPhase::Waiting);
        let next = stable_arena_slots(&[board("g-z")], 2, &held);
        assert_eq!(next[0].phase, ArenaSlotPhase::Settled);
        assert_eq!(next[0].game_key(), Some("g-a"));
        assert_eq!(next[1].phase, ArenaSlotPhase::Live);
        assert_eq!(next[1].game_key(), Some("g-z"));
        let replaced = stable_arena_slots(&[board("g-z"), board("g-new")], 2, &next);
        assert_eq!(replaced[0].game_key(), Some("g-new"));
        assert_eq!(replaced[1].game_key(), Some("g-z"));
    }

    #[test]
    fn paint_token_ignores_eval_churn() {
        let mut slot = ArenaSlot {
            phase: ArenaSlotPhase::Live,
            game: Some(board("g1")),
        };
        let before = slot.paint_token();
        if let Some(game) = slot.game.as_mut() {
            game.score_cp = 340;
            game.nodes = 99_000;
        }
        assert_eq!(slot.paint_token(), before);
        if let Some(game) = slot.game.as_mut() {
            game.last_uci = "e2e4".into();
            game.moves.push("e2e4".into());
        }
        assert_ne!(slot.paint_token(), before);
    }
}
