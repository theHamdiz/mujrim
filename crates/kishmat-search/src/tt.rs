//! Thread-safe transposition table with bucket system and aging.
//! Uses lock-free reads/writes for Lazy SMP — benign data races are
//! acceptable since hash verification catches corruption.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use types::Move;
use types::chess_move::NULL_MOVE;

/// Entry type: how the score was derived.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeType {
    Exact = 0,
    LowerBound = 1,
    UpperBound = 2,
}

impl From<u8> for NodeType {
    #[inline(always)]
    fn from(v: u8) -> Self {
        match v {
            0 => NodeType::Exact,
            1 => NodeType::LowerBound,
            _ => NodeType::UpperBound,
        }
    }
}

/// A single TT entry packed for atomic access.
/// Layout: hash(8) + data(8) where data packs depth, score, node_type, move, age.
#[derive(Copy, Clone, Debug)]
pub struct TTEntry {
    pub hash: u64,
    pub depth: i32,
    pub score: i32,
    pub node_type: NodeType,
    pub best_move: Move,
    pub age: u8,
    /// Was this position part of a PV line? (Stockfish/Viridithas trick)
    pub was_pv: bool,
}

impl Default for TTEntry {
    #[inline(always)]
    fn default() -> Self {
        Self {
            hash: 0,
            depth: 0,
            score: 0,
            node_type: NodeType::Exact,
            best_move: NULL_MOVE,
            age: 0,
            was_pv: false,
        }
    }
}

/// Number of entries per bucket.
const BUCKET_SIZE: usize = 4;

/// An atomic TT entry — stores hash and packed data as two u64s.
/// This allows lock-free concurrent access.
#[repr(C)]
struct AtomicEntry {
    key: AtomicU64,
    data: AtomicU64,
}

impl AtomicEntry {
    fn new() -> Self {
        Self {
            key: AtomicU64::new(0),
            data: AtomicU64::new(0),
        }
    }

    /// Packs an entry into key + data.
    #[inline(always)]
    fn store(&self, entry: &TTEntry) {
        let data = pack_entry(entry);
        // XOR key with data for verification (Stockfish technique)
        let key = entry.hash ^ data;
        self.key.store(key, Ordering::Relaxed);
        self.data.store(data, Ordering::Relaxed);
    }

    /// Tries to load an entry. Returns None if hash doesn't match.
    #[inline(always)]
    fn load(&self, hash: u64) -> Option<TTEntry> {
        let data = self.data.load(Ordering::Relaxed);
        let key = self.key.load(Ordering::Relaxed);
        // Verify: key XOR data should equal the original hash
        if key ^ data == hash {
            Some(unpack_entry(hash, data))
        } else {
            None
        }
    }

    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.key.load(Ordering::Relaxed) == 0 && self.data.load(Ordering::Relaxed) == 0
    }

    fn clear(&self) {
        self.key.store(0, Ordering::Relaxed);
        self.data.store(0, Ordering::Relaxed);
    }
}

/// Pack entry fields into a single u64.
/// Layout: depth(16) | score(16) | from(6) | to(6) | flag(3) | promo(3) | node_type(2) | age(8) = 60 bits
#[inline(always)]
fn pack_entry(e: &TTEntry) -> u64 {
    let depth = (e.depth as i16) as u16;
    let score = (e.score as i16) as u16;
    let from = e.best_move.from.index() as u64;
    let to = e.best_move.to.index() as u64;
    let flag = e.best_move.flag as u64;
    let promo: u64 = match e.best_move.promotion {
        None => 0,
        Some(types::Piece::Queen) => 1,
        Some(types::Piece::Rook) => 2,
        Some(types::Piece::Bishop) => 3,
        Some(types::Piece::Knight) => 4,
        _ => 0,
    };
    let nt = e.node_type as u64;
    let age = e.age as u64;
    let pv = if e.was_pv { 1u64 } else { 0u64 };

    (depth as u64)
        | ((score as u64) << 16)
        | (from << 32)
        | (to << 38)
        | (flag << 44)
        | (promo << 47)
        | (nt << 50)
        | (age << 52)
        | (pv << 60)
}

/// Unpack a u64 back into entry fields.
#[inline(always)]
fn unpack_entry(hash: u64, data: u64) -> TTEntry {
    let depth = (data & 0xFFFF) as u16 as i16 as i32;
    let score = ((data >> 16) & 0xFFFF) as u16 as i16 as i32;
    let from = ((data >> 32) & 0x3F) as usize;
    let to = ((data >> 38) & 0x3F) as usize;
    let flag_val = ((data >> 44) & 0x7) as u8;
    let promo_val = ((data >> 47) & 0x7) as u8;
    let nt = ((data >> 50) & 0x3) as u8;
    let age = ((data >> 52) & 0xFF) as u8;
    let was_pv = ((data >> 60) & 0x1) != 0;

    let flag = match flag_val {
        0 => types::chess_move::MoveFlag::Quiet,
        1 => types::chess_move::MoveFlag::DoublePawn,
        2 => types::chess_move::MoveFlag::KingCastle,
        3 => types::chess_move::MoveFlag::QueenCastle,
        4 => types::chess_move::MoveFlag::Capture,
        5 => types::chess_move::MoveFlag::EnPassant,
        6 => types::chess_move::MoveFlag::Promotion,
        7 => types::chess_move::MoveFlag::PromotionCapture,
        _ => types::chess_move::MoveFlag::Quiet,
    };

    let promotion = match promo_val {
        1 => Some(types::Piece::Queen),
        2 => Some(types::Piece::Rook),
        3 => Some(types::Piece::Bishop),
        4 => Some(types::Piece::Knight),
        _ => None,
    };

    TTEntry {
        hash,
        depth,
        score,
        node_type: NodeType::from(nt),
        best_move: Move {
            from: types::Square::from_index(from),
            to: types::Square::from_index(to),
            flag,
            promotion,
        },
        age,
        was_pv,
    }
}

/// A bucket of atomic TT entries.
#[repr(C)]
struct Bucket {
    entries: [AtomicEntry; BUCKET_SIZE],
}

impl Bucket {
    fn new() -> Self {
        Self {
            entries: std::array::from_fn(|_| AtomicEntry::new()),
        }
    }
}

/// Thread-safe transposition table.
/// Safe to share between threads via Arc for Lazy SMP.
pub struct TranspositionTable {
    buckets: Vec<Bucket>,
    mask: usize,
    generation: AtomicU8,
}

// SAFETY: The TT uses atomic operations for all reads/writes.
// Benign data races on entries are acceptable — hash verification catches corruption.
unsafe impl Sync for TranspositionTable {}
unsafe impl Send for TranspositionTable {}

impl TranspositionTable {
    /// Creates a new TT with the given size in megabytes.
    pub fn new(size_mb: usize) -> Self {
        let bucket_size = std::mem::size_of::<Bucket>();
        let num_buckets = (size_mb * 1024 * 1024 / bucket_size).next_power_of_two();
        let mask = num_buckets - 1;

        let mut buckets = Vec::with_capacity(num_buckets);
        for _ in 0..num_buckets {
            buckets.push(Bucket::new());
        }

        Self {
            buckets,
            mask,
            generation: AtomicU8::new(0),
        }
    }

    /// Increments the generation counter.
    #[inline(always)]
    pub fn new_generation(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    #[inline(always)]
    pub fn current_generation(&self) -> u8 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Probes the TT for an entry matching the given hash.
    #[inline(always)]
    pub fn probe(&self, hash: u64) -> Option<TTEntry> {
        let idx = (hash as usize) & self.mask;
        let bucket = &self.buckets[idx];
        for entry in &bucket.entries {
            if let Some(e) = entry.load(hash) {
                return Some(e);
            }
        }
        None
    }

    /// Prefetch the TT bucket for a hash.
    #[inline(always)]
    pub fn prefetch(&self, hash: u64) {
        let idx = (hash as usize) & self.mask;
        let ptr = &self.buckets[idx] as *const Bucket as *const u8;
        #[cfg(target_arch = "x86_64")]
        unsafe {
            std::arch::x86_64::_mm_prefetch(ptr as *const i8, std::arch::x86_64::_MM_HINT_T0);
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            // PLDL1KEEP: prefetch for load into L1 cache, temporal locality
            std::arch::asm!("prfm pldl1keep, [{ptr}]", ptr = in(reg) ptr);
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        let _ = ptr;
    }

    /// Stores an entry in the TT with smart replacement.
    #[inline(always)]
    pub fn store(&self, hash: u64, depth: i32, score: i32, node_type: NodeType, best_move: Move, was_pv: bool) {
        let idx = (hash as usize) & self.mask;
        let bucket = &self.buckets[idx];
        let current_gen = self.current_generation();

        let new_entry = TTEntry {
            hash,
            depth,
            score,
            node_type,
            best_move,
            age: current_gen,
            was_pv,
        };

        // Find best slot to replace
        let mut replace_idx = 0;
        let mut replace_score = i32::MAX;

        for (i, entry) in bucket.entries.iter().enumerate() {
            // Same position: always replace
            if let Some(_existing) = entry.load(hash) {
                bucket.entries[i].store(&new_entry);
                return;
            }

            // Empty slot
            if entry.is_empty() {
                entry.store(&new_entry);
                return;
            }

            // Try to load any entry for scoring
            let data = entry.data.load(Ordering::Relaxed);
            let entry_depth = (data & 0xFFFF) as u16 as i16 as i32;
            let entry_age = ((data >> 52) & 0xFF) as u8;

            let age_penalty = if entry_age != current_gen { -1000 } else { 0 };
            let entry_score = entry_depth + age_penalty;

            if entry_score < replace_score {
                replace_score = entry_score;
                replace_idx = i;
            }
        }

        // Replace the worst entry
        bucket.entries[replace_idx].store(&new_entry);
    }

    /// Clears the entire transposition table.
    pub fn clear(&self) {
        for bucket in &self.buckets {
            for entry in &bucket.entries {
                entry.clear();
            }
        }
        self.generation.store(0, Ordering::Relaxed);
    }
}

impl Default for TranspositionTable {
    fn default() -> Self {
        Self::new(64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::Square;

    #[test]
    fn test_tt_store_and_probe() {
        let tt = TranspositionTable::new(1);
        let mv = Move::quiet(Square::E2, Square::E4);
        tt.store(12345, 5, 100, NodeType::Exact, mv, false);

        let entry = tt.probe(12345).unwrap();
        assert_eq!(entry.depth, 5);
        assert_eq!(entry.score, 100);
        assert_eq!(entry.best_move.from, Square::E2);
    }

    #[test]
    fn test_tt_miss() {
        let tt = TranspositionTable::new(1);
        assert!(tt.probe(99999).is_none());
    }

    #[test]
    fn test_tt_generation() {
        let tt = TranspositionTable::new(1);
        let mv = Move::quiet(Square::E2, Square::E4);
        tt.store(12345, 5, 100, NodeType::Exact, mv, false);
        assert_eq!(tt.probe(12345).unwrap().age, 0);

        tt.new_generation();
        tt.store(12345, 5, 200, NodeType::Exact, mv, false);
        assert_eq!(tt.probe(12345).unwrap().age, 1);
        assert_eq!(tt.probe(12345).unwrap().score, 200);
    }

    #[test]
    fn test_tt_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        let tt = Arc::new(TranspositionTable::new(1));

        let handles: Vec<_> = (0..4).map(|t| {
            let tt = Arc::clone(&tt);
            thread::spawn(move || {
                let mv = Move::quiet(Square::E2, Square::E4);
                for i in 0..1000u64 {
                    let hash = t * 100000 + i;
                    tt.store(hash, 5, i as i32, NodeType::Exact, mv, false);
                }
            })
        }).collect();

        for h in handles { h.join().unwrap(); }

        // Verify some entries are retrievable
        let mv = Move::quiet(Square::E2, Square::E4);
        let mut found = 0;
        for t in 0..4u64 {
            for i in 0..1000u64 {
                if tt.probe(t * 100000 + i).is_some() {
                    found += 1;
                }
            }
        }
        assert!(found > 0, "Should find at least some entries after concurrent writes");
    }
}
