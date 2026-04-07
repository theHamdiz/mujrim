use bevy::prelude::*;
use crate::state::{AppState, ChessGame, GameResult, EngineConfig};
use crate::engine::EngineInfo;

// ── Colors ──────────────────────────────────────────────────────────────────
const BG_COLOR: Color = Color::srgba(0.12, 0.12, 0.15, 0.95);
const BTN_COLOR: Color = Color::srgb(0.25, 0.55, 0.35);
const BTN_HOVER: Color = Color::srgb(0.35, 0.70, 0.45);
const TEXT_COLOR: Color = Color::srgb(0.92, 0.92, 0.92);
const ACCENT: Color = Color::srgb(0.85, 0.70, 0.30);

// ── Component markers ───────────────────────────────────────────────────────

#[derive(Component)]
pub struct MenuRoot;

#[derive(Component)]
pub struct MenuPlayButton;

#[derive(Component)]
pub struct HudRoot;

#[derive(Component)]
pub struct HudTurnText;

#[derive(Component)]
pub struct HudEngineText;

#[derive(Component)]
pub struct HudMoveList;

#[derive(Component)]
pub struct HudUndoButton;

#[derive(Component)]
pub struct HudFlipButton;

#[derive(Component)]
pub struct HudNewGameButton;

#[derive(Component)]
pub struct HudResignButton;

#[derive(Component)]
pub struct HudModeButton;

#[derive(Component)]
pub struct HudDepthText;

#[derive(Component)]
pub struct GameOverRoot;

#[derive(Component)]
pub struct GameOverPlayAgainButton;

// ── Menu ────────────────────────────────────────────────────────────────────

pub fn spawn_menu(mut commands: Commands) {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(32.0),
                ..default()
            },
            BackgroundColor(Color::srgb(0.08, 0.08, 0.10)),
            MenuRoot,
        ))
        .with_children(|parent| {
            // Title
            parent.spawn((
                Text::new("KishMat Chess"),
                TextFont {
                    font_size: 56.0,
                    ..default()
                },
                TextColor(ACCENT),
            ));

            parent.spawn((
                Text::new("The First Arabian Chess Engine"),
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(Color::srgb(0.6, 0.6, 0.6)),
            ));

            // Play button
            parent
                .spawn(Button)
                .insert(Node {
                    padding: UiRect::axes(Val::Px(48.0), Val::Px(16.0)),
                    margin: UiRect::top(Val::Px(24.0)),
                    ..default()
                })
                .insert(BackgroundColor(BTN_COLOR))
                .insert(MenuPlayButton)
                .with_child((
                    Text::new("Play"),
                    TextFont {
                        font_size: 28.0,
                        ..default()
                    },
                    TextColor(TEXT_COLOR),
                ));
        });
}

pub fn despawn_menu(mut commands: Commands, menu: Query<Entity, With<MenuRoot>>) {
    for entity in menu.iter() {
        commands.entity(entity).despawn();
    }
}

pub fn menu_button_system(
    mut interaction_q: Query<(&Interaction, &mut BackgroundColor), (Changed<Interaction>, With<MenuPlayButton>)>,
    mut app_state: ResMut<NextState<AppState>>,
    mut audio_messages: MessageWriter<crate::audio::SoundMessage>,
) {
    for (interaction, mut bg) in interaction_q.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                audio_messages.write(crate::audio::SoundMessage::Click);
                app_state.set(AppState::Playing);
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(BTN_HOVER);
            }
            Interaction::None => {
                *bg = BackgroundColor(BTN_COLOR);
            }
        }
    }
}

// ── HUD ─────────────────────────────────────────────────────────────────────

pub fn spawn_hud(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(260.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(14.0)),
                row_gap: Val::Px(8.0),
                overflow: Overflow::clip_y(),
                ..default()
            },
            BackgroundColor(BG_COLOR),
            HudRoot,
        ))
        .with_children(|parent| {
            // Turn indicator
            parent.spawn((
                Text::new("White to move"),
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(TEXT_COLOR),
                HudTurnText,
            ));

            // Engine info
            parent.spawn((
                Text::new("Engine: idle"),
                TextFont {
                    font_size: 13.0,
                    ..default()
                },
                TextColor(Color::srgb(0.6, 0.8, 0.6)),
                HudEngineText,
            ));

            // Engine depth
            parent.spawn((
                Text::new("Depth: 6"),
                TextFont {
                    font_size: 11.0,
                    ..default()
                },
                TextColor(Color::srgb(0.5, 0.5, 0.5)),
                HudDepthText,
            ));

            // Move list header
            parent.spawn((
                Text::new("Moves:"),
                TextFont {
                    font_size: 13.0,
                    ..default()
                },
                TextColor(ACCENT),
            ));

            // Scrollable move list container
            parent
                .spawn(Node {
                    flex_grow: 1.0,
                    min_height: Val::Px(50.0),
                    max_height: Val::Percent(50.0),
                    overflow: Overflow::scroll_y(),
                    flex_direction: FlexDirection::Column,
                    ..default()
                })
                .with_children(|scroll| {
                    scroll.spawn((
                        Text::new(""),
                        TextFont {
                            font_size: 12.0,
                            ..default()
                        },
                        TextColor(TEXT_COLOR),
                        HudMoveList,
                    ));
                });

            // Buttons row 1
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(4.0),
                    flex_wrap: FlexWrap::Wrap,
                    row_gap: Val::Px(4.0),
                    margin: UiRect::top(Val::Auto),
                    ..default()
                })
                .with_children(|row| {
                    spawn_hud_button(row, "Undo", HudUndoButton);
                    spawn_hud_button(row, "Flip", HudFlipButton);
                    spawn_hud_button(row, "Resign", HudResignButton);
                    spawn_hud_button(row, "New", HudNewGameButton);
                });

            // Mode toggle button
            parent
                .spawn(Button)
                .insert(Node {
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                    align_self: AlignSelf::Stretch,
                    justify_content: JustifyContent::Center,
                    ..default()
                })
                .insert(BackgroundColor(Color::srgb(0.3, 0.3, 0.5)))
                .insert(HudModeButton)
                .with_child((
                    Text::new("2D / 3D [Tab]"),
                    TextFont {
                        font_size: 11.0,
                        ..default()
                    },
                    TextColor(TEXT_COLOR),
                ));
        });
}

fn spawn_hud_button<M: Component>(parent: &mut ChildSpawnerCommands, label: &str, marker: M) {
    parent
        .spawn(Button)
        .insert(Node {
            padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
            ..default()
        })
        .insert(BackgroundColor(BTN_COLOR))
        .insert(marker)
        .with_child((
            Text::new(label.to_string()),
            TextFont {
                font_size: 13.0,
                ..default()
            },
            TextColor(TEXT_COLOR),
        ));
}

pub fn update_hud(
    game: Option<Res<ChessGame>>,
    engine_info: Option<Res<EngineInfo>>,
    mut turn_text: Query<&mut Text, (With<HudTurnText>, Without<HudEngineText>, Without<HudMoveList>)>,
    mut engine_text: Query<&mut Text, (With<HudEngineText>, Without<HudTurnText>, Without<HudMoveList>)>,
    mut move_list: Query<&mut Text, (With<HudMoveList>, Without<HudTurnText>, Without<HudEngineText>)>,
) {
    let Some(game) = game else { return };

    // Only update when game state changed
    if !game.is_changed() {
        // Still update engine info if it changed
        if let Some(info) = engine_info {
            if info.is_changed() {
                for mut text in engine_text.iter_mut() {
                    if info.depth > 0 {
                        let score_str = if info.score.abs() > 28000 {
                            "Mate".to_string()
                        } else {
                            format!("{:+.1}", info.score as f64 / 100.0)
                        };
                        **text = format!(
                            "d{} {} | {}n",
                            info.depth, score_str, info.nodes
                        );
                    }
                }
            }
        }
        return;
    }

    // Turn indicator
    for mut text in turn_text.iter_mut() {
        let side = match game.board.side_to_move {
            types::Color::White => "White",
            types::Color::Black => "Black",
        };
        let check = if game.board.in_check() { " ✓" } else { "" };
        **text = format!("{side} to move{check}");
    }

    // Engine info
    if let Some(info) = engine_info {
        for mut text in engine_text.iter_mut() {
            if info.depth > 0 {
                let score_str = if info.score.abs() > 28000 {
                    "Mate".to_string()
                } else {
                    format!("{:+.1}", info.score as f64 / 100.0)
                };
                **text = format!(
                    "d{} {} | {}n",
                    info.depth, score_str, info.nodes
                );
            }
        }
    }

    // Move list (only last 20 moves to avoid perf issues)
    for mut text in move_list.iter_mut() {
        let total = game.move_history.len();
        let skip = if total > 40 { total - 40 } else { 0 };
        let start_pair = skip / 2;
        let moves: Vec<String> = game
            .move_history
            .chunks(2)
            .enumerate()
            .skip(start_pair)
            .map(|(i, chunk)| {
                let w = chunk[0].to_uci();
                let b = chunk.get(1).map(|m| m.to_uci()).unwrap_or_default();
                format!("{}. {} {}", i + 1, w, b)
            })
            .collect();
        **text = moves.join("\n");
    }
}

pub fn hud_button_system(
    undo_q: Query<&Interaction, (Changed<Interaction>, With<HudUndoButton>)>,
    flip_q: Query<&Interaction, (Changed<Interaction>, With<HudFlipButton>)>,
    new_q: Query<&Interaction, (Changed<Interaction>, With<HudNewGameButton>)>,
    mut undo_messages: MessageWriter<crate::game_logic::UndoMessage>,
    mut game: Option<ResMut<ChessGame>>,
    mut app_state: ResMut<NextState<AppState>>,
    mut audio_messages: MessageWriter<crate::audio::SoundMessage>,
) {
    for interaction in undo_q.iter() {
        if *interaction == Interaction::Pressed {
            audio_messages.write(crate::audio::SoundMessage::Click);
            undo_messages.write(crate::game_logic::UndoMessage);
        }
    }

    for interaction in flip_q.iter() {
        if *interaction == Interaction::Pressed {
            audio_messages.write(crate::audio::SoundMessage::Click);
            if let Some(ref mut g) = game {
                g.flipped = !g.flipped;
            }
        }
    }

    for interaction in new_q.iter() {
        if *interaction == Interaction::Pressed {
            audio_messages.write(crate::audio::SoundMessage::Click);
            app_state.set(AppState::Menu);
        }
    }
}

pub fn hud_resign_system(
    resign_q: Query<&Interaction, (Changed<Interaction>, With<HudResignButton>)>,
    mut game: Option<ResMut<ChessGame>>,
    mut app_state: ResMut<NextState<AppState>>,
    mut audio_messages: MessageWriter<crate::audio::SoundMessage>,
) {
    for interaction in resign_q.iter() {
        if *interaction == Interaction::Pressed {
            audio_messages.write(crate::audio::SoundMessage::Click);
            if let Some(ref mut g) = game {
                let result = match g.player_color {
                    types::Color::White => GameResult::BlackWins,
                    types::Color::Black => GameResult::WhiteWins,
                };
                g.game_result = Some(result);
                app_state.set(AppState::GameOver);
            }
        }
    }
}

pub fn update_depth_text(
    config: Res<EngineConfig>,
    mut text_q: Query<&mut Text, With<HudDepthText>>,
) {
    if !config.is_changed() {
        return;
    }
    for mut text in text_q.iter_mut() {
        **text = format!("Engine depth: {}", config.depth);
    }
}

// ── Game Over ───────────────────────────────────────────────────────────────

pub fn spawn_game_over_overlay(mut commands: Commands, game: Option<Res<ChessGame>>) {
    let result_text = game
        .and_then(|g| g.game_result)
        .map(|r| match r {
            GameResult::WhiteWins => "Checkmate! White wins!",
            GameResult::BlackWins => "Checkmate! Black wins!",
            GameResult::Draw => "Game drawn!",
        })
        .unwrap_or("Game Over");

    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                position_type: PositionType::Absolute,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(24.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
            GameOverRoot,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(result_text.to_string()),
                TextFont {
                    font_size: 48.0,
                    ..default()
                },
                TextColor(ACCENT),
            ));

            parent
                .spawn(Button)
                .insert(Node {
                    padding: UiRect::axes(Val::Px(36.0), Val::Px(14.0)),
                    ..default()
                })
                .insert(BackgroundColor(BTN_COLOR))
                .insert(GameOverPlayAgainButton)
                .with_child((
                    Text::new("Play Again"),
                    TextFont {
                        font_size: 22.0,
                        ..default()
                    },
                    TextColor(TEXT_COLOR),
                ));
        });
}

pub fn game_over_button_system(
    interaction_q: Query<&Interaction, (Changed<Interaction>, With<GameOverPlayAgainButton>)>,
    mut app_state: ResMut<NextState<AppState>>,
    mut commands: Commands,
    overlay: Query<Entity, With<GameOverRoot>>,
    hud: Query<Entity, With<HudRoot>>,
    mut audio_messages: MessageWriter<crate::audio::SoundMessage>,
) {
    for interaction in interaction_q.iter() {
        if *interaction == Interaction::Pressed {
            audio_messages.write(crate::audio::SoundMessage::Click);
            for entity in overlay.iter() {
                commands.entity(entity).despawn();
            }
            for entity in hud.iter() {
                commands.entity(entity).despawn();
            }
            app_state.set(AppState::Menu);
        }
    }
}
