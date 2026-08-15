//! Framework-free PGN, study, catalog, and tournament helpers.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mujrim_protocols::catalog::{DiscoveredEngine, RuntimeCompatibility};
use mujrim_study::annotation::{AnnotationContext, MoveAnnotation};
use mujrim_study::database::{EngineMetadata, GameQuery, GameSummary, StudyDatabase};
use mujrim_study::game_export::{self, GameExportFormat, GameRecord};
use mujrim_study::opening::{MoveStatistics, OpeningExplorer, PrepSide, SavedLine};
use mujrim_study::tournament::{Entrant, TournamentFormat};
use mujrim_study::tournament_store::{StoredTournament, StoredTournamentGame};
use mujrim_study::training::Puzzle;
use mujrim_study::training_store::TrainingStore;

use super::engine::{GameMode, PlayerConfig, QuickTournamentEngine, bundled_engine_label};
use super::game::GameState;
use super::tournament_live::{self, LiveTournamentHandle};
use super::tournament_setup::TournamentSetup;
use super::uci_process::{self, ExternalEngineProtocol};

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

pub fn apply_uci_move(board: &mut types::Board, uci: &str) -> Result<types::Move, String> {
    let mv = find_logged_move(board, uci)
        .ok_or_else(|| format!("Opening move '{uci}' is no longer legal."))?;
    board.make_move(mv);
    Ok(mv)
}

pub fn opening_white_score(stats: &MoveStatistics) -> u64 {
    stats
        .white_wins
        .saturating_mul(100)
        .saturating_add(stats.draws.saturating_mul(50))
        .checked_div(stats.games)
        .unwrap_or(0)
}

pub fn displayed_study_fen(
    initial_fen: &str,
    moves: &[String],
    review_ply: Option<usize>,
    live_fen: Option<String>,
) -> String {
    let ply = review_ply.unwrap_or(moves.len());
    board_at_ply(initial_fen, moves, ply.min(moves.len()))
        .map(|board| board.to_fen())
        .ok()
        .or(live_fen)
        .unwrap_or_else(|| mujrim_study::opening::START_FEN.to_owned())
}

pub fn san_annotated_moves(
    initial_fen: &str,
    moves: &[String],
    annotations: &[Option<MoveAnnotation>],
) -> Vec<String> {
    types::init();
    let Ok(mut board) = types::Board::from_fen(initial_fen) else {
        return moves
            .iter()
            .enumerate()
            .map(|(index, notation)| {
                annotated_move_label(notation, annotations.get(index).copied().flatten())
            })
            .collect();
    };
    let mut labels = Vec::with_capacity(moves.len());
    for (index, uci) in moves.iter().enumerate() {
        let san = uci_to_san(&board, uci);
        labels.push(annotated_move_label(
            &san,
            annotations.get(index).copied().flatten(),
        ));
        let _ = apply_uci_move(&mut board, uci);
    }
    labels
}

pub fn uci_to_san(board: &types::Board, uci: &str) -> String {
    let Some(mv) = find_logged_move(&mut board.clone(), uci) else {
        return uci.to_owned();
    };
    match mv.flag {
        types::chess_move::MoveFlag::KingCastle => return "O-O".to_owned(),
        types::chess_move::MoveFlag::QueenCastle => return "O-O-O".to_owned(),
        _ => {}
    }
    let Some((piece, _)) = board.piece_on(mv.from) else {
        return uci.to_owned();
    };
    let dest = mv.to.to_string();
    let capture = mv.is_capture() || board.piece_on(mv.to).is_some();
    let promo = mv
        .promotion
        .map(|piece| format!("={}", piece.to_char().to_ascii_uppercase()))
        .unwrap_or_default();
    match piece {
        types::Piece::Pawn => {
            if capture {
                format!("{}x{dest}{promo}", file_char(mv.from))
            } else {
                format!("{dest}{promo}")
            }
        }
        _ => {
            let letter = piece.to_char().to_ascii_uppercase();
            if capture {
                format!("{letter}x{dest}{promo}")
            } else {
                format!("{letter}{dest}{promo}")
            }
        }
    }
}

fn file_char(square: types::Square) -> char {
    (b'a' + square.file()) as char
}

pub fn save_current_line(
    name: String,
    side: PrepSide,
    initial_fen: String,
    moves: Vec<String>,
    notes: String,
) -> Result<SavedLine, String> {
    let line = SavedLine::from_current(name, side, initial_fen, moves, notes);
    line.to_repertoire().validate(&line.initial_fen)?;
    Ok(line)
}

pub fn next_incremental_uci(previous: &[String], next: &[String]) -> Option<String> {
    if next.len() == previous.len() + 1 && next.starts_with(previous) {
        next.last().cloned()
    } else {
        None
    }
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
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    list_engine_binaries_in_roots(&mujrim_protocols::catalog::engine_search_roots(&exe, &cwd))
}

pub fn list_engine_binaries_in_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for root in roots {
        if root.is_dir() {
            collect_engine_executables(root, 0, &mut candidates);
        }
    }
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
    candidates.retain(|path| !is_product_engine_binary(path));
    let mut seen_ids = std::collections::HashSet::new();
    candidates.retain(|path| seen_ids.insert(engine_identity_key(path)));
    candidates.truncate(64);
    candidates
}

pub fn engine_identity_key(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
        .to_ascii_lowercase();
    mujrim_protocols::catalog::canonical_engine_id(&stem).to_ascii_lowercase()
}

pub fn is_product_engine_binary(path: &Path) -> bool {
    matches!(
        engine_identity_key(path).as_str(),
        "mujrim-ui"
            | "mujrim-updater"
            | "mujrim-tooling"
            | "mujrim-benchmarker"
            | "mujrim-external"
    )
}

pub fn engine_is_selected(selected: &[PathBuf], path: &Path) -> bool {
    let key = engine_identity_key(path);
    selected.iter().any(|item| engine_identity_key(item) == key)
}

pub fn toggle_engine_selection(selected: &mut Vec<PathBuf>, path: PathBuf) {
    let key = engine_identity_key(&path);
    if selected.iter().any(|item| engine_identity_key(item) == key) {
        selected.retain(|item| engine_identity_key(item) != key);
    } else {
        selected.push(path);
    }
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
    if !path.is_file() || is_engine_sidecar(path) {
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

fn is_engine_sidecar(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    name.ends_with(".pb.gz")
        || name.ends_with(".nnue")
        || name.ends_with(".json")
        || name.ends_with(".txt")
        || name.contains("weights")
}

pub fn path_is_under_local_engines(path: &Path) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    path_is_under_engine_roots(
        path,
        &mujrim_protocols::catalog::engine_search_roots(&exe, &cwd),
    )
}

pub fn path_is_under_engine_roots(path: &Path, roots: &[PathBuf]) -> bool {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    roots.iter().any(|root| {
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        path.starts_with(&root)
    })
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
    let mut seen_ids = std::collections::HashSet::new();
    for path in list_local_engine_binaries() {
        if !path_is_under_local_engines(&path) {
            continue;
        }
        let path_key = path.canonicalize().unwrap_or_else(|_| path.clone());
        let identity = engine_identity_key(&path);
        if !seen_paths.insert(path_key) || !seen_ids.insert(identity) {
            continue;
        }
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "engine".to_owned());
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
        let identity = engine_identity_key(&path);
        if engine.protocol.eq_ignore_ascii_case("UCI")
            && path.is_file()
            && path_is_under_local_engines(&path)
            && seen_paths.insert(path.canonicalize().unwrap_or_else(|_| path.clone()))
            && seen_ids.insert(identity)
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

pub fn eval_bar_engine_choices(
    bundled: &[DiscoveredEngine],
    catalog: &[EngineMetadata],
) -> Vec<crate::app_core::settings::EvalBarEngineChoice> {
    use crate::app_core::settings::{EVAL_BAR_DEFAULT_ENGINE, EvalBarEngineChoice};
    let mut choices = vec![EvalBarEngineChoice {
        id: EVAL_BAR_DEFAULT_ENGINE.to_owned(),
        label: "Mujrim v60".to_owned(),
    }];
    for engine in tournament_engine_roster(bundled, catalog) {
        let id = engine_identity_key(&engine.path);
        if choices.iter().any(|choice| choice.id == id) {
            continue;
        }
        choices.push(EvalBarEngineChoice {
            id,
            label: engine.name,
        });
    }
    choices
}

pub fn resolve_eval_bar_engine_path(
    id: &str,
    bundled: &[DiscoveredEngine],
    catalog: &[EngineMetadata],
) -> Option<PathBuf> {
    tournament_engine_roster(bundled, catalog)
        .into_iter()
        .find(|engine| engine_identity_key(&engine.path) == id)
        .map(|engine| engine.path)
}

pub fn engine_player_from_roster(
    roster: &[QuickTournamentEngine],
    index: usize,
    builtin_depth: i32,
) -> PlayerConfig {
    roster
        .get(index)
        .map(|engine| PlayerConfig::External {
            path: engine.path.to_string_lossy().into_owned(),
            protocol: ExternalEngineProtocol::Uci,
        })
        .unwrap_or(PlayerConfig::BuiltIn {
            depth: builtin_depth,
        })
}

pub fn players_for_detected_engines(
    mode: GameMode,
    bundled: &[DiscoveredEngine],
    catalog: &[EngineMetadata],
) -> (PlayerConfig, PlayerConfig) {
    let roster = tournament_engine_roster(bundled, catalog);
    match mode {
        GameMode::HumanVsHuman => (PlayerConfig::Human, PlayerConfig::Human),
        GameMode::HumanVsEngine => (
            PlayerConfig::Human,
            engine_player_from_roster(&roster, 0, 16),
        ),
        GameMode::EngineVsEngine => (
            engine_player_from_roster(&roster, 0, 16),
            engine_player_from_roster(&roster, 1, 12),
        ),
    }
}

pub fn default_tournament_engine_paths(roster: &[QuickTournamentEngine]) -> Vec<PathBuf> {
    roster
        .iter()
        .take(crate::app_core::tournament_setup::GUI_TOURNAMENT_DEFAULT_ENGINES)
        .map(|engine| engine.path.clone())
        .collect()
}

pub fn gui_safe_engine_args(path: &Path) -> Vec<String> {
    resolved_engine_launch(path).1
}

/// Mujrim product wrappers already pick a backend. Only raw `lc0` gets launch argv.
pub fn resolved_engine_launch(path: &Path) -> (PathBuf, Vec<String>) {
    let key = engine_identity_key(path);
    if is_official_lc0_identity(&key) {
        let launch = mujrim_protocols::plan_launch(path, mujrim_protocols::detect_device_kind());
        let args = launch.argv();
        return (launch.binary, args);
    }
    if is_mujrim_lc0_identity(&key) {
        if let Some(official) = official_lc0_for_wrapper(path) {
            let launch =
                mujrim_protocols::plan_launch(&official, mujrim_protocols::detect_device_kind());
            let args = launch.argv();
            return (launch.binary, args);
        }
        return (path.to_path_buf(), Vec::new());
    }
    (path.to_path_buf(), Vec::new())
}

fn is_official_lc0_identity(key: &str) -> bool {
    !key.contains("mujrim") && (key == "lc0" || key.contains("lc0") || key.contains("leela"))
}

fn is_mujrim_lc0_identity(key: &str) -> bool {
    key.contains("mujrim") && (key.contains("lc0") || key.contains("leela"))
}

fn official_lc0_for_wrapper(wrapper: &Path) -> Option<PathBuf> {
    let parent = wrapper.parent()?;
    ["lc0", "lc0.exe"].into_iter().find_map(|name| {
        let candidate = parent.join(name);
        candidate.is_file().then_some(candidate)
    })
}

pub fn run_quick_tournament(
    engines: Vec<QuickTournamentEngine>,
    setup: TournamentSetup,
    handle: LiveTournamentHandle,
) -> mujrim_benchmarker::strength::TournamentSummary {
    let cancel = Arc::clone(&handle.cancel);
    let pause = Arc::clone(&handle.pause);
    let abort_game = Arc::clone(&handle.abort_game);
    let snapshot = Arc::clone(&handle.snapshot);
    let format = setup.format;
    let worker = std::thread::Builder::new()
        .name("mujrim-tournament".to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_quick_tournament_body(engines, setup, cancel, pause, abort_game, snapshot)
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
    pause: Arc<std::sync::atomic::AtomicBool>,
    abort_game: Arc<std::sync::atomic::AtomicBool>,
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
            let (path, args) = resolved_engine_launch(&engine.path);
            let mut spec = EngineSpec::new(path.clone());
            spec.name = engine.name;
            spec.args = args;
            spec.uci_options = uci_process::uci_resource_options(&path, false, true, None);
            let established_elo = mujrim_study::rating::seed_elo_for_engine(&spec.name);
            TournamentEngine {
                engine: spec,
                established_elo,
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
                            clock_synced_ms: Some(tournament_live::now_unix_ms()),
                            ..tournament_live::LiveGameBoard::default()
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
                    TournamentEvent::Thinking {
                        game_key,
                        score_cp,
                        depth,
                        nodes,
                        pv,
                        multipv_lines,
                        white_clock_ms,
                        black_clock_ms,
                    } => {
                        guard.apply_thinking(
                            &game_key,
                            score_cp,
                            depth,
                            nodes,
                            pv,
                            multipv_lines
                                .into_iter()
                                .map(|line| tournament_live::ThinkingLine {
                                    multipv: line.multipv,
                                    score_cp: line.score,
                                    pv: line.pv,
                                })
                                .collect(),
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
                        guard.refresh_standings();
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
                        standings: _,
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
                        guard.game_results = game_results;
                        let already_live = guard
                            .played_games
                            .iter()
                            .any(|game| game.match_index == index);
                        if !already_live {
                            guard.append_games(games);
                        }
                        guard.refresh_standings();
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
                        standings: _,
                        game_results,
                    } => {
                        guard.cancelled = true;
                        guard.running = false;
                        guard.paused = false;
                        guard.game_results = game_results;
                        guard.refresh_standings();
                        guard.status_line =
                            "Tournament cancelled. Partial standings are available.".to_owned();
                    }
                }
            }));
        }
    });
    let mut match_config = setup.to_match_config();
    match_config.stop_flag = Some(Arc::clone(&cancel));
    match_config.pause_flag = Some(pause);
    match_config.abort_game_flag = Some(abort_game);
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
            completed_pairings: setup.completed_pairings.clone(),
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
        guard.engine_names = names;
        guard.game_results = summary.game_results.clone();
        let games = mujrim_benchmarker::strength::games_from_summary(&summary);
        if guard.played_games.len() < games.len() {
            guard.played_games.clear();
            guard.append_games(games);
        }
        guard.refresh_standings();
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

pub fn ply_chip_label(
    notation: &str,
    annotation: Option<MoveAnnotation>,
    score_cp: Option<i32>,
) -> String {
    let mut label = annotated_move_label(notation, annotation);
    if let Some(score) = score_cp {
        label.push_str("  ");
        label.push_str(&eval_label(score));
    }
    label
}

pub fn move_list_chip_labels(
    initial_fen: &str,
    moves: &[String],
    annotations: &[Option<MoveAnnotation>],
    scores: &[Option<i32>],
) -> Vec<String> {
    san_annotated_moves(initial_fen, moves, annotations)
        .into_iter()
        .enumerate()
        .map(
            |(index, label)| match scores.get(index).copied().flatten() {
                Some(score) => format!("{label}  {}", eval_label(score)),
                None => label,
            },
        )
        .collect()
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

pub fn current_game_record(
    white: &str,
    black: &str,
    event: &str,
    site: &str,
    initial_fen: &str,
    moves: &[String],
    result: &str,
) -> GameRecord {
    GameRecord {
        event: if event.trim().is_empty() {
            "Mujrim Game".to_owned()
        } else {
            event.to_owned()
        },
        site: if site.trim().is_empty() {
            "Local".to_owned()
        } else {
            site.to_owned()
        },
        date: String::new(),
        round: String::new(),
        white: white.to_owned(),
        black: black.to_owned(),
        result: result.to_owned(),
        initial_fen: if initial_fen.is_empty() {
            mujrim_study::opening::START_FEN.to_owned()
        } else {
            initial_fen.to_owned()
        },
        moves: moves.to_vec(),
        comments: mujrim_study::explain::comments_for_line(
            if initial_fen.is_empty() {
                mujrim_study::opening::START_FEN
            } else {
                initial_fen
            },
            moves,
        ),
    }
}

pub fn played_game_record(
    event: &str,
    site: &str,
    game: &tournament_live::PlayedGame,
) -> GameRecord {
    GameRecord {
        event: event.to_owned(),
        site: site.to_owned(),
        date: String::new(),
        round: game.round.to_string(),
        white: game.white.clone(),
        black: game.black.clone(),
        result: game_export::result_from_white_score(game.white_score).to_owned(),
        initial_fen: game.initial_fen.clone(),
        moves: game.moves.clone(),
        comments: Vec::new(),
    }
}

pub fn stored_game_record(event: &str, site: &str, game: &StoredTournamentGame) -> GameRecord {
    GameRecord {
        event: event.to_owned(),
        site: site.to_owned(),
        date: String::new(),
        round: game.round.to_string(),
        white: game.white.clone(),
        black: game.black.clone(),
        result: game_export::result_from_white_score(game.white_score).to_owned(),
        initial_fen: game.initial_fen.clone(),
        moves: game.moves.clone(),
        comments: Vec::new(),
    }
}

pub fn tournament_records(
    event: &str,
    site: &str,
    played: &[tournament_live::PlayedGame],
    stored: &[StoredTournamentGame],
) -> Vec<GameRecord> {
    if !played.is_empty() {
        played
            .iter()
            .map(|game| played_game_record(event, site, game))
            .collect()
    } else {
        stored
            .iter()
            .map(|game| stored_game_record(event, site, game))
            .collect()
    }
}

pub fn optimistic_live_board(
    white: impl Into<String>,
    black: impl Into<String>,
    initial_clock_ms: u64,
) -> tournament_live::LiveGameBoard {
    tournament_live::LiveGameBoard {
        game_key: "pending-0".to_owned(),
        match_index: 1,
        round: 1,
        white: white.into(),
        black: black.into(),
        initial_fen: mujrim_study::opening::START_FEN.to_owned(),
        moves: Vec::new(),
        last_uci: String::new(),
        score_cp: 0,
        depth: 0,
        nodes: 0,
        white_clock_ms: Some(initial_clock_ms),
        black_clock_ms: Some(initial_clock_ms),
        clock_synced_ms: Some(tournament_live::now_unix_ms()),
        ..tournament_live::LiveGameBoard::default()
    }
}

pub fn eval_label(score_cp: i32) -> String {
    format!("{:+.2}", score_cp as f32 / 100.0)
}

pub fn eval_tint(score_cp: i32) -> (u8, u8, u8) {
    let t = (score_cp as f32 / 280.0).clamp(-1.0, 1.0);
    if t >= 0.0 {
        let mix = t;
        (
            (160.0 * (1.0 - mix) + 72.0 * mix) as u8,
            (170.0 * (1.0 - mix) + 210.0 * mix) as u8,
            (160.0 * (1.0 - mix) + 92.0 * mix) as u8,
        )
    } else {
        let mix = -t;
        (
            (160.0 * (1.0 - mix) + 230.0 * mix) as u8,
            (170.0 * (1.0 - mix) + 72.0 * mix) as u8,
            (160.0 * (1.0 - mix) + 72.0 * mix) as u8,
        )
    }
}

pub fn annotation_tint(annotation: Option<MoveAnnotation>) -> Option<(u8, u8, u8)> {
    annotation
        .filter(|item| item.shows_board_badge())
        .map(MoveAnnotation::chess_com_rgb)
}

pub fn snapshot_to_stored(
    id: &str,
    name: &str,
    format: TournamentFormat,
    created_at: i64,
    snap: &tournament_live::LiveTournamentSnapshot,
) -> StoredTournament {
    StoredTournament {
        id: id.to_owned(),
        name: name.to_owned(),
        format,
        created_at,
        status: mujrim_study::tournament_store::lifecycle_status(
            snap.paused,
            snap.running,
            snap.cancelled,
            snap.finished,
        )
        .to_owned(),
        entrants: snap
            .engine_names
            .iter()
            .enumerate()
            .map(|(index, name)| Entrant {
                id: format!("engine-{index}"),
                name: name.clone(),
                seed_elo: mujrim_study::rating::seed_elo_for_engine(name),
            })
            .collect(),
        results: snap.game_results.clone(),
        games: snap
            .played_games
            .iter()
            .map(|game| StoredTournamentGame {
                game_index: game.id,
                round: game.round,
                white: game.white.clone(),
                black: game.black.clone(),
                white_score: game.white_score,
                initial_fen: game.initial_fen.clone(),
                moves: game.moves.clone(),
            })
            .collect(),
    }
}

pub fn stored_to_played(games: &[StoredTournamentGame]) -> Vec<tournament_live::PlayedGame> {
    games
        .iter()
        .map(|game| tournament_live::PlayedGame {
            id: game.game_index,
            match_index: game.game_index,
            round: game.round,
            white: game.white.clone(),
            black: game.black.clone(),
            white_score: game.white_score,
            initial_fen: game.initial_fen.clone(),
            moves: game.moves.clone(),
        })
        .collect()
}

pub fn persist_live_tournament(
    database: &mut StudyDatabase,
    id: &str,
    name: &str,
    format: TournamentFormat,
    created_at: i64,
    snap: &tournament_live::LiveTournamentSnapshot,
) -> Result<(), String> {
    let stored = snapshot_to_stored(id, name, format, created_at, snap);
    database.save_tournament(&stored)
}

pub fn export_records_to_path(
    records: &[GameRecord],
    path: &Path,
) -> Result<GameExportFormat, String> {
    if records.is_empty() {
        return Err("No games to export.".to_owned());
    }
    game_export::write_games(records, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::engine::{GameMode, PlayerConfig, QuickTournamentEngine};
    use mujrim_study::annotation::MoveAnnotation;
    use mujrim_study::database::{GameMetadata, GameSummary};

    #[test]
    fn next_incremental_uci_accepts_a_single_new_ply() {
        assert_eq!(
            next_incremental_uci(&["e2e4".into()], &["e2e4".into(), "e7e5".into()]),
            Some("e7e5".into())
        );
        assert_eq!(
            next_incremental_uci(&["e2e4".into()], &["d2d4".into()]),
            None
        );
        assert_eq!(
            next_incremental_uci(&[], &["e2e4".into()]),
            Some("e2e4".into())
        );
    }

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
    fn live_tournament_standings_stay_on_played_games() {
        let src = include_str!("logic.rs");
        let body = src
            .split("fn run_quick_tournament_body")
            .nth(1)
            .expect("tournament body");
        assert!(body.contains("refresh_standings"));
        let match_finished = body
            .split("TournamentEvent::MatchFinished")
            .nth(1)
            .expect("match finished")
            .split("TournamentEvent::Cancelled")
            .next()
            .expect("cancelled follows match");
        assert!(
            match_finished.contains("refresh_standings"),
            "match completion must keep the same field Elo as live games"
        );
        assert!(
            !match_finished.contains("standing_rows"),
            "match completion must not swap in a second Elo scale"
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
        let primary = PathBuf::from("engines")
            .join("mujrim")
            .join("bin")
            .join("windows-aarch64")
            .join("mujrim-elite.exe");
        let alias = PathBuf::from("engines")
            .join("mujrim")
            .join("bin")
            .join("windows-arm64")
            .join("mujrim-elite.exe");
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

    #[test]
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

    #[test]
    fn list_engine_binaries_scans_cwd_layout_and_skips_sidecars() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mujrim-ui-engines-{}-{}",
            std::process::id(),
            stamp
        ));
        let target = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        let bin = root
            .join("engines")
            .join("stockfish")
            .join("bin")
            .join(&target);
        std::fs::create_dir_all(&bin).unwrap();
        let engine = bin.join("stockfish");
        let weights = bin.join("weights.pb.gz");
        std::fs::write(&engine, b"engine").unwrap();
        std::fs::write(&weights, b"net").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&engine).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&engine, perms.clone()).unwrap();
            std::fs::set_permissions(&weights, perms).unwrap();
        }
        let roots = vec![root.join("engines")];
        let found = list_engine_binaries_in_roots(&roots);
        assert_eq!(found, vec![engine.clone()]);
        assert_eq!(engine_identity_key(&engine), "stockfish");
        assert!(path_is_under_engine_roots(&engine, &roots));
        assert!(!path_is_under_engine_roots(
            Path::new("/tmp/not-an-engine"),
            &roots
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn engine_identity_dedupes_aliases_and_skips_product_bins() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mujrim-ui-engine-identity-{}-{}",
            std::process::id(),
            stamp
        ));
        let target = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        let write_bin = |folder: &str, name: &str| {
            let bin = root.join(folder).join(name).join("bin").join(&target);
            std::fs::create_dir_all(&bin).unwrap();
            let path = bin.join(name);
            std::fs::write(&path, b"engine").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&path).unwrap().permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&path, perms).unwrap();
            }
            path
        };
        let cwd_stockfish = write_bin("engines-a", "stockfish");
        write_bin("engines-b", "stockfish");
        write_bin("engines-a", "mujrim");
        write_bin("engines-b", "mujrim-elite");
        write_bin("engines-a", "mujrim-ui");
        let roots = vec![root.join("engines-a"), root.join("engines-b")];
        let found = list_engine_binaries_in_roots(&roots);
        let ids: Vec<_> = found.iter().map(|path| engine_identity_key(path)).collect();
        assert_eq!(ids.iter().filter(|id| *id == "stockfish").count(), 1);
        assert_eq!(ids.iter().filter(|id| *id == "mujrim-elite").count(), 1);
        assert!(!ids.iter().any(|id| id == "mujrim-ui"));
        assert_eq!(found.len(), 2);
        let other_stockfish = root
            .join("engines-b")
            .join("stockfish")
            .join("bin")
            .join(&target)
            .join("stockfish");
        assert!(engine_is_selected(
            std::slice::from_ref(&cwd_stockfish),
            &other_stockfish
        ));
        let mut selected = vec![cwd_stockfish.clone()];
        toggle_engine_selection(&mut selected, cwd_stockfish.clone());
        assert!(selected.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_uci_move_and_opening_score_are_legal() {
        types::init();
        let mut board = types::Board::new();
        let played = apply_uci_move(&mut board, "e2e4").expect("e4");
        assert_eq!(played.to_uci(), "e2e4");
        assert!(apply_uci_move(&mut board, "e2e4").is_err());
        let stats = MoveStatistics {
            games: 10,
            white_wins: 4,
            draws: 4,
            black_wins: 2,
        };
        assert_eq!(opening_white_score(&stats), 60);
    }

    #[test]
    fn detected_engines_drive_home_and_tournament_player_assignment() {
        let roster = vec![
            QuickTournamentEngine {
                name: "Stockfish".into(),
                path: PathBuf::from("/tmp/engines/stockfish"),
                search_limits: mujrim_protocols::catalog::SearchLimitSupport::STANDARD,
            },
            QuickTournamentEngine {
                name: "Lc0".into(),
                path: PathBuf::from("/tmp/engines/lc0"),
                search_limits: mujrim_protocols::catalog::SearchLimitSupport::STANDARD,
            },
        ];
        assert!(matches!(
            engine_player_from_roster(&roster, 0, 16),
            PlayerConfig::External { ref path, .. } if path.ends_with("stockfish")
        ));
        let (white, black) = (
            engine_player_from_roster(&roster, 0, 16),
            engine_player_from_roster(&roster, 1, 12),
        );
        assert!(matches!(white, PlayerConfig::External { .. }));
        assert!(matches!(black, PlayerConfig::External { .. }));
        let empty = players_for_detected_engines(GameMode::HumanVsHuman, &[], &[]);
        assert!(matches!(empty.0, PlayerConfig::Human));
        assert!(matches!(empty.1, PlayerConfig::Human));
    }

    #[test]
    fn default_tournament_selection_is_two_engines() {
        let roster: Vec<QuickTournamentEngine> = (0..5)
            .map(|i| QuickTournamentEngine {
                name: format!("e{i}"),
                path: PathBuf::from(format!("/tmp/e{i}")),
                search_limits: mujrim_protocols::catalog::SearchLimitSupport::STANDARD,
            })
            .collect();
        let selected = default_tournament_engine_paths(&roster);
        assert_eq!(
            selected.len(),
            crate::app_core::tournament_setup::GUI_TOURNAMENT_DEFAULT_ENGINES
        );
        assert_eq!(selected[0], PathBuf::from("/tmp/e0"));
        assert_eq!(selected[1], PathBuf::from("/tmp/e1"));
        assert!(default_tournament_engine_paths(&[]).is_empty());
    }

    #[test]
    fn lc0_gui_args_select_a_backend_but_mujrim_wrappers_stay_clean() {
        let args = gui_safe_engine_args(Path::new("/engines/lc0"));
        assert!(
            args.windows(2).any(|pair| pair == ["--backend", "eigen"])
                || args.iter().any(|arg| arg.contains("backend"))
        );
        assert!(gui_safe_engine_args(Path::new("/engines/stockfish")).is_empty());
        assert!(gui_safe_engine_args(Path::new("/engines/mujrim")).is_empty());
        assert!(gui_safe_engine_args(Path::new("/engines/mujrim-lc0")).is_empty());
        assert!(gui_safe_engine_args(Path::new("/engines/mujrim-v60")).is_empty());
        let (path, args) = resolved_engine_launch(Path::new("/engines/mujrim-akimbo"));
        assert_eq!(path, PathBuf::from("/engines/mujrim-akimbo"));
        assert!(args.is_empty());
    }

    #[test]
    fn uci_to_san_covers_pawn_knight_and_castle() {
        types::init();
        let board = types::Board::new();
        assert_eq!(uci_to_san(&board, "e2e4"), "e4");
        assert_eq!(uci_to_san(&board, "g1f3"), "Nf3");
        let labels = san_annotated_moves(
            mujrim_study::opening::START_FEN,
            &["e2e4".into(), "e7e5".into(), "g1f3".into()],
            &[
                None,
                None,
                Some(mujrim_study::annotation::MoveAnnotation::Book),
            ],
        );
        assert_eq!(labels, ["e4", "e5", "Nf3 B"]);
        assert_eq!(
            ply_chip_label(
                "Nf3",
                Some(mujrim_study::annotation::MoveAnnotation::Book),
                Some(32)
            ),
            "Nf3 B  +0.32"
        );
        assert_eq!(
            move_list_chip_labels(
                mujrim_study::opening::START_FEN,
                &["e2e4".into(), "e7e5".into(), "g1f3".into()],
                &[
                    Some(mujrim_study::annotation::MoveAnnotation::Book),
                    None,
                    Some(mujrim_study::annotation::MoveAnnotation::Brilliant),
                ],
                &[Some(20), None, Some(45)],
            ),
            ["e4 B  +0.20", "e5", "Nf3 !!  +0.45"]
        );
        let mut after_e4 = board.clone();
        let mv = find_logged_move(&mut after_e4, "e2e4").unwrap();
        after_e4.make_move(mv);
        assert_eq!(uci_to_san(&after_e4, "e7e5"), "e5");
    }

    #[test]
    fn displayed_study_fen_follows_review_ply() {
        types::init();
        let moves = vec!["e2e4".to_owned(), "e7e5".to_owned()];
        let start = displayed_study_fen(mujrim_study::opening::START_FEN, &moves, Some(0), None);
        assert!(start.contains(" w "));
        let after_one =
            displayed_study_fen(mujrim_study::opening::START_FEN, &moves, Some(1), None);
        assert!(after_one.contains(" b "));
    }

    #[test]
    fn current_game_record_attaches_explainer_comments() {
        types::init();
        let record = current_game_record(
            "W",
            "B",
            "",
            "",
            "rnbqk2r/pppp1ppp/5n2/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
            &["e1g1".to_owned()],
            "*",
        );
        assert!(
            record
                .comments
                .iter()
                .any(|(_, text)| text.to_lowercase().contains("castl")),
            "{:?}",
            record.comments
        );
    }

    #[test]
    fn save_current_line_rejects_illegal_prep() {
        assert!(
            save_current_line(
                "Bad".into(),
                mujrim_study::opening::PrepSide::White,
                mujrim_study::opening::START_FEN.to_owned(),
                vec!["e2e5".into()],
                String::new(),
            )
            .is_err()
        );
        let ok = save_current_line(
            "Open".into(),
            mujrim_study::opening::PrepSide::White,
            mujrim_study::opening::START_FEN.to_owned(),
            vec!["e2e4".into()],
            "notes".into(),
        )
        .unwrap();
        assert_eq!(ok.moves, vec!["e2e4".to_owned()]);
    }

    #[test]
    fn tournament_records_prefer_played_games_and_export_binpack() {
        let played = vec![tournament_live::PlayedGame {
            id: 0,
            match_index: 0,
            round: 1,
            white: "Alpha".into(),
            black: "Beta".into(),
            white_score: 1.0,
            initial_fen: mujrim_study::opening::START_FEN.into(),
            moves: vec!["e2e4".into(), "e7e5".into()],
        }];
        let records = tournament_records("Cup", "Local", &played, &[]);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].result, "1-0");
        let bytes =
            mujrim_study::game_export::encode_games(&records, GameExportFormat::Binpack).unwrap();
        let positions = mujrim_study::game_export::decode_binpack(&bytes).unwrap();
        assert_eq!(positions.len(), 2);
        assert_eq!(positions[0].mv, "e2e4");
    }

    #[test]
    fn optimistic_board_and_eval_tints_are_stable() {
        let board = optimistic_live_board("A", "B", 180_000);
        assert_eq!(board.white, "A");
        assert_eq!(board.black, "B");
        assert!(board.moves.is_empty());
        assert_eq!(eval_label(32), "+0.32");
        let (r, g, _b) = eval_tint(300);
        assert!(g > r);
        let (r, g, _b) = eval_tint(-300);
        assert!(r > g);
        assert_eq!(
            annotation_tint(Some(MoveAnnotation::Blunder)),
            Some((224, 40, 40))
        );
    }

    #[test]
    fn eval_bar_engine_defaults_to_v60_and_resolves_by_identity() {
        use crate::app_core::settings::EVAL_BAR_DEFAULT_ENGINE;
        let choices = eval_bar_engine_choices(&[], &[]);
        assert_eq!(choices[0].id, EVAL_BAR_DEFAULT_ENGINE);
        assert_eq!(choices[0].label, "Mujrim v60");
        assert!(resolve_eval_bar_engine_path(EVAL_BAR_DEFAULT_ENGINE, &[], &[]).is_none());
    }
}
