use std::{
    ffi, mem,
    path::{Path, PathBuf},
    ptr,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{
    bindings::{
        TB_DRAW, TB_LARGEST, TB_LOSS, TB_MAX_MOVES, TB_WIN, TbRootMove, TbRootMoves, tb_init, tb_probe_root_dtz,
        tb_probe_root_wdl, tb_probe_wdl,
    },
    board::{Board, NullBoardObserver},
    thread::{RootMove, ThreadData},
    types::{Color, MAX_PLY, PieceType, Score},
};

#[derive(Eq, PartialEq)]
pub enum GameOutcome {
    Win,
    Loss,
    Draw,
}

static SIZE: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug)]
pub struct Installation {
    pub path: PathBuf,
    pub pieces: usize,
}

pub fn initialize(path: &str) -> Option<usize> {
    let cpath = ffi::CString::new(path).ok()?;

    let initialized = unsafe {
        tb_init(cpath.as_ptr());
        TB_LARGEST as usize
    };
    SIZE.store(initialized, Ordering::Release);

    match size() {
        0 => None,
        _ => Some(size()),
    }
}

pub fn size() -> usize {
    SIZE.load(Ordering::Acquire)
}

pub fn initialize_discovered() -> Option<Installation> {
    if let Some(explicit) = std::env::var_os("MUJRIM_SYZYGY") {
        let path = PathBuf::from(explicit);
        let display = path.to_string_lossy();
        return initialize(&display).map(|pieces| Installation { path, pieces });
    }

    for path in tablebase_directories() {
        let display = path.to_string_lossy();
        if let Some(pieces) = initialize(&display) {
            return Some(Installation { path, pieces });
        }
    }
    None
}

fn tablebase_directories() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        for ancestor in executable.parent().into_iter().flat_map(|path| path.ancestors()).take(7) {
            candidates.push(ancestor.join("syzygy"));
        }
    }
    if let Ok(current) = std::env::current_dir() {
        candidates.push(current.join("syzygy"));
        candidates.push(current.join("dist").join("syzygy"));
    }
    candidates.sort();
    candidates.dedup();
    candidates.into_iter().filter(|directory| contains_wdl_table(directory)).collect()
}

fn contains_wdl_table(directory: &Path) -> bool {
    std::fs::read_dir(directory).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry.path().is_file()
                && entry.path().extension().is_some_and(|extension| extension.eq_ignore_ascii_case("rtbw"))
        })
    })
}

pub fn probe(board: &Board) -> Option<GameOutcome> {
    let ep_square = tb_en_passant_square(board);

    let code = unsafe {
        tb_probe_wdl(
            board.colors(Color::White).0,
            board.colors(Color::Black).0,
            board.pieces(PieceType::King).0,
            board.pieces(PieceType::Queen).0,
            board.pieces(PieceType::Rook).0,
            board.pieces(PieceType::Bishop).0,
            board.pieces(PieceType::Knight).0,
            board.pieces(PieceType::Pawn).0,
            0,
            0,
            ep_square,
            board.side_to_move() == Color::White,
        )
    };

    match code {
        TB_WIN => Some(GameOutcome::Win),
        TB_LOSS => Some(GameOutcome::Loss),
        TB_DRAW => Some(GameOutcome::Draw),
        _ => None,
    }
}

pub fn rank_rootmoves(td: &mut ThreadData) {
    let mut rootmoves_in_c: mem::MaybeUninit<TbRootMoves> = mem::MaybeUninit::uninit();
    let ep_square = tb_en_passant_square(&td.board);

    unsafe {
        let tb_ptr = rootmoves_in_c.as_mut_ptr();

        (*tb_ptr).size = td.root_moves.len().min(TB_MAX_MOVES as usize) as u32;

        for (i, root_move) in td.root_moves.iter().enumerate() {
            if i >= TB_MAX_MOVES as usize {
                break;
            }

            let c_move_ptr = (*tb_ptr).moves.as_mut_ptr().add(i);
            ptr::write(
                c_move_ptr,
                TbRootMove {
                    move_: root_move.mv.to_tb_move(),
                    pv: [0; MAX_PLY],
                    pvSize: 0,
                    tbScore: 0,
                    tbRank: 0,
                },
            );
        }

        // Helper to copy back from C struct and sort
        let update_rootmoves = |root_moves: &mut Vec<RootMove>, c_rootmoves: &TbRootMoves| {
            for i in 0..c_rootmoves.size as usize {
                let tb_move = c_rootmoves.moves[i].move_;
                if let Some(rm) = root_moves.iter_mut().find(|rm| rm.mv.to_tb_move() == tb_move) {
                    rm.tb_score = c_rootmoves.moves[i].tbScore;
                    rm.tb_rank = c_rootmoves.moves[i].tbRank;
                }
            }
            root_moves.sort_by_key(|rm| std::cmp::Reverse(rm.tb_rank));
        };

        let dtz_success = tb_probe_root_dtz(
            td.board.colors(Color::White).0,
            td.board.colors(Color::Black).0,
            td.board.pieces(PieceType::King).0,
            td.board.pieces(PieceType::Queen).0,
            td.board.pieces(PieceType::Rook).0,
            td.board.pieces(PieceType::Bishop).0,
            td.board.pieces(PieceType::Knight).0,
            td.board.pieces(PieceType::Pawn).0,
            td.board.fiftymove_clock() as u32,
            0,
            ep_square,
            td.board.side_to_move() == Color::White,
            td.board.has_repeated(),
            true,
            tb_ptr,
        );

        if dtz_success != 0 {
            let c_rootmoves: &TbRootMoves = &*tb_ptr;
            update_rootmoves(&mut td.root_moves, c_rootmoves);
            td.shared.stop_probing_tb.store(true, Ordering::Relaxed);
            td.shared.root_in_tb.store(true, Ordering::Relaxed);
            return;
        }

        // fallback to wdl
        let wdl_success = tb_probe_root_wdl(
            td.board.colors(Color::White).0,
            td.board.colors(Color::Black).0,
            td.board.pieces(PieceType::King).0,
            td.board.pieces(PieceType::Queen).0,
            td.board.pieces(PieceType::Rook).0,
            td.board.pieces(PieceType::Bishop).0,
            td.board.pieces(PieceType::Knight).0,
            td.board.pieces(PieceType::Pawn).0,
            td.board.fiftymove_clock() as u32,
            0,
            ep_square,
            td.board.side_to_move() == Color::White,
            true,
            tb_ptr,
        );

        if wdl_success != 0 {
            let c_rootmoves: &TbRootMoves = &*tb_ptr;
            update_rootmoves(&mut td.root_moves, c_rootmoves);
            td.shared.root_in_tb.store(true, Ordering::Relaxed);

            // Keep probing in search if DTZ is not available and we are winning
            td.shared.stop_probing_tb.store(td.root_moves[0].tb_score <= Score::ZERO, Ordering::Relaxed);
        }
    }
}

pub fn normalize_draw_ranks(td: &mut ThreadData) {
    if !td.shared.root_in_tb.load(Ordering::Relaxed) {
        return;
    }

    for i in 0..td.root_moves.len() {
        let mv = td.root_moves[i].mv;
        td.board.make_move(mv, &mut NullBoardObserver);
        let draw = td.board.is_draw(1);
        td.board.undo_move(mv);
        if draw {
            td.root_moves[i].tb_rank = 0;
            td.root_moves[i].tb_score = Score::ZERO;
        }
    }

    td.root_moves.sort_by_key(|rm| std::cmp::Reverse(rm.tb_rank));
}

const fn tb_en_passant_square(board: &Board) -> u32 {
    board.en_passant() as u32 & 0x3F
}
