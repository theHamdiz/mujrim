use bevy::prelude::*;

/// Top-level application state.
#[derive(States, Clone, Eq, PartialEq, Debug, Hash, Default)]
pub enum AppState {
    #[default]
    Menu,
    Playing,
    GameOver,
}

/// In-game turn state.
#[derive(States, Clone, Eq, PartialEq, Debug, Hash, Default)]
pub enum TurnState {
    #[default]
    PlayerTurn,
    EngineTurn,
    Idle,
}

/// Render dimension mode.
#[derive(Resource, Clone, Copy, Eq, PartialEq, Debug, Hash, Default)]
pub enum RenderDimension {
    #[default]
    TwoD,
    ThreeD,
}

/// Bridge between kismat-types Board and Bevy ECS.
#[derive(Resource)]
pub struct ChessGame {
    pub board: types::Board,
    pub move_history: Vec<types::Move>,
    pub selected_square: Option<types::Square>,
    pub legal_moves: Vec<types::Move>,
    pub last_move: Option<types::Move>,
    pub flipped: bool,
    pub player_color: types::Color,
    pub game_result: Option<GameResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameResult {
    WhiteWins,
    BlackWins,
    Draw,
}

impl ChessGame {
    pub fn new(player_color: types::Color) -> Self {
        Self {
            board: types::Board::new(),
            move_history: Vec::new(),
            selected_square: None,
            legal_moves: Vec::new(),
            last_move: None,
            flipped: player_color == types::Color::Black,
            player_color,
            game_result: None,
        }
    }

    /// Select a square and compute legal move highlights for it.
    pub fn select_square(&mut self, sq: types::Square) {
        self.selected_square = Some(sq);
        self.legal_moves = self
            .board
            .generate_legal_moves()
            .iter()
            .filter(|m| m.from == sq)
            .copied()
            .collect();
    }

    /// Clear the current selection.
    pub fn deselect(&mut self) {
        self.selected_square = None;
        self.legal_moves.clear();
    }

    /// Try to execute a move. Returns the move if legal, None otherwise.
    pub fn try_move(&mut self, target: types::Square) -> Option<types::Move> {
        let from = self.selected_square?;
        let legal = self.board.generate_legal_moves();
        let mv = legal
            .iter()
            .find(|m| m.from == from && m.to == target)
            .copied()?;

        self.last_move = Some(mv);
        self.board.make_move(mv);
        self.move_history.push(mv);
        self.deselect();

        if self.board.is_game_over() {
            self.game_result = Some(self.compute_result());
        }

        Some(mv)
    }

    /// Execute a move directly (for engine moves).
    pub fn make_move(&mut self, mv: types::Move) {
        self.last_move = Some(mv);
        self.board.make_move(mv);
        self.move_history.push(mv);
        self.deselect();

        if self.board.is_game_over() {
            self.game_result = Some(self.compute_result());
        }
    }

    /// Undo the last move if possible.
    pub fn undo_move(&mut self) -> Option<types::Move> {
        let mv = self.move_history.pop()?;
        self.board.unmake_move(mv);
        self.last_move = self.move_history.last().copied();
        self.game_result = None;
        self.deselect();
        Some(mv)
    }

    fn compute_result(&mut self) -> GameResult {
        if self.board.is_checkmate() {
            // The side to move is the loser in checkmate.
            match self.board.side_to_move {
                types::Color::White => GameResult::BlackWins,
                types::Color::Black => GameResult::WhiteWins,
            }
        } else {
            GameResult::Draw
        }
    }

    /// Whether it is the human player's turn.
    pub fn is_player_turn(&self) -> bool {
        self.board.side_to_move == self.player_color
    }
}

/// Configuration for the AI opponent.
#[derive(Resource)]
pub struct EngineConfig {
    pub depth: i32,
    pub time_limit_ms: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            depth: 6,
            time_limit_ms: 2000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() {
        types::init();
    }

    #[test]
    fn test_new_game_state() {
        setup();
        let game = ChessGame::new(types::Color::White);
        assert!(game.selected_square.is_none());
        assert!(game.legal_moves.is_empty());
        assert!(game.move_history.is_empty());
        assert!(game.last_move.is_none());
        assert!(!game.flipped);
        assert_eq!(game.player_color, types::Color::White);
        assert!(game.game_result.is_none());
    }

    #[test]
    fn test_select_and_legal_moves() {
        setup();
        let mut game = ChessGame::new(types::Color::White);
        let e2 = types::Square::from_index(12);
        game.select_square(e2);
        assert_eq!(game.selected_square, Some(e2));
        // e2 pawn has exactly 2 legal moves (e3, e4)
        assert_eq!(game.legal_moves.len(), 2);
    }

    #[test]
    fn test_try_move_legal() {
        setup();
        let mut game = ChessGame::new(types::Color::White);
        let e2 = types::Square::from_index(12);
        let e4 = types::Square::from_index(28);
        game.select_square(e2);
        let mv = game.try_move(e4);
        assert!(mv.is_some());
        assert_eq!(game.board.side_to_move, types::Color::Black);
        assert_eq!(game.move_history.len(), 1);
    }

    #[test]
    fn test_try_move_illegal() {
        setup();
        let mut game = ChessGame::new(types::Color::White);
        let e2 = types::Square::from_index(12);
        let e5 = types::Square::from_index(36);
        game.select_square(e2);
        let mv = game.try_move(e5);
        assert!(mv.is_none());
    }

    #[test]
    fn test_undo_move() {
        setup();
        let mut game = ChessGame::new(types::Color::White);
        let e2 = types::Square::from_index(12);
        let e4 = types::Square::from_index(28);
        game.select_square(e2);
        game.try_move(e4);
        assert_eq!(game.board.side_to_move, types::Color::Black);

        let undone = game.undo_move();
        assert!(undone.is_some());
        assert_eq!(game.board.side_to_move, types::Color::White);
        assert!(game.move_history.is_empty());
    }

    #[test]
    fn test_flipped_for_black() {
        setup();
        let game = ChessGame::new(types::Color::Black);
        assert!(game.flipped);
    }

    #[test]
    fn test_make_move_direct() {
        setup();
        let mut game = ChessGame::new(types::Color::White);
        let e2e4 = types::Move::quiet(types::Square::E2, types::Square::E4);
        // Find the legal double-pawn push
        let legal = game.board.generate_legal_moves();
        let mv = legal
            .iter()
            .find(|m| m.from == types::Square::E2 && m.to == types::Square::E4)
            .copied()
            .unwrap();
        game.make_move(mv);
        assert_eq!(game.board.side_to_move, types::Color::Black);
        assert_eq!(game.move_history.len(), 1);
    }
}
