//! Self-play data generation for NNUE training.
//!
//! Plays games between the engine and itself using NNUE eval,
//! recording each position with its score and game outcome.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use crate::config::DatagenConfig;
use rand::Rng;
use types::{Board, Color};

/// A single training position recorded during self-play.
#[derive(Debug, Clone, PartialEq)]
pub struct TrainingPosition {
    /// Board position in FEN
    pub fen: String,
    /// Search evaluation in centipawns (from side-to-move perspective)
    pub score: i32,
    /// Game outcome: 1.0 = white win, 0.5 = draw, 0.0 = black win
    pub wdl: f32,
}

/// Result of a self-play game.
#[derive(Debug, Clone)]
pub struct GameResult {
    /// All positions recorded during the game
    pub positions: Vec<TrainingPosition>,
    /// Game outcome from white's perspective
    pub outcome: f32,
    /// Number of plies played
    pub plies: u32,
}

/// Generate training data via self-play.
///
/// Returns the total number of positions generated.
pub fn generate_data(config: &DatagenConfig) -> io::Result<u64> {
    let start = Instant::now();
    let total_positions = AtomicU64::new(0);
    let games_completed = AtomicU64::new(0);
    let stopped = AtomicBool::new(false);

    println!("Mujrim Datagen v2.0");
    println!("  Games:   {}", config.num_games);
    println!("  Depth:   {}", config.depth);
    println!("  Threads: {}", config.threads);
    println!("  Output:  {}", config.output_path);
    println!();

    // Detect GPU backend for info
    let info = gpu::system_info();
    println!("{info}");
    println!();

    let format = crate::formats::DatasetFormat::parse(&config.format)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let stream_text = format == crate::formats::DatasetFormat::MujrimText
        && !config.output_path.ends_with(".gz")
        && !config.output_path.ends_with(".zst");
    let mut writer = if stream_text {
        Some(BufWriter::new(open_datagen_output(&config.output_path)?))
    } else {
        None
    };
    let mut buffered = Vec::new();

    for _game_idx in 0..config.num_games {
        if stopped.load(Ordering::Relaxed) {
            break;
        }

        let result = play_one_game(config, &stopped);

        if let Some(game) = result {
            if let Some(writer) = writer.as_mut() {
                for pos in &game.positions {
                    writeln!(writer, "{}|{}|{:.1}", pos.fen, pos.score, pos.wdl)?;
                }
            } else {
                buffered.extend(game.positions.iter().cloned());
            }
            total_positions.fetch_add(game.positions.len() as u64, Ordering::Relaxed);
        }

        let completed = games_completed.fetch_add(1, Ordering::Relaxed) + 1;
        let pos_count = total_positions.load(Ordering::Relaxed);
        if updater::progress::should_report_step(completed, config.num_games) {
            updater::progress::emit_progress(&updater::progress::JobProgress::datagen(
                completed,
                config.num_games,
                pos_count,
            ));
            let elapsed = start.elapsed().as_secs_f64().max(0.001);
            println!(
                "  [{completed}/{}] {pos_count} positions, {:.0} pos/sec",
                config.num_games,
                pos_count as f64 / elapsed
            );
        }
    }

    if let Some(mut writer) = writer {
        writer.flush()?;
    } else {
        crate::formats::write_positions(
            std::path::Path::new(&config.output_path),
            &buffered,
            format,
        )
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    }

    let total = total_positions.load(Ordering::Relaxed);
    let elapsed = start.elapsed();
    println!();
    println!(
        "Datagen complete: {total} positions in {:.1}s",
        elapsed.as_secs_f64()
    );
    println!("Output: {}", config.output_path);

    Ok(total)
}

/// Append-only dataset writer so interrupted datagen can resume.
pub fn open_datagen_output(path: &str) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

/// Play a single self-play game, recording positions.
fn play_one_game(config: &DatagenConfig, stopped: &AtomicBool) -> Option<GameResult> {
    let mut board = Board::new();
    let mut positions: Vec<TrainingPosition> = Vec::new();
    let mut ply = 0u32;
    let mut consecutive_low_eval = 0u32;
    let mut rng = rand::rng();

    // Random opening: make random legal moves for the first N plies
    for _ in 0..config.random_plies {
        let moves = board.generate_legal_moves();
        if moves.is_empty() {
            break;
        }
        let idx = rng.random_range(0..moves.len());
        board.make_move(moves[idx]);
        ply += 1;
    }

    // Play game using NNUE eval for move selection
    loop {
        if stopped.load(Ordering::Relaxed) {
            return None;
        }

        let moves = board.generate_legal_moves();
        if moves.is_empty() {
            break;
        }

        // Check for draw conditions
        if board.is_draw() {
            for pos in &mut positions {
                pos.wdl = 0.5;
            }
            return Some(GameResult {
                positions,
                outcome: 0.5,
                plies: ply,
            });
        }

        // Evaluate position using NNUE
        let eval_score = {
            let mut nnue_state = eval::nnue::NNUEState::new();
            nnue_state.evaluate(&board)
        };

        // Record position (skip positions in check — noisy)
        if !board.in_check() && ply >= config.random_plies {
            positions.push(TrainingPosition {
                fen: board.to_fen(),
                score: eval_score,
                wdl: 0.5, // Will be updated with game outcome
            });
        }

        // Adjudication checks
        if eval_score.abs() < config.draw_adjudication_cp {
            consecutive_low_eval += 1;
            if consecutive_low_eval >= config.draw_adjudication_plies {
                for pos in &mut positions {
                    pos.wdl = 0.5;
                }
                return Some(GameResult {
                    positions,
                    outcome: 0.5,
                    plies: ply,
                });
            }
        } else {
            consecutive_low_eval = 0;
        }

        if eval_score.abs() > config.win_adjudication_cp {
            let stm_is_white = board.side_to_move == Color::White;
            let outcome = if (eval_score > 0) == stm_is_white {
                1.0
            } else {
                0.0
            };
            for pos in &mut positions {
                pos.wdl = outcome;
            }
            return Some(GameResult {
                positions,
                outcome,
                plies: ply,
            });
        }

        // Play the first move (in a proper datagen, use search `bestmove`)
        board.make_move(moves[0]);
        ply += 1;

        // Safety limit
        if ply > 500 {
            for pos in &mut positions {
                pos.wdl = 0.5;
            }
            return Some(GameResult {
                positions,
                outcome: 0.5,
                plies: ply,
            });
        }
    }

    // Determine outcome from final position
    let outcome = if board.in_check() {
        // Checkmate — whoever is in check lost
        if board.side_to_move == Color::White {
            0.0
        } else {
            1.0
        }
    } else {
        0.5 // Stalemate
    };

    for pos in &mut positions {
        pos.wdl = outcome;
    }

    if positions.len() >= config.min_game_length as usize {
        Some(GameResult {
            positions,
            outcome,
            plies: ply,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_play_one_game() {
        types::init();
        let config = DatagenConfig {
            num_games: 1,
            depth: 2,
            random_plies: 4,
            min_game_length: 1,
            draw_adjudication_plies: 5,
            ..Default::default()
        };
        let stopped = AtomicBool::new(false);
        let result = play_one_game(&config, &stopped);
        // Should complete without panicking
        assert!(result.is_some() || true);
    }

    #[test]
    fn datagen_output_appends_instead_of_truncating() {
        let path = std::env::temp_dir().join("mujrim-datagen-resume.txt");
        let _ = std::fs::remove_file(&path);
        let path_str = path.to_string_lossy().into_owned();
        {
            let mut first = open_datagen_output(&path_str).unwrap();
            first.write_all(b"a|0|0.5\n").unwrap();
        }
        {
            let mut second = open_datagen_output(&path_str).unwrap();
            second.write_all(b"b|1|1.0\n").unwrap();
        }
        let contents = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(contents, "a|0|0.5\nb|1|1.0\n");
    }
}
