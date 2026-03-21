use bevy::prelude::*;
use crate::state::{AppState, ChessGame, TurnState};

/// Fired when the player wants to move to a target square.
#[derive(Message)]
pub struct MoveMessage {
    pub target: types::Square,
}

/// Fired when the player wants to undo the last move.
#[derive(Message)]
pub struct UndoMessage;

/// Initialize a new game when entering the Playing state.
pub fn start_new_game(mut commands: Commands) {
    types::init();
    commands.insert_resource(ChessGame::new(types::Color::White));
}

/// Execute a player move when a MoveMessage is received.
pub fn execute_move(
    mut game: ResMut<ChessGame>,
    mut move_messages: MessageReader<MoveMessage>,
    mut turn_state: ResMut<NextState<TurnState>>,
    mut audio_messages: MessageWriter<crate::audio::SoundMessage>,
) {
    for msg in move_messages.read() {
        let target = msg.target;
        // Determine if a capture will occur for sound selection
        let is_capture = game
            .legal_moves
            .iter()
            .find(|m| m.to == target)
            .is_some_and(|m| m.is_capture());

        if game.try_move(target).is_some() {
            // Play appropriate sound
            if game.board.in_check() {
                audio_messages.write(crate::audio::SoundMessage::Check);
            } else if is_capture {
                audio_messages.write(crate::audio::SoundMessage::Capture);
            } else {
                audio_messages.write(crate::audio::SoundMessage::Move);
            }

            // Switch to engine turn
            if game.game_result.is_none() {
                turn_state.set(TurnState::EngineTurn);
            }
        }
    }
}

/// Handle undo requests.
pub fn handle_undo(
    mut game: ResMut<ChessGame>,
    mut undo_messages: MessageReader<UndoMessage>,
    mut turn_state: ResMut<NextState<TurnState>>,
) {
    for _msg in undo_messages.read() {
        // Undo the engine move and then the player move
        game.undo_move();
        game.undo_move();
        turn_state.set(TurnState::PlayerTurn);
    }
}

/// Detect game over conditions.
pub fn detect_game_over(
    game: Res<ChessGame>,
    mut app_state: ResMut<NextState<AppState>>,
) {
    if game.game_result.is_some() {
        app_state.set(AppState::GameOver);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_move_message_creation() {
        let msg = MoveMessage {
            target: types::Square::E4,
        };
        assert_eq!(msg.target, types::Square::E4);
    }
}
