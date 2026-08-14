//! Syzygy tablebase path handling and optional Fathom WDL probes.
//!
//! Fathom (`tbprobe.c`) is MIT/zlib and lives with the v60 native tree. This
//! module only links it when the `syzygy` feature is enabled so default search
//! unit tests stay free of a C toolchain.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use types::Board;
#[cfg(feature = "syzygy")]
use types::{Color, Piece};

#[cfg(any(test, feature = "syzygy"))]
const TB_WIN_SCORE: i32 = 20_000;
#[cfg(any(test, feature = "syzygy"))]
const TB_LOSS: u32 = 0;
#[cfg(any(test, feature = "syzygy"))]
const TB_BLESSED_LOSS: u32 = 1;
#[cfg(any(test, feature = "syzygy"))]
const TB_DRAW: u32 = 2;
#[cfg(any(test, feature = "syzygy"))]
const TB_CURSED_WIN: u32 = 3;
#[cfg(any(test, feature = "syzygy"))]
const TB_WIN: u32 = 4;
#[cfg(any(test, feature = "syzygy"))]
const TB_RESULT_FAILED: u32 = 0xFFFF_FFFF;

/// Default UCI SyzygyProbeLimit (0 disables probes).
pub const DEFAULT_PROBE_LIMIT: u32 = 7;
/// Default UCI SyzygyProbeDepth (plies before in-tree WDL probes).
pub const DEFAULT_PROBE_DEPTH: i32 = 1;

static INIT: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug)]
pub struct SyzygyTables {
    path: String,
    largest: u32,
    files: usize,
    ready: bool,
    probe_limit: u32,
    probe_depth: i32,
}

impl Default for SyzygyTables {
    fn default() -> Self {
        Self::empty()
    }
}

impl SyzygyTables {
    pub const fn empty() -> Self {
        Self {
            path: String::new(),
            largest: 0,
            files: 0,
            ready: false,
            probe_limit: DEFAULT_PROBE_LIMIT,
            probe_depth: DEFAULT_PROBE_DEPTH,
        }
    }

    pub fn from_path(path: &str) -> Self {
        let trimmed = path.trim();
        if trimmed.is_empty() || trimmed == "<empty>" {
            return Self::empty();
        }
        let discovered = discover_tables(trimmed);
        let mut tables = Self {
            path: trimmed.to_string(),
            largest: discovered.0,
            files: discovered.1,
            ready: false,
            probe_limit: DEFAULT_PROBE_LIMIT,
            probe_depth: DEFAULT_PROBE_DEPTH,
        };
        tables.ready = init_probe(&tables.path);
        if tables.ready {
            tables.largest = tables.largest.max(probe_largest());
        }
        tables
    }

    #[inline(always)]
    pub fn is_ready(&self) -> bool {
        self.ready && self.largest >= 3
    }

    #[inline(always)]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[inline(always)]
    pub fn largest(&self) -> u32 {
        self.largest
    }

    #[inline(always)]
    pub fn file_count(&self) -> usize {
        self.files
    }

    #[inline(always)]
    pub fn probe_limit(&self) -> u32 {
        self.probe_limit
    }

    #[inline(always)]
    pub fn probe_depth(&self) -> i32 {
        self.probe_depth
    }

    pub fn set_probe_limit(&mut self, limit: u32) {
        self.probe_limit = limit.min(7);
    }

    pub fn set_probe_depth(&mut self, depth: i32) {
        self.probe_depth = depth.clamp(1, 100);
    }

    #[inline(always)]
    pub fn probe_wdl(&self, board: &Board) -> Option<i32> {
        if !self.is_ready() || self.probe_limit < 3 {
            return None;
        }
        let pieces = board.all_occupancy().count_ones();
        if pieces < 3
            || pieces > self.largest
            || pieces > self.probe_limit
            || board.castling_rights != 0
        {
            return None;
        }
        wdl_score(board)
    }
}

fn discover_tables(path: &str) -> (u32, usize) {
    let mut largest = 0u32;
    let mut files = 0usize;
    for part in path.split([':', ';']) {
        let dir = Path::new(part);
        if !dir.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if !(lower.ends_with(".rtbw") || lower.ends_with(".rtbz")) {
                continue;
            }
            files += 1;
            largest = largest.max(pieces_in_table_name(name));
        }
    }
    (largest, files)
}

fn pieces_in_table_name(name: &str) -> u32 {
    name.chars()
        .take_while(|ch| *ch != '.')
        .filter(|ch| {
            matches!(
                ch,
                'K' | 'Q' | 'R' | 'B' | 'N' | 'P' | 'k' | 'q' | 'r' | 'b' | 'n' | 'p'
            )
        })
        .count() as u32
}

#[cfg(feature = "syzygy")]
fn init_probe(path: &str) -> bool {
    let _guard = INIT.lock().unwrap_or_else(|err| err.into_inner());
    let c_path = match std::ffi::CString::new(path) {
        Ok(path) => path,
        Err(_) => return false,
    };
    unsafe { bindings::tb_init(c_path.as_ptr()) }
}

#[cfg(not(feature = "syzygy"))]
fn init_probe(_path: &str) -> bool {
    drop(INIT.lock());
    false
}

#[cfg(feature = "syzygy")]
fn probe_largest() -> u32 {
    unsafe { bindings::TB_LARGEST }
}

#[cfg(not(feature = "syzygy"))]
fn probe_largest() -> u32 {
    0
}

#[cfg(feature = "syzygy")]
fn wdl_score(board: &Board) -> Option<i32> {
    let white = board.color_occupancy(Color::White);
    let black = board.color_occupancy(Color::Black);
    let ep = board.en_passant.map_or(0, |sq| sq.index() as u32);
    let wdl = unsafe {
        bindings::tb_probe_wdl(
            white,
            black,
            board.piece_bb(Piece::King, Color::White) | board.piece_bb(Piece::King, Color::Black),
            board.piece_bb(Piece::Queen, Color::White) | board.piece_bb(Piece::Queen, Color::Black),
            board.piece_bb(Piece::Rook, Color::White) | board.piece_bb(Piece::Rook, Color::Black),
            board.piece_bb(Piece::Bishop, Color::White)
                | board.piece_bb(Piece::Bishop, Color::Black),
            board.piece_bb(Piece::Knight, Color::White)
                | board.piece_bb(Piece::Knight, Color::Black),
            board.piece_bb(Piece::Pawn, Color::White) | board.piece_bb(Piece::Pawn, Color::Black),
            board.halfmove_clock,
            u32::from(board.castling_rights),
            ep,
            board.side_to_move == Color::White,
        )
    };
    score_from_wdl(wdl)
}

#[cfg(not(feature = "syzygy"))]
fn wdl_score(_board: &Board) -> Option<i32> {
    None
}

#[cfg(any(test, feature = "syzygy"))]
#[inline(always)]
fn score_from_wdl(wdl: u32) -> Option<i32> {
    match wdl {
        TB_LOSS => Some(-TB_WIN_SCORE),
        TB_BLESSED_LOSS | TB_DRAW | TB_CURSED_WIN => Some(0),
        TB_WIN => Some(TB_WIN_SCORE),
        TB_RESULT_FAILED => None,
        _ => None,
    }
}

pub fn default_syzygy_dir() -> PathBuf {
    if let Ok(current) = std::env::current_dir() {
        let dist = current.join("dist").join("syzygy");
        if dist.is_dir() {
            return dist;
        }
        return current.join("syzygy");
    }
    PathBuf::from("syzygy")
}

#[cfg(feature = "syzygy")]
mod bindings {
    use std::os::raw::{c_char, c_uint};

    unsafe extern "C" {
        pub static mut TB_LARGEST: c_uint;
        pub fn tb_init(path: *const c_char) -> bool;
        pub fn tb_probe_wdl(
            white: u64,
            black: u64,
            kings: u64,
            queens: u64,
            rooks: u64,
            bishops: u64,
            knights: u64,
            pawns: u64,
            rule50: c_uint,
            castling: c_uint,
            ep: c_uint,
            turn: bool,
        ) -> c_uint;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_path_is_inactive() {
        let tables = SyzygyTables::from_path("");
        assert!(!tables.is_ready());
        assert_eq!(tables.largest(), 0);
    }

    #[test]
    fn table_name_piece_count_matches_syzygy_convention() {
        assert_eq!(pieces_in_table_name("KQvK.rtbw"), 3);
        assert_eq!(pieces_in_table_name("KQPvKR.rtbz"), 5);
    }

    #[test]
    fn missing_directory_stays_inactive() {
        let tables = SyzygyTables::from_path("/tmp/mujrim-missing-syzygy");
        assert!(!tables.is_ready());
        assert_eq!(tables.file_count(), 0);
    }

    #[test]
    fn probe_limit_zero_disables_wdl() {
        let mut tables = SyzygyTables::empty();
        tables.set_probe_limit(0);
        assert_eq!(tables.probe_limit(), 0);
        assert_eq!(tables.probe_wdl(&Board::new()), None);
        tables.set_probe_depth(12);
        assert_eq!(tables.probe_depth(), 12);
    }

    #[test]
    fn wdl_mapping_covers_terminal_results() {
        assert_eq!(score_from_wdl(TB_WIN), Some(TB_WIN_SCORE));
        assert_eq!(score_from_wdl(TB_LOSS), Some(-TB_WIN_SCORE));
        assert_eq!(score_from_wdl(TB_DRAW), Some(0));
        assert_eq!(score_from_wdl(TB_RESULT_FAILED), None);
    }
}
