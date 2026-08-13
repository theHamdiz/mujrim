//! Framework-free PGN, study, catalog, and tournament helpers.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mujrim_protocols::catalog::{DiscoveredEngine, RuntimeCompatibility};
use mujrim_study::annotation::{AnnotationContext, MoveAnnotation};
use mujrim_study::database::{EngineMetadata, GameQuery, GameSummary, StudyDatabase};
use mujrim_study::opening::OpeningExplorer;
use mujrim_study::tournament::TournamentFormat;
use mujrim_study::training::Puzzle;
use mujrim_study::training_store::TrainingStore;

use super::engine::{QuickTournamentEngine, bundled_engine_label};
use super::game::GameState;
use super::tournament_live::{self, LiveTournamentHandle};
use super::tournament_setup::TournamentSetup;
use super::uci_process;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnalyzedPly {
    pub annotation: MoveAnnotation,
    pub score_cp: i32,
}

pub fn strip_move_annotations(notation: &str) -> &str {
    notation.trim().trim_end_matches(['+', '#', '!', '?'])
}

pub fn normalize_logged_uci(notation: &str) -> String {
    strip_move_annotations(notation).to_ascii_lowercase()
}

pub fn find_logged_move(board: &mut types::Board, notation: &str) -> Option<types::Move> {
    let uci = normalize_logged_uci(notation);
    board
        .generate_legal_moves()
        .iter()
        .find(|mv| mv.to_uci() == uci)
        .copied()
}

pub fn review_annotation_badge(
    initial_fen: &str,
    moves: &[String],
    review_ply: Option<usize>,
    annotations: &[Option<MoveAnnotation>],
) -> Option<(types::Square, MoveAnnotation)> {
    let ply = review_ply.filter(|ply| *ply > 0)?;
    let annotation = annotations.get(ply - 1).copied().flatten()?;
    if !annotation.shows_board_badge() {
        return None;
    }
    let mut board = types::Board::from_fen(initial_fen).ok()?;
    for notation in moves.iter().take(ply - 1) {
        let mv = find_logged_move(&mut board, notation)?;
        board.make_move(mv);
    }
    let played = find_logged_move(&mut board, moves.get(ply - 1)?)?;
    Some((played.to, annotation))
}

pub fn replay_study_game(initial_fen: &str, moves: &[String]) -> Result<GameState, String> {
    types::init();
    let mut state = GameState::new(types::Board::from_fen(initial_fen)?);
    for (ply, notation) in moves.iter().enumerate() {
        let mv = find_logged_move(&mut state.board, notation)
            .ok_or_else(|| format!("illegal move {notation} at ply {}", ply + 1))?;
        state.last_move_squares = vec![mv.from, mv.to];
        state.board.make_move(mv);
    }
    state.game_over = state.board.is_game_over();
    Ok(state)
}

pub fn display_metadata<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

pub fn game_summary_label(summary: &GameSummary) -> (String, String) {
    let metadata = &summary.metadata;
    let white = display_metadata(&metadata.white, "White");
    let black = display_metadata(&metadata.black, "Black");
    let ratings = match (metadata.white_elo, metadata.black_elo) {
        (Some(white), Some(black)) => format!("{white}–{black}"),
        _ => "unrated".to_owned(),
    };
    let event = display_metadata(&metadata.event, "Casual game");
    let eco = display_metadata(&metadata.eco, "—");
    (
        format!("{white} vs {black}  {}", metadata.result),
        format!("{event} · {eco} · {ratings}"),
    )
}

pub fn study_database_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("Mujrim")
        .join("library")
}

pub fn training_database_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("Mujrim")
        .join("training")
}

pub fn apply_opening_move(board: &mut types::Board, uci: &str) -> Result<types::Move, String> {
    let mv = board
        .generate_legal_moves()
        .iter()
        .find(|mv| mv.to_uci() == uci)
        .copied()
        .ok_or_else(|| format!("Opening move '{uci}' is no longer legal."))?;
    board.make_move(mv);
    Ok(mv)
}

pub fn today_day() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() / 86_400)
}

pub fn preferred_engine_arch_folders() -> Vec<String> {
    let mut folders = Vec::with_capacity(6);
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    if std::arch::is_x86_feature_detected!("avx2") {
        folders.push("windows-x86_64-avx2".to_owned());
    }
    folders.push(format!(
        "{}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        folders.push("windows-arm64".to_owned());
        folders.push("windows-x86_64-avx2".to_owned());
        folders.push("windows-x86_64".to_owned());
    }
    folders
}

pub fn list_local_engine_binaries() -> Vec<PathBuf> {
    let Some(root) = std::env::current_exe()
        .ok()
        .as_ref()
        .and_then(|exe| mujrim_protocols::catalog::local_engines_root(exe))
    else {
        return Vec::new();
    };
    if !root.is_dir() {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    collect_engine_executables(&root, 0, &mut candidates);
    let preferred = preferred_engine_arch_folders();
    candidates.sort_by(|left, right| {
        engine_path_rank(left, &preferred)
            .cmp(&engine_path_rank(right, &preferred))
            .then_with(|| {
                let left_native = mujrim_protocols::is_host_native_binary(left);
                let right_native = mujrim_protocols::is_host_native_binary(right);
                right_native.cmp(&left_native)
            })
            .then_with(|| left.cmp(right))
    });
    let mut seen_stems = std::collections::HashSet::new();
    candidates.retain(|path| {
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        seen_stems.insert(stem)
    });
    candidates.truncate(64);
    candidates
}

pub fn engine_path_rank(path: &Path, preferred: &[String]) -> usize {
    path.components()
        .filter_map(|component| component.as_os_str().to_str())
        .find_map(|component| {
            preferred
                .iter()
                .position(|folder| folder.eq_ignore_ascii_case(component))
        })
        .unwrap_or(usize::MAX)
}

fn collect_engine_executables(root: &Path, depth: usize, output: &mut Vec<PathBuf>) {
    if depth > 5 || output.len() >= 128 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_engine_executables(&path, depth + 1, output);
        } else if is_engine_executable(&path) {
            output.push(path);
        }
    }
}

fn is_engine_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(windows)]
    {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
}

pub fn probe_engine_protocol(path: &Path) -> Option<EngineMetadata> {
    use mujrim_protocols::{EngineSession, ProtocolKind};

    const PROBE_MEMORY: u64 = 256 * 1024 * 1024;
    let protocol = if EngineSession::spawn_with_args_and_memory_limit(
        path,
        &[],
        ProtocolKind::Uci,
        Some(PROBE_MEMORY),
    )
    .is_ok()
    {
        "UCI"
    } else if EngineSession::spawn_with_args_and_memory_limit(
        path,
        &[],
        ProtocolKind::Xboard,
        Some(PROBE_MEMORY),
    )
    .is_ok()
    {
        "XBoard"
    } else {
        return None;
    };

    let name = path
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Chess engine".to_owned());
    let architecture = path
        .components()
        .rev()
        .filter_map(|component| component.as_os_str().to_str())
        .find(|component| {
            component.contains("arm64")
                || component.contains("aarch64")
                || component.contains("x86_64")
        })
        .unwrap_or(std::env::consts::ARCH)
        .to_owned();
    Some(EngineMetadata {
        path: path.to_string_lossy().into_owned(),
        name,
        protocol: protocol.to_owned(),
        architecture,
        author: String::new(),
    })
}

pub fn probe_adjacent_engines() -> Vec<EngineMetadata> {
    std::thread::Builder::new()
        .name("mujrim-engine-discovery".to_owned())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            list_local_engine_binaries()
                .into_iter()
                .filter_map(|path| probe_engine_protocol(&path))
                .collect()
        })
        .ok()
        .and_then(|worker| worker.join().ok())
        .unwrap_or_default()
}

pub fn index_openings(path: PathBuf) -> (OpeningExplorer, usize) {
    std::thread::Builder::new()
        .name("mujrim-opening-index".to_owned())
        .stack_size(4 * 1024 * 1024)
        .spawn(move || {
            let Ok(database) = StudyDatabase::open(path) else {
                return (OpeningExplorer::default(), 0);
            };
            let summaries = database.search(&GameQuery::default());
            let mut explorer = OpeningExplorer::default();
            let mut indexed = 0;
            for summary in summaries.iter().take(5_000) {
                let Ok(game) = database.load_game(&summary.id) else {
                    continue;
                };
                let plies = game.moves.len().min(24);
                if explorer
                    .record_game(&game.initial_fen, &game.moves[..plies], &game.result)
                    .is_ok()
                {
                    indexed += 1;
                }
            }
            (explorer, indexed)
        })
        .ok()
        .and_then(|worker| worker.join().ok())
        .unwrap_or_default()
}

pub fn starter_puzzles() -> Vec<Puzzle> {
    vec![
        Puzzle {
            id: "starter-development".to_owned(),
            fen: mujrim_study::opening::START_FEN.to_owned(),
            solution: vec!["e2e4".to_owned(), "e7e5".to_owned()],
            themes: vec!["opening fundamentals".to_owned()],
            rating: 600,
        },
        Puzzle {
            id: "starter-mate-white".to_owned(),
            fen: "r1bqkbnr/pppp1ppp/2n5/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 3".to_owned(),
            solution: vec!["h5f7".to_owned()],
            themes: vec!["mate in one".to_owned()],
            rating: 750,
        },
        Puzzle {
            id: "starter-mate-black".to_owned(),
            fen: "rnbqkbnr/pppp1ppp/4p3/8/6P1/5P2/PPPPP2P/RNBQKBNR b KQkq - 0 2".to_owned(),
            solution: vec!["d8h4".to_owned()],
            themes: vec!["mate in one".to_owned(), "king safety".to_owned()],
            rating: 800,
        },
    ]
}

pub fn seed_training(store: &mut TrainingStore) -> Result<usize, String> {
    let mut added = 0;
    for puzzle in starter_puzzles() {
        if store.add(puzzle)? {
            added += 1;
        }
    }
    Ok(added)
}

pub fn path_is_under_local_engines(path: &Path) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Some(root) = mujrim_protocols::catalog::local_engines_root(&exe) else {
        return false;
    };
    let root = root.canonicalize().unwrap_or(root);
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    path.starts_with(&root)
}

pub fn catalog_display_name(stem: &str, bundled: &[DiscoveredEngine]) -> String {
    if let Some(engine) = bundled.iter().find(|engine| {
        engine.id.eq_ignore_ascii_case(stem)
            || engine
                .path
                .file_stem()
                .is_some_and(|name| name.eq_ignore_ascii_case(stem))
    }) {
        return if engine.compatibility == RuntimeCompatibility::Emulated
            || !mujrim_protocols::is_host_native_binary(&engine.path)
        {
            bundled_engine_label(engine)
        } else {
            engine.display_name.to_owned()
        };
    }
    for &(id, display) in mujrim_protocols::catalog::BUNDLED_ENGINES {
        if id.eq_ignore_ascii_case(stem) {
            return display.to_owned();
        }
    }
    stem.to_owned()
}

pub fn tournament_engine_roster(
    bundled: &[DiscoveredEngine],
    discovered: &[EngineMetadata],
) -> Vec<QuickTournamentEngine> {
    let mut roster = Vec::new();
    let mut seen_paths = std::collections::HashSet::new();
    let mut seen_stems = std::collections::HashSet::new();
    for path in list_local_engine_binaries() {
        if !path_is_under_local_engines(&path) || !seen_paths.insert(path.clone()) {
            continue;
        }
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "engine".to_owned());
        if !seen_stems.insert(stem.clone()) {
            continue;
        }
        let mut name = catalog_display_name(&stem, bundled);
        if !mujrim_protocols::is_host_native_binary(&path)
            && !name.to_ascii_lowercase().contains("emulation")
            && !name.to_ascii_lowercase().contains("x64")
        {
            name = format!("{name} (x64 emulation)");
        }
        let search_limits = bundled
            .iter()
            .find(|engine| engine.path == path || engine.id.eq_ignore_ascii_case(&stem))
            .map(|engine| engine.search_limits)
            .unwrap_or(mujrim_protocols::catalog::SearchLimitSupport::STANDARD);
        roster.push(QuickTournamentEngine {
            name,
            path,
            search_limits,
        });
    }
    for engine in discovered {
        let path = PathBuf::from(&engine.path);
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| engine.name.clone());
        if engine.protocol.eq_ignore_ascii_case("UCI")
            && path.is_file()
            && path_is_under_local_engines(&path)
            && seen_paths.insert(path.clone())
            && seen_stems.insert(stem)
        {
            roster.push(QuickTournamentEngine {
                name: engine.name.clone(),
                path,
                search_limits: mujrim_protocols::catalog::SearchLimitSupport::STANDARD,
            });
        }
    }
    roster.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    roster
}

pub fn run_quick_tournament(
    engines: Vec<QuickTournamentEngine>,
    setup: TournamentSetup,
    handle: LiveTournamentHandle,
) -> mujrim_benchmarker::strength::TournamentSummary {
    let cancel = Arc::clone(&handle.cancel);
    let snapshot = Arc::clone(&handle.snapshot);
    let format = setup.format;
    let worker = std::thread::Builder::new()
        .name("mujrim-tournament".to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_quick_tournament_body(engines, setup, cancel, snapshot)
            }))
            .unwrap_or_else(|_| mujrim_benchmarker::strength::TournamentSummary {
                format,
                engines: Vec::new(),
                matches: Vec::new(),
                standings: Vec::new(),
                game_results: Vec::new(),
                cancelled: false,
                error: Some(
                    "Tournament worker panicked. The UI stayed up — check engine compatibility (Arm64/Prism) and try fewer engines."
                        .to_owned(),
                ),
            })
        });
    match worker {
        Ok(worker) => match worker.join() {
            Ok(summary) => summary,
            Err(_) => mujrim_benchmarker::strength::TournamentSummary {
                format,
                engines: Vec::new(),
                matches: Vec::new(),
                standings: Vec::new(),
                game_results: Vec::new(),
                cancelled: false,
                error: Some("Tournament worker failed unexpectedly.".to_owned()),
            },
        },
        Err(error) => mujrim_benchmarker::strength::TournamentSummary {
            format,
            engines: Vec::new(),
            matches: Vec::new(),
            standings: Vec::new(),
            game_results: Vec::new(),
            cancelled: false,
            error: Some(format!("Could not start tournament worker: {error}")),
        },
    }
}

fn run_quick_tournament_body(
    engines: Vec<QuickTournamentEngine>,
    setup: TournamentSetup,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    snapshot: Arc<std::sync::Mutex<tournament_live::LiveTournamentSnapshot>>,
) -> mujrim_benchmarker::strength::TournamentSummary {
    use mujrim_benchmarker::strength::{
        EngineSpec, TournamentConfig, TournamentEngine, TournamentEvent, TournamentProgress,
        run_tournament_with_control,
    };

    let format = setup.format;
    let roster: Vec<TournamentEngine> = engines
        .into_iter()
        .map(|engine| {
            let mut spec = EngineSpec::new(engine.path.clone());
            spec.name = engine.name;
            spec.uci_options = uci_process::uci_resource_options(&engine.path, false, true, None);
            TournamentEngine {
                engine: spec,
                established_elo: None,
                search_limits: engine.search_limits,
            }
        })
        .collect();
    let initial_clock_ms = setup.time_control.match_clock().initial.as_millis() as u64;
    let progress: TournamentProgress = Arc::new({
        let snapshot = Arc::clone(&snapshot);
        move |event: TournamentEvent| {
            let snapshot = Arc::clone(&snapshot);
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let Ok(mut guard) = snapshot.lock() else {
                    return;
                };
                match event {
                    TournamentEvent::Planned {
                        total_matches,
                        engine_names,
                    } => {
                        guard.total_matches = total_matches;
                        guard.engine_names = engine_names;
                        guard.status_line =
                            format!("Scheduled {total_matches} pairings. Starting…");
                    }
                    TournamentEvent::MatchStarted {
                        index,
                        total,
                        round,
                        white,
                        black,
                    } => {
                        guard.total_matches = total.max(guard.total_matches);
                        guard.current_round = round;
                        guard.current_white = white.clone();
                        guard.current_black = black.clone();
                        guard.status_line =
                            format!("Playing {index}/{total} · Round {round} · {white} vs {black}");
                    }
                    TournamentEvent::GameStarted {
                        game_key,
                        match_index,
                        round,
                        white,
                        black,
                        initial_fen,
                    } => {
                        guard.upsert_live_game(tournament_live::LiveGameBoard {
                            game_key,
                            match_index,
                            round,
                            white: white.clone(),
                            black: black.clone(),
                            initial_fen,
                            moves: Vec::new(),
                            last_uci: String::new(),
                            score_cp: 0,
                            depth: 0,
                            nodes: 0,
                            white_clock_ms: Some(initial_clock_ms),
                            black_clock_ms: Some(initial_clock_ms),
                        });
                        guard.current_round = round;
                        guard.current_white = white;
                        guard.current_black = black;
                    }
                    TournamentEvent::PlyPlayed {
                        game_key,
                        ply,
                        uci,
                        score_cp,
                        depth,
                        nodes,
                        moves,
                        white_clock_ms,
                        black_clock_ms,
                    } => {
                        guard.apply_ply(
                            &game_key,
                            ply,
                            uci,
                            score_cp,
                            depth,
                            nodes,
                            moves,
                            white_clock_ms,
                            black_clock_ms,
                        );
                    }
                    TournamentEvent::GameFinished {
                        game_key,
                        white_score,
                        moves,
                    } => {
                        guard.finish_live_game(&game_key, white_score, moves);
                    }
                    TournamentEvent::MatchFinished {
                        index,
                        total,
                        round,
                        white,
                        black,
                        white_points,
                        black_points,
                        error,
                        standings,
                        game_results,
                        games,
                    } => {
                        guard.completed_matches = index;
                        guard.total_matches = total.max(guard.total_matches);
                        guard
                            .finished_matches
                            .push(tournament_live::FinishedMatchRow {
                                index,
                                round,
                                white: white.clone(),
                                black: black.clone(),
                                white_points,
                                black_points,
                                error: error.clone(),
                            });
                        guard.standings =
                            tournament_live::standing_rows(&guard.engine_names, &standings);
                        guard.game_results = game_results;
                        let already_live = guard
                            .played_games
                            .iter()
                            .any(|game| game.match_index == index);
                        if !already_live {
                            guard.append_games(games);
                        }
                        guard.current_white.clear();
                        guard.current_black.clear();
                        guard.status_line = if let Some(error) = error {
                            format!("Match {index}/{total}: {error}")
                        } else {
                            format!(
                                "Finished {index}/{total} · {white} {} {}",
                                tournament_live::score_label(white_points, black_points),
                                black
                            )
                        };
                    }
                    TournamentEvent::Cancelled {
                        standings,
                        game_results,
                    } => {
                        guard.cancelled = true;
                        guard.running = false;
                        guard.standings =
                            tournament_live::standing_rows(&guard.engine_names, &standings);
                        guard.game_results = game_results;
                        guard.status_line =
                            "Tournament cancelled. Partial standings are available.".to_owned();
                    }
                }
            }));
        }
    });
    let mut match_config = setup.to_match_config();
    match_config.stop_flag = Some(Arc::clone(&cancel));
    let summary = run_tournament_with_control(
        roster,
        TournamentConfig {
            match_config,
            format,
            swiss_rounds: matches!(format, TournamentFormat::Swiss)
                .then_some(setup.swiss_rounds.max(1) as usize),
            checkpoint_directory: study_database_path().parent().map(|path| {
                path.join("tournaments")
                    .join(tournament_directory_name(format))
            }),
        },
        cancel,
        Some(progress),
    );
    if let Ok(mut guard) = snapshot.lock() {
        guard.running = false;
        guard.finished = true;
        guard.cancelled = summary.cancelled;
        guard.error = summary.error.clone();
        let names = summary
            .engines
            .iter()
            .map(|engine| engine.engine.name.clone())
            .collect::<Vec<_>>();
        guard.engine_names = names.clone();
        guard.standings = tournament_live::standing_rows(&names, &summary.standings);
        guard.game_results = summary.game_results.clone();
        let games = mujrim_benchmarker::strength::games_from_summary(&summary);
        if guard.played_games.len() < games.len() {
            guard.played_games.clear();
            guard.append_games(games);
        }
    }
    summary
}

pub fn tournament_directory_name(format: TournamentFormat) -> &'static str {
    match format {
        TournamentFormat::RoundRobin => "round-robin",
        TournamentFormat::DoubleRoundRobin => "double-round-robin",
        TournamentFormat::Swiss => "swiss",
        TournamentFormat::Knockout => "knockout",
    }
}

pub fn format_tournament_summary(
    summary: &mujrim_benchmarker::strength::TournamentSummary,
) -> String {
    let podium = summary
        .standings
        .iter()
        .take(3)
        .enumerate()
        .filter_map(|(rank, standing)| {
            let name = summary
                .engines
                .get(standing.entrant)
                .map(|engine| engine.engine.name.as_str())?;
            let rating = standing.performance.map_or_else(
                || "rating pending".to_owned(),
                |estimate| format!("{:.0} Elo", estimate.elo),
            );
            Some(format!(
                "{}. {name} — {:.1} points, {rating}",
                rank + 1,
                standing.points
            ))
        })
        .collect::<Vec<_>>()
        .join("  ·  ");
    if podium.is_empty() {
        format!("{} finished without completed games.", summary.format)
    } else {
        format!("{} · {podium}", summary.format)
    }
}

pub fn analyze_game_at_depth_from(
    initial_fen: &str,
    moves: &[String],
    depth: i32,
) -> Result<Vec<AnalyzedPly>, String> {
    types::init();
    let mut board = types::Board::from_fen(initial_fen)
        .map_err(|error| format!("invalid initial position: {error}"))?;
    let mut engine = search::SearchEngine::new(32, 1);
    let mut analysis = Vec::with_capacity(moves.len());
    for (ply, notation) in moves.iter().enumerate() {
        let legal_moves = board.generate_legal_moves();
        let played = find_logged_move(&mut board, notation).ok_or_else(|| {
            format!(
                "illegal move '{}' at ply {}",
                normalize_logged_uci(notation),
                ply + 1
            )
        })?;
        let moving_value = board
            .piece_on(played.from)
            .map_or(0, |(piece, _)| piece_value(piece));
        let captured_value = board
            .piece_on(played.to)
            .map_or(0, |(piece, _)| piece_value(piece));
        let mut before = board.clone();
        let best = engine.search_depth(&mut before, depth.max(1));
        let is_best_move = best.best_move.from == played.from
            && best.best_move.to == played.to
            && best.best_move.promotion == played.promotion;
        let mut after = board.clone();
        after.make_move(played);
        let can_be_recaptured = after
            .generate_legal_moves()
            .iter()
            .any(|reply| reply.to == played.to && reply.is_capture());
        let reply = engine.search_depth(&mut after, depth.max(1));
        let played_score = reply.score.saturating_neg();
        let annotation = AnnotationContext {
            best_score_cp: best.score,
            played_score_cp: played_score,
            second_best_score_cp: None,
            is_sacrifice: moving_value >= 300
                && captured_value.saturating_add(100) < moving_value
                && can_be_recaptured,
            is_best_move,
            is_only_move: legal_moves.len() == 1,
            position_in_opening_database: false,
            move_in_opening_database: false,
        }
        .classify();
        analysis.push(AnalyzedPly {
            annotation,
            score_cp: if ply % 2 == 0 {
                played_score
            } else {
                played_score.saturating_neg()
            },
        });
        board.make_move(played);
    }
    Ok(analysis)
}

#[cfg(test)]
pub fn analyze_game_at_depth(moves: &[String], depth: i32) -> Result<Vec<AnalyzedPly>, String> {
    analyze_game_at_depth_from(mujrim_study::opening::START_FEN, moves, depth)
}

pub fn board_at_ply(
    initial_fen: &str,
    moves: &[String],
    ply: usize,
) -> Result<types::Board, String> {
    if ply > moves.len() {
        return Err(format!("ply {ply} is beyond the {}-ply game", moves.len()));
    }
    let state = replay_study_game(initial_fen, &moves[..ply])?;
    Ok(state.board)
}

const fn piece_value(piece: types::Piece) -> i32 {
    match piece {
        types::Piece::Pawn => 100,
        types::Piece::Knight => 320,
        types::Piece::Bishop => 330,
        types::Piece::Rook => 500,
        types::Piece::Queen => 900,
        types::Piece::King => 20_000,
    }
}

pub fn annotated_move_label(notation: &str, annotation: Option<MoveAnnotation>) -> String {
    annotation.map_or_else(
        || notation.to_owned(),
        |annotation| {
            let symbol = annotation.symbol();
            if symbol.is_empty() {
                notation.to_owned()
            } else {
                format!("{notation} {symbol}")
            }
        },
    )
}

pub fn puzzle_line_matches(played: &[String], solution: &[String]) -> bool {
    played.len() == solution.len()
        && played
            .iter()
            .zip(solution)
            .all(|(played, expected)| played.trim_end_matches(['+', '#']) == expected)
}

pub fn build_pgn(white: &str, black: &str, moves: &[String], result: &str) -> String {
    let mut pgn = format!(
        "[Event \"Mujrim Game\"]\n[Site \"Local\"]\n[Date \"????.??.??\"]\n[White \"{white}\"]\n[Black \"{black}\"]\n[Result \"{result}\"]\n\n"
    );
    for (index, pair) in moves.chunks(2).enumerate() {
        pgn.push_str(&format!(
            "{}. {}",
            index + 1,
            strip_move_annotations(&pair[0])
        ));
        if pair.len() > 1 {
            pgn.push(' ');
            pgn.push_str(strip_move_annotations(&pair[1]));
        }
        pgn.push(' ');
    }
    pgn.push_str(result);
    pgn
}

pub fn build_annotated_pgn(
    white: &str,
    black: &str,
    moves: &[String],
    annotations: &[Option<MoveAnnotation>],
    result: &str,
) -> String {
    let mut pgn = format!(
        "[Event \"Mujrim Game\"]\n[Site \"Local\"]\n[Date \"????.??.??\"]\n[White \"{white}\"]\n[Black \"{black}\"]\n[Result \"{result}\"]\n\n"
    );
    for (index, pair) in moves.chunks(2).enumerate() {
        let white_ann = annotations.get(index * 2).copied().flatten();
        pgn.push_str(&format!(
            "{}. {}",
            index + 1,
            annotated_move_label(&pair[0], white_ann)
        ));
        if pair.len() > 1 {
            let black_ann = annotations.get(index * 2 + 1).copied().flatten();
            pgn.push(' ');
            pgn.push_str(&annotated_move_label(&pair[1], black_ann));
        }
        pgn.push(' ');
    }
    pgn.push_str(result);
    pgn
}

#[cfg(test)]
mod tests {
    use super::*;
    use mujrim_study::annotation::MoveAnnotation;
    use mujrim_study::database::{GameMetadata, GameSummary};

    #[test]
    fn pgn_builder_numbers_moves_and_preserves_result() {
        let pgn = build_pgn(
            "White",
            "Black",
            &["e4".to_owned(), "e5".to_owned(), "Nf3".to_owned()],
            "1-0",
        );
        assert!(pgn.contains("[White \"White\"]"));
        assert!(pgn.ends_with("1. e4 e5 2. Nf3 1-0"));
    }

    #[test]
    fn review_badge_uses_destination_square_and_classification() {
        types::init();
        let badge = review_annotation_badge(
            mujrim_study::opening::START_FEN,
            &["e2e4".to_owned()],
            Some(1),
            &[Some(MoveAnnotation::Brilliant)],
        )
        .expect("badge");
        assert_eq!(badge.0, types::Square::from_index(28));
        assert_eq!(badge.1, MoveAnnotation::Brilliant);
    }

    #[test]
    fn all_tournament_formats_have_stable_checkpoint_directories() {
        let names = TournamentFormat::ALL.map(tournament_directory_name);
        assert_eq!(
            names,
            ["round-robin", "double-round-robin", "swiss", "knockout"]
        );
    }

    #[test]
    fn empty_tournament_summary_reports_the_selected_format() {
        let summary = mujrim_benchmarker::strength::TournamentSummary {
            format: TournamentFormat::Swiss,
            engines: Vec::new(),
            matches: Vec::new(),
            standings: Vec::new(),
            game_results: Vec::new(),
            cancelled: false,
            error: None,
        };
        assert_eq!(
            format_tournament_summary(&summary),
            "Swiss finished without completed games."
        );
    }

    #[test]
    fn engine_path_rank_prefers_primary_host_arch_folder() {
        let preferred = vec!["windows-aarch64".to_owned(), "windows-arm64".to_owned()];
        let primary =
            PathBuf::from(r"C:\Mujrim\engines\mujrim\bin\windows-aarch64\mujrim-elite.exe");
        let alias = PathBuf::from(r"C:\Mujrim\engines\mujrim\bin\windows-arm64\mujrim-elite.exe");
        assert!(engine_path_rank(&primary, &preferred) < engine_path_rank(&alias, &preferred));
    }

    #[test]
    fn starter_puzzles_cover_opening_and_mates() {
        let puzzles = starter_puzzles();
        assert_eq!(puzzles.len(), 3);
        assert_eq!(puzzles[0].solution[0], "e2e4");
        assert_eq!(puzzles[1].solution[0], "h5f7");
        assert_eq!(puzzles[2].solution[0], "d8h4");
    }

    fn game_summary_label_uses_players_and_result() {
        let summary = GameSummary {
            id: "1".into(),
            metadata: GameMetadata {
                white: "A".into(),
                black: "B".into(),
                result: "1-0".into(),
                event: "Test".into(),
                eco: "C20".into(),
                white_elo: Some(2000),
                black_elo: Some(1900),
                ..GameMetadata::default()
            },
        };
        let (title, detail) = game_summary_label(&summary);
        assert!(title.contains("A vs B"));
        assert!(detail.contains("C20"));
    }
}
