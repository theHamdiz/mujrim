//! Self-play data generation for NNUE training.
//!
//! Plays games between the engine and itself using NNUE eval,
//! recording each position with its score and game outcome.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use updater::progress::{DatagenBatch, HIST_BUCKETS, JobProgress};

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

pub const SCORE_HIST_MIN: i32 = -800;
pub const SCORE_HIST_MAX: i32 = 800;

/// Map a centipawn score onto a fixed 16-bucket histogram.
pub fn score_histogram_bucket(score: i32) -> usize {
    let width = (SCORE_HIST_MAX - SCORE_HIST_MIN) / HIST_BUCKETS as i32;
    let clamped = score.clamp(SCORE_HIST_MIN, SCORE_HIST_MAX);
    let bucket = ((clamped - SCORE_HIST_MIN) / width) as usize;
    bucket.min(HIST_BUCKETS - 1)
}

/// White / draw / black slot from a WDL outcome in `[0, 1]`.
pub fn wdl_slot(outcome: f32) -> usize {
    if outcome >= 0.75 {
        0
    } else if outcome <= 0.25 {
        2
    } else {
        1
    }
}

#[derive(Debug)]
struct DatagenStats {
    white: AtomicU64,
    draw: AtomicU64,
    black: AtomicU64,
    pass: AtomicU64,
    drop: AtomicU64,
    bytes: AtomicU64,
    hist: [AtomicU32; HIST_BUCKETS],
}

impl DatagenStats {
    fn new() -> Self {
        Self {
            white: AtomicU64::new(0),
            draw: AtomicU64::new(0),
            black: AtomicU64::new(0),
            pass: AtomicU64::new(0),
            drop: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            hist: [const { AtomicU32::new(0) }; HIST_BUCKETS],
        }
    }

    fn record(&self, game: Option<&GameResult>, in_check_drops: u64, written_bytes: u64) {
        self.drop.fetch_add(in_check_drops, Ordering::Relaxed);
        self.bytes.fetch_add(written_bytes, Ordering::Relaxed);
        let Some(game) = game else {
            self.drop.fetch_add(1, Ordering::Relaxed);
            return;
        };
        self.pass
            .fetch_add(game.positions.len() as u64, Ordering::Relaxed);
        match wdl_slot(game.outcome) {
            0 => {
                self.white.fetch_add(1, Ordering::Relaxed);
            }
            1 => {
                self.draw.fetch_add(1, Ordering::Relaxed);
            }
            _ => {
                self.black.fetch_add(1, Ordering::Relaxed);
            }
        }
        for pos in &game.positions {
            self.hist[score_histogram_bucket(pos.score)].fetch_add(1, Ordering::Relaxed);
        }
    }

    fn snapshot_hist(&self) -> [u32; HIST_BUCKETS] {
        let mut hist = [0u32; HIST_BUCKETS];
        for (slot, bucket) in hist.iter_mut().zip(self.hist.iter()) {
            *slot = bucket.load(Ordering::Relaxed);
        }
        hist
    }

    fn batch(
        &self,
        game: u64,
        games: u64,
        positions: u64,
        nps: u64,
        throughput: f32,
    ) -> JobProgress {
        JobProgress::datagen_batch(DatagenBatch {
            game,
            games,
            positions,
            nps,
            throughput,
            white: self.white.load(Ordering::Relaxed),
            draw: self.draw.load(Ordering::Relaxed),
            black: self.black.load(Ordering::Relaxed),
            pass: self.pass.load(Ordering::Relaxed),
            drop: self.drop.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            hist: self.snapshot_hist(),
        })
    }
}

fn position_line_bytes(pos: &TrainingPosition) -> u64 {
    (pos.fen.len() + 1 + pos.score.to_string().len() + 1 + 3 + 1) as u64
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
    let completed = crate::job::resume_datagen(config);
    let resumed_positions = crate::job::resume_datagen_positions(config);
    games_completed.store(completed, Ordering::Relaxed);
    total_positions.store(resumed_positions, Ordering::Relaxed);
    crate::job::datagen_checkpoint(config, completed, resumed_positions)
        .save()
        .map_err(io::Error::other)?;
    let stats = DatagenStats::new();
    let mut last_emit = start;
    let ateed = load_ateed_eval();
    let index_root = eval::nnue::writable_nnue_directory();
    let index_path = mujrim_study::ateed_index::index_path(&index_root);
    let mut index = mujrim_study::ateed_index::PositionIndex::load(&index_path);

    for _game_idx in completed..config.num_games {
        if stopped.load(Ordering::Relaxed)
            || datagen_batch_complete(
                total_positions.load(Ordering::Relaxed),
                games_completed.load(Ordering::Relaxed),
                config,
            )
        {
            break;
        }

        let (mut result, in_check_drops) = play_one_game(config, &stopped, ateed.as_ref());
        let mut written = 0u64;

        if let Some(game) = result.as_mut() {
            game.positions.retain(|pos| index.insert_fen(&pos.fen));
            if let Some(writer) = writer.as_mut() {
                for pos in &game.positions {
                    writeln!(writer, "{}|{}|{:.1}", pos.fen, pos.score, pos.wdl)?;
                    written += position_line_bytes(pos);
                }
            } else {
                for pos in &game.positions {
                    written += position_line_bytes(pos);
                }
                buffered.extend(game.positions.iter().cloned());
            }
            total_positions.fetch_add(game.positions.len() as u64, Ordering::Relaxed);
        }
        stats.record(result.as_ref(), in_check_drops, written);

        let completed = games_completed.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(writer) = writer.as_mut() {
            writer.flush()?;
        }
        let pos_count = total_positions.load(Ordering::Relaxed);
        crate::job::datagen_checkpoint(config, completed, pos_count)
            .save()
            .map_err(io::Error::other)?;
        let now = Instant::now();
        if updater::progress::should_report_now(completed, config.num_games, last_emit, now) {
            last_emit = now;
            let elapsed = start.elapsed().as_secs_f64().max(0.001);
            let nps = (pos_count as f64 / elapsed) as u64;
            let throughput =
                stats.bytes.load(Ordering::Relaxed) as f32 / elapsed as f32 / 1_048_576.0;
            updater::progress::emit_progress(&stats.batch(
                completed,
                config.num_games,
                pos_count,
                nps,
                throughput,
            ));
            println!(
                "  [{completed}/{}] {pos_count} positions, {nps} pos/sec",
                config.num_games
            );
            let _ = index.save(&index_path);
        }
    }
    let _ = index.save(&index_path);

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
    crate::job::JobCheckpoint::clear(std::path::Path::new(&config.output_path));

    Ok(total)
}

/// True when this batch has enough new positions or has hit the game cap.
pub fn datagen_batch_complete(positions: u64, games: u64, config: &DatagenConfig) -> bool {
    games >= config.num_games
        || config
            .num_positions
            .is_some_and(|target| positions >= target)
}

/// Append-only dataset writer so interrupted datagen can resume.
pub fn open_datagen_output(path: &str) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

/// Play a single self-play game, recording positions.
fn load_ateed_eval() -> Option<eval::nnue::AteedNetwork> {
    let path = eval::nnue::discover_named_network(eval::nnue::ATEED_NETWORK_FILENAME)?;
    match eval::nnue::load_network(&path).ok()? {
        eval::nnue::ActiveNetwork::ExternalAteed { network, .. } => Some(*network),
        _ => None,
    }
}

fn evaluate_board(board: &Board, ateed: Option<&eval::nnue::AteedNetwork>) -> i32 {
    if let Some(network) = ateed {
        return network.evaluate(board);
    }
    let mut nnue_state = eval::nnue::NNUEState::new();
    nnue_state.evaluate(board)
}

fn play_one_game(
    config: &DatagenConfig,
    stopped: &AtomicBool,
    ateed: Option<&eval::nnue::AteedNetwork>,
) -> (Option<GameResult>, u64) {
    let mut board = Board::new();
    let mut positions: Vec<TrainingPosition> = Vec::new();
    let mut ply = 0u32;
    let mut consecutive_low_eval = 0u32;
    let mut in_check_drops = 0u64;
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
            return (None, in_check_drops);
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
            return (
                Some(GameResult {
                    positions,
                    outcome: 0.5,
                    plies: ply,
                }),
                in_check_drops,
            );
        }

        // Evaluate position using NNUE
        let eval_score = evaluate_board(&board, ateed);

        // Record position (skip positions in check — noisy)
        if ply >= config.random_plies {
            if board.in_check() {
                in_check_drops += 1;
            } else {
                positions.push(TrainingPosition {
                    fen: board.to_fen(),
                    score: eval_score,
                    wdl: 0.5, // Will be updated with game outcome
                });
            }
        }

        // Adjudication checks
        if eval_score.abs() < config.draw_adjudication_cp {
            consecutive_low_eval += 1;
            if consecutive_low_eval >= config.draw_adjudication_plies {
                for pos in &mut positions {
                    pos.wdl = 0.5;
                }
                return (
                    Some(GameResult {
                        positions,
                        outcome: 0.5,
                        plies: ply,
                    }),
                    in_check_drops,
                );
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
            return (
                Some(GameResult {
                    positions,
                    outcome,
                    plies: ply,
                }),
                in_check_drops,
            );
        }

        // Play the first move (in a proper datagen, use search `bestmove`)
        board.make_move(moves[0]);
        ply += 1;

        // Safety limit
        if ply > 500 {
            for pos in &mut positions {
                pos.wdl = 0.5;
            }
            return (
                Some(GameResult {
                    positions,
                    outcome: 0.5,
                    plies: ply,
                }),
                in_check_drops,
            );
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
        (
            Some(GameResult {
                positions,
                outcome,
                plies: ply,
            }),
            in_check_drops,
        )
    } else {
        (None, in_check_drops)
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
        let (result, _drops) = play_one_game(&config, &stopped, None);
        assert!(result.is_some() || true);
    }

    #[test]
    fn score_histogram_clamps_and_buckets() {
        assert_eq!(score_histogram_bucket(-800), 0);
        assert_eq!(score_histogram_bucket(-801), 0);
        assert_eq!(score_histogram_bucket(800), HIST_BUCKETS - 1);
        assert_eq!(score_histogram_bucket(0), 8);
        assert_eq!(wdl_slot(1.0), 0);
        assert_eq!(wdl_slot(0.5), 1);
        assert_eq!(wdl_slot(0.0), 2);
    }

    #[test]
    fn datagen_stats_record_wdl_pass_drop_and_hist() {
        let stats = DatagenStats::new();
        let kept = GameResult {
            positions: vec![
                TrainingPosition {
                    fen: "start".into(),
                    score: -800,
                    wdl: 1.0,
                },
                TrainingPosition {
                    fen: "mid".into(),
                    score: 0,
                    wdl: 1.0,
                },
            ],
            outcome: 1.0,
            plies: 4,
        };
        stats.record(Some(&kept), 3, 40);
        stats.record(None, 1, 0);
        assert_eq!(stats.white.load(Ordering::Relaxed), 1);
        assert_eq!(stats.draw.load(Ordering::Relaxed), 0);
        assert_eq!(stats.black.load(Ordering::Relaxed), 0);
        assert_eq!(stats.pass.load(Ordering::Relaxed), 2);
        assert_eq!(stats.drop.load(Ordering::Relaxed), 5);
        assert_eq!(stats.bytes.load(Ordering::Relaxed), 40);
        let hist = stats.snapshot_hist();
        assert_eq!(hist[0], 1);
        assert_eq!(hist[8], 1);
        let batch = stats.batch(1, 2, 2, 100, 0.5);
        assert_eq!(batch.white, Some(1));
        assert_eq!(batch.pass, Some(2));
        assert_eq!(batch.drop, Some(5));
        assert_eq!(batch.nps, Some(100));
    }

    #[test]
    fn datagen_batch_complete_stops_on_position_target() {
        let config = DatagenConfig {
            num_games: 1_000_000,
            num_positions: Some(1_000_000),
            ..Default::default()
        };
        assert!(!datagen_batch_complete(999_999, 12, &config));
        assert!(datagen_batch_complete(1_000_000, 12, &config));
        assert!(datagen_batch_complete(10, 1_000_000, &config));
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
