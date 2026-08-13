//! Framework-free engine-vs-engine / human-vs-engine turn scheduling.

use types::Color;

use super::engine::PlayerConfig;
use super::game::GameState;
use super::settings::Screen;

pub const DEFAULT_ENGINE_RETRIES: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchAction {
    Idle,
    Think,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinishOutcome {
    Applied,
    Retry,
    Failed,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchSnapshot {
    pub generation: u64,
    pub searching: bool,
    pub retries_left: u8,
    pub game_over: bool,
    pub side: Color,
    pub white_engine: bool,
    pub black_engine: bool,
}

impl MatchSnapshot {
    pub fn from_game(
        generation: u64,
        searching: bool,
        retries_left: u8,
        game: &GameState,
        white: &PlayerConfig,
        black: &PlayerConfig,
    ) -> Self {
        Self {
            generation,
            searching,
            retries_left,
            game_over: game.game_over,
            side: game.board.side_to_move,
            white_engine: !matches!(white, PlayerConfig::Human),
            black_engine: !matches!(black, PlayerConfig::Human),
        }
    }

    pub fn current_is_engine(&self) -> bool {
        match self.side {
            Color::White => self.white_engine,
            Color::Black => self.black_engine,
        }
    }
}

pub fn next_action(snap: &MatchSnapshot) -> MatchAction {
    if snap.game_over {
        MatchAction::Stopped
    } else if snap.searching {
        MatchAction::Idle
    } else if snap.current_is_engine() {
        MatchAction::Think
    } else {
        MatchAction::Idle
    }
}

pub fn begin_search(searching: &mut bool) {
    *searching = true;
}

pub fn bump_generation(generation: &mut u64, searching: &mut bool, retries_left: &mut u8) {
    *generation = generation.wrapping_add(1);
    *searching = false;
    *retries_left = DEFAULT_ENGINE_RETRIES;
}

pub fn finish_search(
    generation: u64,
    live_generation: u64,
    searching: &mut bool,
    retries_left: &mut u8,
    ok: bool,
) -> FinishOutcome {
    *searching = false;
    if generation != live_generation {
        return FinishOutcome::Stale;
    }
    if ok {
        *retries_left = DEFAULT_ENGINE_RETRIES;
        FinishOutcome::Applied
    } else if *retries_left > 0 {
        *retries_left -= 1;
        FinishOutcome::Retry
    } else {
        FinishOutcome::Failed
    }
}

pub fn should_sync_tournament_board(screen: Screen) -> bool {
    matches!(screen, Screen::Tournaments)
}

pub fn should_cancel_ponder(ponder_enabled: bool, ponder_hit: bool) -> bool {
    !ponder_enabled || !ponder_hit
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::Board;

    use crate::app_core::engine::PlayerConfig;
    use crate::app_core::game::GameState;

    fn eve_snap() -> MatchSnapshot {
        MatchSnapshot {
            generation: 1,
            searching: false,
            retries_left: DEFAULT_ENGINE_RETRIES,
            game_over: false,
            side: Color::White,
            white_engine: true,
            black_engine: true,
        }
    }

    #[test]
    fn engine_vs_engine_plays_four_plies_with_a_mock_thinker() {
        types::init();
        let mut board = Board::new();
        let mut snap = eve_snap();
        for ply in 0..4 {
            assert_eq!(next_action(&snap), MatchAction::Think, "ply {ply}");
            begin_search(&mut snap.searching);
            assert_eq!(next_action(&snap), MatchAction::Idle);
            let legal = board.generate_legal_moves();
            let mv = *legal.iter().next().expect("legal move");
            assert_eq!(
                finish_search(
                    snap.generation,
                    snap.generation,
                    &mut snap.searching,
                    &mut snap.retries_left,
                    true
                ),
                FinishOutcome::Applied
            );
            board.make_move(mv);
            snap.side = board.side_to_move;
            snap.game_over = board.is_game_over();
        }
        assert_eq!(board.side_to_move, Color::White);
        assert_eq!(next_action(&snap), MatchAction::Think);
    }

    #[test]
    fn generation_bump_discards_stale_results() {
        let mut snap = eve_snap();
        begin_search(&mut snap.searching);
        bump_generation(
            &mut snap.generation,
            &mut snap.searching,
            &mut snap.retries_left,
        );
        assert_eq!(
            finish_search(
                1,
                snap.generation,
                &mut snap.searching,
                &mut snap.retries_left,
                true
            ),
            FinishOutcome::Stale
        );
        assert!(!snap.searching);
        assert_eq!(next_action(&snap), MatchAction::Think);
    }

    #[test]
    fn game_over_stops_thinking() {
        let mut snap = eve_snap();
        snap.game_over = true;
        assert_eq!(next_action(&snap), MatchAction::Stopped);
    }

    #[test]
    fn human_side_does_not_think() {
        types::init();
        let game = GameState::new(Board::new());
        let snap = MatchSnapshot::from_game(
            0,
            false,
            1,
            &game,
            &PlayerConfig::Human,
            &PlayerConfig::BuiltIn { depth: 8 },
        );
        assert_eq!(next_action(&snap), MatchAction::Idle);
    }

    #[test]
    fn failed_search_retries_once_then_stops() {
        let mut snap = eve_snap();
        begin_search(&mut snap.searching);
        assert_eq!(
            finish_search(1, 1, &mut snap.searching, &mut snap.retries_left, false),
            FinishOutcome::Retry
        );
        begin_search(&mut snap.searching);
        assert_eq!(
            finish_search(1, 1, &mut snap.searching, &mut snap.retries_left, false),
            FinishOutcome::Failed
        );
        assert_eq!(next_action(&snap), MatchAction::Think);
    }

    #[test]
    fn tournament_sync_is_isolated_from_local_play() {
        assert!(!should_sync_tournament_board(Screen::Playing));
        assert!(!should_sync_tournament_board(Screen::Menu));
        assert!(should_sync_tournament_board(Screen::Tournaments));
    }

    #[test]
    fn ponder_is_cancelled_unless_a_hit_is_in_flight() {
        assert!(should_cancel_ponder(false, false));
        assert!(should_cancel_ponder(true, false));
        assert!(!should_cancel_ponder(true, true));
    }
}
