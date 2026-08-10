use std::collections::HashMap;
use std::collections::VecDeque;
use std::io::BufRead;
use std::sync::Arc;

use crate::{
    board::{Board, NullBoardObserver},
    search::Report,
    thread::{SharedContext, Status, ThreadData},
    threadpool::ThreadPool,
    time::{Limits, TimeManager},
    tools,
    transposition::DEFAULT_TT_SIZE,
    types::{Color, MAX_MOVES, Move, Piece, Score, Square, is_decisive, is_loss, is_win},
};

#[derive(Copy, Clone, PartialEq, Eq)]
enum Mode {
    Cli,
    Uci,
}

struct Settings {
    frc: bool,
    multi_pv: usize,
    move_overhead: u64,
    ponder: bool,
    report: Report,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            frc: false,
            multi_pv: 1,
            move_overhead: 100,
            ponder: true,
            report: Report::Full,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn message_loop(mut buffer: VecDeque<String>) {
    let shared = Arc::new(SharedContext::default());
    let mut settings = Settings::default();
    let mut threads = ThreadPool::new(shared.clone());
    let mut board = Board::starting_position();

    #[cfg(feature = "syzygy")]
    let auto_tablebases = crate::tb::initialize_discovered();

    let rx = spawn_listener(shared.clone());

    let mut mode = if buffer.is_empty() { Mode::Uci } else { Mode::Cli };

    loop {
        let message = if let Some(cmd) = buffer.pop_front() {
            cmd
        } else if mode == Mode::Uci {
            match rx.recv() {
                Ok(cmd) => cmd,
                Err(_) => break,
            }
        } else {
            break;
        };

        let message = normalize_command(&message);
        let tokens = message.split_whitespace().collect::<Vec<_>>();
        match tokens.as_slice() {
            ["uci"] => {
                uci(
                    #[cfg(feature = "syzygy")]
                    auto_tablebases.as_ref(),
                );
                mode = Mode::Uci;
            }

            ["isready"] => println!("readyok"),

            ["go", tokens @ ..] => go(&mut threads, &settings, &board, &shared, tokens),
            ["position", tokens @ ..] => {
                if let Err(error) = position(&mut board, &settings, tokens) {
                    println!("info string position rejected: {error}");
                }
            }
            ["setoption", tokens @ ..] => set_option(&mut threads, &mut settings, &shared, tokens),
            ["ucinewgame"] => reset(&mut threads, &shared),

            ["stop"] => shared.status.set(Status::STOPPED),
            ["quit"] => {
                drop(threads);
                break;
            }

            // Non-UCI commands
            ["compiler"] => compiler(),
            ["eval"] => eval(threads.main_thread(), &board),
            ["d"] => println!("{board}"),
            ["bench", args @ ..] => match mode {
                Mode::Uci => tools::bench::<true>(args),
                Mode::Cli => tools::bench::<false>(args),
            },
            ["speedtest", args @ ..] => tools::speedtest(args),
            ["perft", depth] => match depth.parse() {
                Ok(depth) => tools::perft(depth, &mut board),
                Err(_) => eprintln!("Invalid perft depth: '{depth}'"),
            },
            ["perft"] => eprintln!("Usage: perft <depth>"),
            ["simpleperft", depth] => match depth.parse() {
                Ok(depth) => tools::simple_perft(depth, &mut board),
                Err(_) => eprintln!("Invalid simpleperft depth: '{depth}'"),
            },
            ["simpleperft"] => eprintln!("Usage: simpleperft <depth>"),
            ["islegalperft", depth] => match depth.parse() {
                Ok(depth) => tools::is_legal_perft(depth, &mut board),
                Err(_) => eprintln!("Invalid islegalperft depth: '{depth}'"),
            },
            ["islegalperft"] => eprintln!("Usage: islegalperft <depth>"),

            // Ignore empty lines
            [] => (),

            _ => eprintln!("Unknown command: '{}'", message.trim_end()),
        }

        // Auto-exit after last CLI command
        if matches!(mode, Mode::Cli) && buffer.is_empty() {
            drop(threads);
            break;
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_listener(shared: Arc<SharedContext>) -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut stdin = stdin.lock();
        loop {
            let message = match read_command(&mut stdin) {
                Ok(Some(message)) => message,
                Ok(None) => {
                    shared.status.set(Status::STOPPED);
                    let _ = tx.send("quit".to_string());
                    break;
                }
                Err(error) => {
                    eprintln!("UCI input error: {error}");
                    shared.status.set(Status::STOPPED);
                    let _ = tx.send("quit".to_string());
                    break;
                }
            };

            match message.as_str() {
                "isready" => println!("readyok"),
                "stop" => shared.status.set(Status::STOPPED),
                "ponderhit" => {
                    println!("info string ponderhit accepted");
                    shared.status.set(Status::STOPPED);
                }
                "quit" => {
                    shared.status.set(Status::STOPPED);
                    let _ = tx.send("quit".to_string());
                    break;
                }
                _ => {
                    // According to the UCI specs, commands that are unexpected
                    // in the current state should be ignored silently.
                    // (https://backscattering.de/chess/uci/#unexpected)
                    if shared.status.get() != Status::RUNNING {
                        let _ = tx.send(message);
                    }
                }
            }
        }
    });

    rx
}

fn normalize_command(command: &str) -> &str {
    command.trim().trim_start_matches('\u{feff}')
}

fn read_command(reader: &mut impl BufRead) -> std::io::Result<Option<String>> {
    let mut command = String::new();
    match reader.read_line(&mut command)? {
        0 => Ok(None),
        _ => Ok(Some(normalize_command(&command).to_string())),
    }
}

fn uci(#[cfg(feature = "syzygy")] auto_tablebases: Option<&crate::tb::Installation>) {
    println!(
        "id name {} {}",
        <crate::V60SearchAdapter as crate::NativeSearchAdapter>::ENGINE_NAME,
        env!("ENGINE_VERSION")
    );
    println!("id author {}", <crate::V60SearchAdapter as crate::NativeSearchAdapter>::ENGINE_AUTHOR);
    println!("option name Hash type spin default {DEFAULT_TT_SIZE} min 1 max 1024");
    println!("option name Threads type spin default 1 min 1 max {}", ThreadPool::configured_thread_limit());
    println!("option name MoveOverhead type spin default 100 min 0 max 2000");
    println!("option name Ponder type check default true");
    println!("option name Minimal type check default false");
    println!("option name Clear Hash type button");
    println!("option name UCI_Chess960 type check default false");
    println!("option name MultiPV type spin default 1 min 1 max {MAX_MOVES}");

    #[cfg(feature = "syzygy")]
    println!("option name SyzygyPath type string default");

    #[cfg(feature = "syzygy")]
    if let Some(installation) = auto_tablebases {
        println!(
            "info string Syzygy auto-loaded {}-piece tables from {}",
            installation.pieces,
            installation.path.display()
        );
    }

    #[cfg(feature = "spsa")]
    crate::parameters::print_options();

    println!("uciok");
}

fn compiler() {
    println!("Compiler Version: {}", env!("COMPILER_VERSION"));
    println!("Compiler Target: {}", env!("COMPILER_TARGET"));
    println!("Compiler Features: {}", env!("COMPILER_FEATURES"));
}

fn reset(threads: &mut ThreadPool, shared: &Arc<SharedContext>) {
    threads.clear();
    shared.tt.clear(threads.len());

    for corrhist in shared.history.all() {
        corrhist.pawn.clear();
        corrhist.non_pawn[Color::White].clear();
        corrhist.non_pawn[Color::Black].clear();
    }
}

fn go(threads: &mut ThreadPool, settings: &Settings, board: &Board, shared: &Arc<SharedContext>, tokens: &[&str]) {
    let pondering = tokens.contains(&"ponder");
    let limits = if pondering { Limits::Infinite } else { parse_limits(board.side_to_move(), tokens) };
    let restricted_root_moves = match parse_searchmoves(board, tokens) {
        Ok(moves) => moves,
        Err(error) => {
            println!("info string go rejected: {error}");
            println!("bestmove 0000");
            return;
        }
    };
    let time_manager = TimeManager::new(limits, board.fullmove_number(), settings.move_overhead);

    threads.execute_searches(
        time_manager,
        settings.report,
        settings.multi_pv,
        restricted_root_moves.as_deref(),
        board,
        shared,
    );

    if threads[0].root_moves.is_empty() {
        println!("bestmove 0000");
        return;
    }

    let min_score = threads.iter().map(|v| v.root_moves[0].score).min().unwrap();
    let vote_value = |td: &ThreadData| (td.root_moves[0].score - min_score + 10) * td.completed_depth;

    let mut votes: HashMap<&Move, i32> = HashMap::new();
    for result in threads.iter() {
        *votes.entry(&result.root_moves[0].mv).or_default() += vote_value(result);
    }

    let mut best = 0;

    if !matches!(threads[best].time_manager.limits(), Limits::Depth(_)) && threads[0].multi_pv == 1 {
        for current in 1..threads.len() {
            let is_better_candidate = || -> bool {
                let best = &threads[best];
                let current = &threads[current];

                if is_win(best.root_moves[0].score) {
                    return current.root_moves[0].score > best.root_moves[0].score;
                }

                if current.root_moves[0].score != -Score::INFINITE
                    && best.root_moves[0].score != -Score::INFINITE
                    && is_loss(best.root_moves[0].score)
                {
                    return current.root_moves[0].score < best.root_moves[0].score;
                }

                if current.root_moves[0].score != -Score::INFINITE && is_decisive(current.root_moves[0].score) {
                    return true;
                }

                let best_vote = votes[&best.root_moves[0].mv];
                let current_vote = votes[&current.root_moves[0].mv];

                !is_loss(current.root_moves[0].score)
                    && (current_vote > best_vote
                        || (current_vote == best_vote && vote_value(current) > vote_value(best)))
            };

            if is_better_candidate() {
                best = current;
            }
        }
    }

    if best != 0 {
        let depth = threads[best].completed_depth;
        threads[best].print_uci_info(depth);
    }

    let best_move = threads[best].root_moves[0].mv;
    let ponder_move = settings.ponder.then(|| threads[best].root_moves[0].pv.line().first().copied()).flatten();
    println!("{}", format_bestmove(board, best_move, ponder_move));
    crate::misc::dbg_print();
}

fn format_bestmove(board: &Board, best_move: Move, ponder_move: Option<Move>) -> String {
    let legal = board.generate_all_moves();
    let Some(best_move) = legal
        .iter()
        .map(|entry| entry.mv)
        .find(|mv| *mv == best_move)
        .or_else(|| legal.iter().next().map(|entry| entry.mv))
    else {
        return "bestmove 0000".to_string();
    };

    let best = best_move.to_uci(board);
    let mut after_best = board.clone();
    after_best.make_move(best_move, &mut NullBoardObserver);
    let legal_ponder =
        ponder_move.filter(|ponder| after_best.generate_all_moves().iter().any(|entry| entry.mv == *ponder));
    legal_ponder.map_or_else(
        || format!("bestmove {best}"),
        |ponder| format!("bestmove {best} ponder {}", ponder.to_uci(&after_best)),
    )
}

fn position(board: &mut Board, settings: &Settings, tokens: &[&str]) -> Result<(), String> {
    let (mut candidate, remaining) = match tokens {
        [] => (Board::starting_position(), &[][..]),
        ["startpos", rest @ ..] => (Board::starting_position(), rest),
        ["moves", ..] => (Board::starting_position(), tokens),
        ["fen", rest @ ..] if rest.len() >= 6 => {
            let fen = rest[..6].join(" ");
            let parsed = Board::from_fen(&fen).map_err(|error| format!("invalid FEN: {error:?}"))?;
            (parsed, &rest[6..])
        }
        ["fen", ..] => return Err("FEN requires six fields".to_string()),
        _ => return Err("expected 'startpos', 'fen', or 'moves'".to_string()),
    };
    candidate.set_frc(settings.frc);

    let moves = match remaining {
        [] => &[][..],
        ["moves", moves @ ..] => moves,
        _ => {
            return Err(format!("unexpected position tokens: {}", remaining.join(" ")));
        }
    };
    for &uci_move in moves {
        make_uci_move(&mut candidate, uci_move)?;
    }
    *board = candidate;
    Ok(())
}

fn make_uci_move(board: &mut Board, uci_move: &str) -> Result<(), String> {
    let moves = board.generate_all_moves();
    if let Some(mv) = moves.iter().map(|entry| entry.mv).find(|mv| mv.to_uci(board) == uci_move) {
        board.make_move(mv, &mut NullBoardObserver);
        Ok(())
    } else {
        Err(format!("illegal move '{uci_move}'"))
    }
}

fn set_option(threads: &mut ThreadPool, settings: &mut Settings, shared: &Arc<SharedContext>, tokens: &[&str]) {
    match tokens {
        ["name", "Minimal", "value", v] => match *v {
            "true" => settings.report = Report::Minimal,
            "false" => settings.report = Report::Full,
            _ => eprintln!("Invalid value: '{v}'"),
        },
        ["name", "Clear", "Hash"] => {
            shared.tt.clear(threads.len());
            println!("info string Hash cleared");
        }
        ["name", "Hash", "value", v] => match v.parse::<usize>() {
            Ok(value) => {
                let value = value.clamp(1, 1024);
                shared.tt.resize(threads.len(), value);
                println!("info string set Hash to {value} MB");
            }
            Err(_) => eprintln!("Invalid Hash value: '{v}'"),
        },
        ["name", "Threads", "value", v] => {
            threads.set_count(v.parse().unwrap_or(1));
            println!("info string set Threads to {}", threads.len());
        }
        ["name", "MoveOverhead", "value", v] => match v.parse::<u64>() {
            Ok(value) => {
                settings.move_overhead = value.min(2000);
                println!("info string set MoveOverhead to {} ms", settings.move_overhead);
            }
            Err(_) => eprintln!("Invalid MoveOverhead value: '{v}'"),
        },
        ["name", "Ponder", "value", v] => match *v {
            "true" => settings.ponder = true,
            "false" => settings.ponder = false,
            _ => eprintln!("Invalid Ponder value: '{v}'"),
        },
        #[cfg(feature = "syzygy")]
        ["name", "SyzygyPath", "value", v] => match crate::tb::initialize(v) {
            Some(size) => println!("info string Loaded Syzygy tablebases with {size} pieces"),
            None => eprintln!("Failed to load Syzygy tablebases"),
        },
        ["name", "UCI_Chess960", "value", v] => {
            settings.frc = v.parse().unwrap_or_default();
            println!("info string set UCI_Chess960 to {v}");
        }
        ["name", "MultiPV", "value", v] => {
            settings.multi_pv = v.parse().unwrap_or_default();
            println!("info string set MultiPV to {v}");
        }
        #[cfg(feature = "spsa")]
        ["name", name, "value", v] => {
            crate::parameters::set_parameter(name, v);
            println!("info string set {name} to {v}");
        }
        _ => eprintln!("Unknown option: '{}'", tokens.join(" ").trim_end()),
    }
}

fn eval(td: &mut ThreadData, board: &Board) {
    td.nnue.full_refresh(board);
    td.nnue.evaluate(board);

    let side = board.side_to_move();

    println!("NNUE derived piece values:");
    println!("+-------+-------+-------+-------+-------+-------+-------+-------+");
    for rank in (0..8).rev() {
        print!("|");
        for file in 0..8 {
            let sq = Square::from_rank_file(rank, file);
            let piece = board.piece_on(sq);
            let piece_str = if piece == Piece::None { " ".to_string() } else { piece.to_string() };
            print!("  {piece_str:^3}  |");
        }
        println!();

        print!("|");
        for file in 0..8 {
            let sq = Square::from_rank_file(rank, file);
            match td.nnue.piece_contribution(board, sq) {
                None => print!("       |"),
                Some(v) => {
                    let white_relative = if side == Color::White { v } else { -v };
                    let val = white_relative as f32 / 100.0;
                    print!("{val:+6.2} |");
                }
            }
        }
        println!();
        println!("+-------+-------+-------+-------+-------+-------+-------+-------+");
    }

    let used_bucket = crate::nnue::OUTPUT_BUCKETS_LAYOUT[board.occupancies().popcount()];

    println!("\nNNUE output buckets (White's POV):");
    println!("+-------------+------------+");
    println!("|   Buckets   |   Total    |");
    println!("+-------------+------------+");

    for bucket in 0..8 {
        let raw_score = td.nnue.eval_with_bucket(board, bucket);
        let white_score = if side == Color::White { raw_score } else { -raw_score };
        let total = white_score as f32 / 100.0;

        if bucket == used_bucket {
            println!("|  >   {bucket:<7}| {total:+7.2}    |");
        } else {
            println!("|{bucket:^13}| {total:+7.2}    |");
        }
    }
    println!("+-------------+------------+");

    let final_eval = td.nnue.evaluate(board);
    let final_total = (if side == Color::White { final_eval } else { -final_eval }) as f32 / 100.0;
    println!("\nNNUE evaluation        {final_total:+.2} (White's POV)");
}

fn parse_limits(color: Color, tokens: &[&str]) -> Limits {
    if tokens.contains(&"infinite") {
        return Limits::Infinite;
    }

    let mut main = None;
    let mut inc = None;
    let mut moves = None;

    let mut index = 0;
    while index + 1 < tokens.len() {
        let name = tokens[index];
        let Ok(value) = tokens[index + 1].parse::<u64>() else {
            index += 1;
            continue;
        };

        match name {
            "depth" if value > 0 => return Limits::Depth(value as i32),
            "movetime" if value > 0 => return Limits::Time(value),
            "nodes" if value > 0 => return Limits::Nodes(value),
            "mate" if value > 0 => return Limits::Mate(value),

            "wtime" if Color::White == color => main = Some(value),
            "btime" if Color::Black == color => main = Some(value),
            "winc" if Color::White == color => inc = Some(value),
            "binc" if Color::Black == color => inc = Some(value),
            "movestogo" => moves = Some(value),

            _ => {}
        }
        index += 2;
    }

    if main.is_none() && inc.is_none() {
        return Limits::Infinite;
    }

    let main = main.unwrap_or_default();
    let inc = inc.unwrap_or_default();

    match moves {
        Some(moves) => Limits::Cyclic(main, inc, moves),
        None => Limits::Fischer(main, inc),
    }
}

fn parse_searchmoves(board: &Board, tokens: &[&str]) -> Result<Option<Vec<Move>>, String> {
    const GO_KEYWORDS: [&str; 11] =
        ["ponder", "wtime", "btime", "winc", "binc", "movestogo", "depth", "nodes", "mate", "movetime", "infinite"];

    let Some(start) = tokens.iter().position(|token| *token == "searchmoves") else {
        return Ok(None);
    };
    let requested =
        tokens[start + 1..].iter().copied().take_while(|token| !GO_KEYWORDS.contains(token)).collect::<Vec<_>>();
    if requested.is_empty() {
        return Err("searchmoves requires at least one legal move".to_string());
    }

    let legal = board.generate_all_moves();
    let mut selected = Vec::with_capacity(requested.len());
    for requested_move in requested {
        let Some(mv) = legal.iter().map(|entry| entry.mv).find(|mv| mv.to_uci(board) == requested_move) else {
            return Err(format!("illegal searchmove '{requested_move}'"));
        };
        if !selected.contains(&mv) {
            selected.push(mv);
        }
    }
    Ok(Some(selected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn command_reader_normalizes_bom_and_detects_eof() {
        let mut input = Cursor::new("\u{feff}uci\r\n");
        assert_eq!(read_command(&mut input).unwrap().as_deref(), Some("uci"));
        assert_eq!(read_command(&mut input).unwrap(), None);
    }

    fn test_position_helper(tokens: &[&str]) -> Board {
        let settings = Settings::default();
        let mut board = Board::starting_position();

        position(&mut board, &settings, tokens).expect("test position must be accepted");
        board.clone()
    }

    #[test]
    fn test_position_startpos() {
        let board = test_position_helper(&["startpos"]);
        assert_eq!(board.to_fen(), "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        let board = test_position_helper(&[]);
        assert_eq!(board.to_fen(), "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    }

    #[test]
    fn test_position_startpos_multiple_moves() {
        let board = test_position_helper(&["moves", "e2e4", "e7e5", "g1f3"]);
        assert_eq!(board.side_to_move(), Color::Black);
        let fen = board.to_fen();
        let fen_position = fen.split_whitespace().next().unwrap();
        assert!(fen_position.contains("5N2"));
    }

    #[test]
    fn test_position_fen_with_moves() {
        let board = test_position_helper(&[
            "fen",
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR",
            "b",
            "KQkq",
            "e3",
            "0",
            "1",
            "moves",
            "e7e5",
        ]);
        assert_eq!(board.side_to_move(), Color::White);
    }

    #[test]
    fn test_position_empty_moves_list() {
        let board = test_position_helper(&["moves"]);
        assert_eq!(board.to_fen(), "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    }

    #[test]
    fn invalid_position_move_rejects_the_entire_update() {
        let settings = Settings::default();
        let mut board = Board::starting_position();
        let original = board.to_fen();

        let error = position(&mut board, &settings, &["moves", "e2e4", "invalid", "e7e5"])
            .expect_err("invalid move must reject the position command");

        assert!(error.contains("illegal move"));
        assert_eq!(board.to_fen(), original);
    }

    #[test]
    fn test_position_long_move_sequence() {
        let board = test_position_helper(&["moves", "e2e4", "e7e5", "g1f3", "b8c6", "f1b5", "a7a6"]);
        assert_eq!(board.side_to_move(), Color::White);
    }

    #[test]
    fn test_position_castling() {
        let board = test_position_helper(&[
            "fen",
            "r3k2r/pppppppp/8/8/8/8/PPPPPPPP/R3K2R",
            "w",
            "KQkq",
            "-",
            "0",
            "1",
            "moves",
            "e1g1",
        ]);
        assert_eq!(board.side_to_move(), Color::Black);
    }

    #[test]
    fn test_position_en_passant() {
        let board = test_position_helper(&[
            "fen",
            "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPP1PPP/RNBQKBNR",
            "w",
            "KQkq",
            "f6",
            "0",
            "1",
            "moves",
            "e5f6",
        ]);
        assert_eq!(board.side_to_move(), Color::Black);
    }

    #[test]
    fn test_position_promotion() {
        let board = test_position_helper(&["fen", "8/P7/8/8/8/8/8/4K2k", "w", "-", "-", "0", "1", "moves", "a7a8q"]);
        assert_eq!(board.side_to_move(), Color::Black);
    }

    #[test]
    fn test_make_uci_move_invalid() {
        let mut board = Board::starting_position();
        let fen_before = board.to_fen();
        assert!(make_uci_move(&mut board, "invalid_move").is_err());
        assert_eq!(board.to_fen(), fen_before);
    }

    #[test]
    fn test_position_moves_without_startpos_ignored() {
        let board = test_position_helper(&["moves", "e2e4", "e7e5"]);
        assert_eq!(board.to_fen(), "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2");
    }

    #[test]
    fn go_limits_remain_aligned_after_flag_tokens() {
        let limits = parse_limits(Color::White, &["ponder", "wtime", "5000", "btime", "4000", "winc", "25"]);
        assert!(matches!(limits, Limits::Fischer(5000, 25)));
    }

    #[test]
    fn searchmoves_accepts_only_legal_root_moves_and_stops_at_limits() {
        let board = Board::starting_position();
        let moves = parse_searchmoves(&board, &["searchmoves", "e2e4", "d2d4", "nodes", "1000"])
            .expect("valid searchmoves")
            .expect("restriction exists");

        assert_eq!(moves.len(), 2);
        assert_eq!(moves[0].to_uci(&board), "e2e4");
        assert_eq!(moves[1].to_uci(&board), "d2d4");
        assert!(parse_searchmoves(&board, &["searchmoves", "e2e5"]).is_err());
    }

    #[test]
    fn bestmove_formats_a_legal_predicted_reply() {
        let board = Board::starting_position();
        let legal = board.generate_all_moves();
        let best = legal.iter().map(|entry| entry.mv).find(|mv| mv.to_uci(&board) == "e2e4").expect("e2e4 is legal");
        let mut after_best = board.clone();
        after_best.make_move(best, &mut NullBoardObserver);
        let ponder = after_best
            .generate_all_moves()
            .iter()
            .map(|entry| entry.mv)
            .find(|mv| mv.to_uci(&after_best) == "e7e5")
            .expect("e7e5 is legal");

        assert_eq!(format_bestmove(&board, best, Some(ponder)), "bestmove e2e4 ponder e7e5");
        assert_eq!(format_bestmove(&board, best, None), "bestmove e2e4");
    }

    #[test]
    fn bestmove_replaces_illegal_search_output_and_drops_illegal_ponder() {
        let board = Board::starting_position();
        let expected = board.generate_all_moves().iter().next().unwrap().mv.to_uci(&board);

        assert_eq!(format_bestmove(&board, Move::NULL, Some(Move::NULL)), format!("bestmove {expected}"));
    }
}
