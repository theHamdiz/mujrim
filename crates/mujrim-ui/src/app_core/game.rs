//! Game state management for the Mujrim UI.

use mujrim_study::board_marks::{ArrowRole, BoardArrow, MarkColor};
use types::{Board, Color, Square};

use super::arrows::{ArrowColor, user_arrow};
use super::premove::{self, MAX_PREMOVES, Premove};

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
#[derive(Clone)]
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
    /// Queued chess.com-style premoves executed when it becomes our turn.
    pub premove_queue: Vec<Premove>,
    /// User-drawn annotation arrows.
    pub arrows: Vec<BoardArrow>,
    /// Engine / coach / ponder / last-move overlay arrows.
    pub overlay_arrows: Vec<BoardArrow>,
    /// Starting square of an arrow being drawn (right-click drag).
    pub arrow_start: Option<Square>,
    /// Piece drag origin (left-button press).
    pub drag_from: Option<Square>,
    /// Square currently under the pointer while dragging.
    pub drag_over: Option<Square>,
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
            overlay_arrows: Vec::new(),
            arrow_start: None,
            drag_from: None,
            drag_over: None,
        }
    }

    /// Rebuild last-move + optional ponder overlays while keeping analysis overlays separately.
    pub fn refresh_move_overlays(
        &mut self,
        show_last_move_arrow: bool,
        ponder: Option<(Square, Square)>,
        analysis: &[BoardArrow],
    ) {
        let mut overlays = Vec::new();
        if show_last_move_arrow && self.last_move_squares.len() == 2 {
            overlays.push(
                BoardArrow::new(
                    self.last_move_squares[0],
                    self.last_move_squares[1],
                    MarkColor::Gold,
                    ArrowRole::LastMove,
                )
                .with_opacity(0.85),
            );
        }
        if let Some((from, to)) = ponder {
            overlays.push(
                BoardArrow::new(from, to, MarkColor::Cyan, ArrowRole::Ponder).with_opacity(0.35),
            );
        }
        overlays.extend(analysis.iter().cloned());
        self.overlay_arrows = overlays;
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

    /// Board after applying the queued premove chain for `human`.
    pub fn projected_premove_board(&self, human: Color) -> Board {
        premove::projected_board(&self.board, &self.premove_queue, human)
    }

    /// Select a piece for the next premove on the projected board.
    pub fn select_premove_square(&mut self, sq: Square, human: Color) {
        self.selected_square = Some(sq);
        self.legal_highlights =
            premove::premove_destinations(&self.board, &self.premove_queue, human, sq);
    }

    /// Queue a premove to `target` from the current selection.
    pub fn queue_premove(&mut self, target: Square, human: Color, allow_multi: bool) -> bool {
        let Some(from) = self.selected_square else {
            return false;
        };
        let Some(entry) =
            premove::make_premove(&self.board, &self.premove_queue, human, from, target)
        else {
            return false;
        };
        if !allow_multi {
            self.premove_queue.clear();
        }
        if self.premove_queue.len() >= MAX_PREMOVES {
            self.deselect();
            return false;
        }
        self.premove_queue.push(entry);
        self.deselect();
        true
    }

    pub fn clear_premoves(&mut self) {
        self.premove_queue.clear();
        self.deselect();
    }

    pub fn begin_drag(&mut self, from: Square) {
        self.drag_from = Some(from);
        self.drag_over = Some(from);
        self.select_square(from);
    }

    pub fn update_drag(&mut self, over: Square) {
        self.drag_over = Some(over);
    }

    pub fn end_drag(&mut self) -> Option<(Square, Square)> {
        let from = self.drag_from.take()?;
        let to = self.drag_over.take().unwrap_or(from);
        if from == to { None } else { Some((from, to)) }
    }

    /// Try to make a move from the selected square to `target`.
    #[cfg_attr(not(test), allow(dead_code))]
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
    pub fn finish_arrow(&mut self, to: Square, color: ArrowColor) {
        let Some(from) = self.arrow_start.take() else {
            return;
        };
        if from == to {
            self.arrows.clear();
            return;
        }
        let arrow = user_arrow(from, to, color);
        if let Some(idx) = self
            .arrows
            .iter()
            .position(|a| a.from == arrow.from && a.to == arrow.to && a.role == ArrowRole::User)
        {
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
        let e2 = Square::from_index(12);
        gs.select_square(e2);
        assert_eq!(gs.selected_square, Some(e2));
        assert_eq!(gs.legal_highlights.len(), 2);
    }

    #[test]
    fn test_try_move_legal() {
        let mut gs = setup();
        let e2 = Square::from_index(12);
        let e4 = Square::from_index(28);
        gs.select_square(e2);
        let result = gs.try_move(e4);
        assert!(result.is_some());
        assert_eq!(gs.board.side_to_move, types::Color::Black);
        assert_eq!(gs.last_move_squares, vec![e2, e4]);
    }

    #[test]
    fn test_arrows_toggle() {
        let mut gs = setup();
        let e2 = Square::from_index(12);
        let e4 = Square::from_index(28);
        gs.begin_arrow(e2);
        gs.finish_arrow(e4, ArrowColor::Orange);
        assert_eq!(gs.arrows.len(), 1);
        gs.begin_arrow(e2);
        gs.finish_arrow(e4, ArrowColor::Orange);
        assert!(gs.arrows.is_empty());
    }

    #[test]
    fn test_arrows_clear_all() {
        let mut gs = setup();
        gs.begin_arrow(Square::from_index(0));
        gs.finish_arrow(Square::from_index(16), ArrowColor::Green);
        gs.begin_arrow(Square::from_index(6));
        gs.finish_arrow(Square::from_index(21), ArrowColor::Blue);
        assert_eq!(gs.arrows.len(), 2);
        gs.begin_arrow(Square::from_index(0));
        gs.finish_arrow(Square::from_index(0), ArrowColor::Orange);
        assert!(gs.arrows.is_empty());
    }

    #[test]
    fn drag_selects_and_returns_from_to() {
        let mut gs = setup();
        let e2 = Square::from_index(12);
        let e4 = Square::from_index(28);
        gs.begin_drag(e2);
        assert_eq!(gs.selected_square, Some(e2));
        gs.update_drag(e4);
        assert_eq!(gs.end_drag(), Some((e2, e4)));
    }

    #[test]
    fn refresh_overlays_adds_last_move_and_ponder() {
        let mut gs = setup();
        gs.last_move_squares = vec![Square::from_index(12), Square::from_index(28)];
        gs.refresh_move_overlays(
            true,
            Some((Square::from_index(52), Square::from_index(36))),
            &[],
        );
        assert_eq!(gs.overlay_arrows.len(), 2);
        assert_eq!(gs.overlay_arrows[0].role, ArrowRole::LastMove);
        assert_eq!(gs.overlay_arrows[1].role, ArrowRole::Ponder);
        assert!((gs.overlay_arrows[1].resolved_opacity() - 0.35).abs() < f32::EPSILON);
    }

    #[test]
    fn test_display_to_square_matches_board_layout() {
        assert_eq!(display_to_square(0, 0, false).index(), 56);
        assert_eq!(display_to_square(7, 7, false).index(), 7);
        assert_eq!(display_to_square(0, 0, true).index(), 7);
        assert_eq!(display_to_square(7, 7, true).index(), 56);
    }

    #[test]
    fn multi_premove_queue_uses_projected_piece() {
        let mut gs = setup();
        let e2 = Square::from_index(12);
        let e4 = Square::from_index(28);
        let g1 = Square::from_index(6);
        let f3 = Square::from_index(21);
        gs.select_premove_square(e2, Color::White);
        assert!(gs.queue_premove(e4, Color::White, true));
        gs.select_premove_square(g1, Color::White);
        assert!(gs.queue_premove(f3, Color::White, true));
        assert_eq!(gs.premove_queue.len(), 2);
        let projected = gs.projected_premove_board(Color::White);
        assert!(projected.piece_on(e4).is_some());
        assert!(projected.piece_on(f3).is_some());
    }
}
