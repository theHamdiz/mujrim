//! Game state management for the Mujrim UI.

use types::{Board, Square};

/// Maps a board display cell `(row, col)` (0 = top/left of the widget) to a chess square.
pub fn display_to_square(row: usize, col: usize, flipped: bool) -> Square {
    let rank = if flipped { row } else { 7 - row };
    let file = if flipped { 7 - col } else { col };
    Square::from_index(rank * 8 + file)
}

/// Maps a board-local pointer position to a display cell `(row, col)`.
pub fn point_to_display(x: f32, y: f32, sq_size: f32) -> Option<(usize, usize)> {
    if !(sq_size.is_finite() && sq_size > 0.0) {
        return None;
    }
    let col = (x / sq_size).floor() as i32;
    let row = (y / sq_size).floor() as i32;
    if (0..8).contains(&col) && (0..8).contains(&row) {
        Some((row as usize, col as usize))
    } else {
        None
    }
}

/// Holds the current game state.
pub struct GameState {
    pub board: Board,
    /// Currently selected square (for move input).
    pub selected_square: Option<Square>,
    /// Squares highlighted as legal move targets.
    pub legal_highlights: Vec<Square>,
    /// Last move from/to squares for highlighting.
    pub last_move_squares: Vec<Square>,
    /// Whether the board is flipped (black at bottom).
    pub flipped: bool,
    /// Whether the game is over.
    pub game_over: bool,
    /// Queued premoves: (from, to) pairs executed when it becomes our turn.
    pub premove_queue: Vec<(Square, Square)>,
    /// User-drawn annotation arrows: (from, to) pairs drawn on the board.
    pub arrows: Vec<(Square, Square)>,
    /// Starting square of an arrow being drawn (right-click drag).
    pub arrow_start: Option<Square>,
}

impl GameState {
    pub fn new(board: Board) -> Self {
        Self {
            board,
            selected_square: None,
            legal_highlights: Vec::new(),
            last_move_squares: Vec::new(),
            flipped: false,
            game_over: false,
            premove_queue: Vec::new(),
            arrows: Vec::new(),
            arrow_start: None,
        }
    }

    /// Select a square and compute legal move highlights for it.
    pub fn select_square(&mut self, sq: Square) {
        self.selected_square = Some(sq);
        self.legal_highlights = self
            .board
            .generate_legal_moves()
            .iter()
            .filter(|m| m.from == sq)
            .map(|m| m.to)
            .collect();
    }

    /// Deselect the current square and clear highlights.
    pub fn deselect(&mut self) {
        self.selected_square = None;
        self.legal_highlights.clear();
    }

    /// Try to make a move from the selected square to `target`.
    /// Returns `Some(move)` if legal, `None` otherwise.
    #[allow(dead_code)]
    pub fn try_move(&mut self, target: Square) -> Option<types::Move> {
        let from = self.selected_square?;
        let legal = self.board.generate_legal_moves();
        let mv = legal
            .iter()
            .find(|m| m.from == from && m.to == target)
            .copied()?;

        self.last_move_squares = vec![mv.from, mv.to];
        self.board.make_move(mv);
        self.deselect();

        if self.board.is_game_over() {
            self.game_over = true;
        }

        Some(mv)
    }

    /// Begin a right-drag annotation arrow on `from`.
    pub fn begin_arrow(&mut self, from: Square) {
        self.arrow_start = Some(from);
    }

    /// Finish a right-drag annotation: toggle the arrow, or clear all on same-square release.
    pub fn finish_arrow(&mut self, to: Square) {
        let Some(from) = self.arrow_start.take() else {
            return;
        };
        if from == to {
            self.arrows.clear();
            return;
        }
        let arrow = (from, to);
        if let Some(idx) = self.arrows.iter().position(|a| *a == arrow) {
            self.arrows.remove(idx);
        } else {
            self.arrows.push(arrow);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> GameState {
        types::init();
        GameState::new(Board::new())
    }

    #[test]
    fn test_initial_state() {
        let gs = setup();
        assert!(gs.selected_square.is_none());
        assert!(gs.legal_highlights.is_empty());
        assert!(gs.last_move_squares.is_empty());
        assert!(!gs.flipped);
        assert!(!gs.game_over);
    }

    #[test]
    fn test_select_square_shows_legal_moves() {
        let mut gs = setup();
        // Select the e2 pawn (index 12)
        let e2 = Square::from_index(12);
        gs.select_square(e2);

        assert_eq!(gs.selected_square, Some(e2));
        // e2 pawn should have exactly 2 legal moves (e3, e4)
        assert_eq!(gs.legal_highlights.len(), 2);

        let e3 = Square::from_index(20);
        let e4 = Square::from_index(28);
        assert!(gs.legal_highlights.contains(&e3));
        assert!(gs.legal_highlights.contains(&e4));
    }

    #[test]
    fn test_select_empty_square_no_highlights() {
        let mut gs = setup();
        // Select an empty square (e4, index 28)
        let e4 = Square::from_index(28);
        gs.select_square(e4);

        assert_eq!(gs.selected_square, Some(e4));
        // No piece on e4, so no legal moves from it
        assert!(gs.legal_highlights.is_empty());
    }

    #[test]
    fn test_deselect_clears_state() {
        let mut gs = setup();
        let e2 = Square::from_index(12);
        gs.select_square(e2);
        assert!(gs.selected_square.is_some());

        gs.deselect();
        assert!(gs.selected_square.is_none());
        assert!(gs.legal_highlights.is_empty());
    }

    #[test]
    fn test_try_move_legal() {
        let mut gs = setup();
        let e2 = Square::from_index(12);
        let e4 = Square::from_index(28);

        gs.select_square(e2);
        let result = gs.try_move(e4);

        assert!(result.is_some());
        let mv = result.unwrap();
        assert_eq!(mv.from, e2);
        assert_eq!(mv.to, e4);

        // Board should now have black to move
        assert_eq!(gs.board.side_to_move, types::Color::Black);
        // last_move_squares should be set
        assert_eq!(gs.last_move_squares, vec![e2, e4]);
        // Selection should be cleared
        assert!(gs.selected_square.is_none());
    }

    #[test]
    fn test_try_move_illegal_returns_none() {
        let mut gs = setup();
        let e2 = Square::from_index(12);
        let e5 = Square::from_index(36); // e5 is not reachable from e2 in one move

        gs.select_square(e2);
        let result = gs.try_move(e5);

        assert!(result.is_none());
        // Board should not have changed
        assert_eq!(gs.board.side_to_move, types::Color::White);
    }

    #[test]
    fn test_try_move_without_selection_returns_none() {
        let mut gs = setup();
        let e4 = Square::from_index(28);

        let result = gs.try_move(e4);
        assert!(result.is_none());
    }

    #[test]
    fn test_flip_board() {
        let mut gs = setup();
        assert!(!gs.flipped);
        gs.flipped = true;
        assert!(gs.flipped);
        gs.flipped = !gs.flipped;
        assert!(!gs.flipped);
    }

    #[test]
    fn test_game_over_on_checkmate() {
        let mut gs = GameState::new(
            Board::from_fen("rnbqkbnr/pppp1ppp/4p3/8/6P1/5P2/PPPPP2P/RNBQKBNR b KQkq - 0 2")
                .unwrap(),
        );
        // This position allows Qh4# (scholar's mate setup after f3 and g4)
        let d8 = Square::from_index(59); // queen on d8
        let h4 = Square::from_index(31); // Qh4#

        gs.select_square(d8);
        let result = gs.try_move(h4);

        assert!(result.is_some());
        assert!(gs.game_over);
    }

    #[test]
    fn test_multiple_moves_sequence() {
        let mut gs = setup();

        // 1. e4
        gs.select_square(Square::from_index(12)); // e2
        assert!(gs.try_move(Square::from_index(28)).is_some()); // e4

        // 1... e5
        gs.select_square(Square::from_index(52)); // e7
        assert!(gs.try_move(Square::from_index(36)).is_some()); // e5

        // 2. Nf3
        gs.select_square(Square::from_index(6)); // g1
        assert!(gs.try_move(Square::from_index(21)).is_some()); // Nf3

        assert_eq!(gs.board.side_to_move, types::Color::Black);
        assert!(!gs.game_over);
    }

    #[test]
    fn test_arrows_initial_state() {
        let gs = setup();
        assert!(gs.arrows.is_empty());
        assert!(gs.arrow_start.is_none());
    }

    #[test]
    fn test_arrows_toggle() {
        let mut gs = setup();
        let e2 = Square::from_index(12);
        let e4 = Square::from_index(28);

        gs.begin_arrow(e2);
        gs.finish_arrow(e4);
        assert_eq!(gs.arrows, vec![(e2, e4)]);

        gs.begin_arrow(e2);
        gs.finish_arrow(e4);
        assert!(gs.arrows.is_empty());
    }

    #[test]
    fn test_arrows_clear_all() {
        let mut gs = setup();
        gs.begin_arrow(Square::from_index(0));
        gs.finish_arrow(Square::from_index(16));
        gs.begin_arrow(Square::from_index(6));
        gs.finish_arrow(Square::from_index(21));
        assert_eq!(gs.arrows.len(), 2);

        gs.begin_arrow(Square::from_index(0));
        gs.finish_arrow(Square::from_index(0));
        assert!(gs.arrows.is_empty());
        assert!(gs.arrow_start.is_none());
    }

    #[test]
    fn test_arrow_start_set_and_take() {
        let mut gs = setup();
        assert!(gs.arrow_start.is_none());

        gs.begin_arrow(Square::from_index(12));
        assert_eq!(gs.arrow_start, Some(Square::from_index(12)));

        gs.finish_arrow(Square::from_index(28));
        assert!(gs.arrow_start.is_none());
        assert_eq!(gs.arrows.len(), 1);
    }

    #[test]
    fn test_display_to_square_matches_board_layout() {
        // Unflipped: top-left display cell is a8.
        assert_eq!(display_to_square(0, 0, false).index(), 56);
        // Unflipped: bottom-right display cell is h1.
        assert_eq!(display_to_square(7, 7, false).index(), 7);
        // Flipped: top-left display cell is h1.
        assert_eq!(display_to_square(0, 0, true).index(), 7);
        // Flipped: bottom-right display cell is a8.
        assert_eq!(display_to_square(7, 7, true).index(), 56);
    }

    #[test]
    fn test_point_to_display_maps_square_centers() {
        let sq = 80.0;
        assert_eq!(point_to_display(40.0, 40.0, sq), Some((0, 0)));
        assert_eq!(point_to_display(600.0, 600.0, sq), Some((7, 7)));
        assert_eq!(point_to_display(-1.0, 40.0, sq), None);
        assert_eq!(point_to_display(40.0, 640.0, sq), None);
    }

    #[test]
    fn test_finish_arrow_without_start_is_noop() {
        let mut gs = setup();
        gs.finish_arrow(Square::from_index(28));
        assert!(gs.arrows.is_empty());
        assert!(gs.arrow_start.is_none());
    }
}
