//! Thread-safe transposition table with bucket system and aging.
//! Uses lock-free reads/writes for Lazy SMP — benign data races are
//! acceptable since hash verification catches corruption.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
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
    /// Uncorrected static evaluation, when one was available at this node.
    pub raw_eval: Option<i32>,
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
            raw_eval: None,
            node_type: NodeType::Exact,
            best_move: NULL_MOVE,
            age: 0,
            was_pv: false,
        }
    }
}

/// Search result payload written to the transposition table.
#[derive(Copy, Clone, Debug)]
pub struct TTData {
    pub depth: i32,
    pub score: i32,
    pub node_type: NodeType,
    pub best_move: Move,
    pub was_pv: bool,
    pub raw_eval: Option<i32>,
}

impl TTData {
    pub const fn new(
        depth: i32,
        score: i32,
        node_type: NodeType,
        best_move: Move,
        was_pv: bool,
        raw_eval: Option<i32>,
    ) -> Self {
        Self {
            depth,
            score,
            node_type,
            best_move,
            was_pv,
            raw_eval,
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

#[inline(always)]
fn pack_move(mv: Move) -> u16 {
    use types::Piece;
    use types::chess_move::MoveFlag;

    let promotion = match mv.promotion {
        Some(Piece::Queen) => 0,
        Some(Piece::Rook) => 1,
        Some(Piece::Bishop) => 2,
        Some(Piece::Knight) => 3,
        _ => 0,
    };
    let kind = match mv.flag {
        MoveFlag::Quiet => 0,
        MoveFlag::DoublePawn => 1,
        MoveFlag::KingCastle => 2,
        MoveFlag::QueenCastle => 3,
        MoveFlag::Capture => 4,
        MoveFlag::EnPassant => 5,
        MoveFlag::Promotion => 6 + promotion,
        MoveFlag::PromotionCapture => 10 + promotion,
    };

    mv.from.index() as u16 | ((mv.to.index() as u16) << 6) | (kind << 12)
}

#[inline(always)]
fn unpack_move(packed: u16) -> Move {
    use types::Piece;
    use types::chess_move::MoveFlag;

    let kind = ((packed >> 12) & 0xf) as u8;
    let (flag, promotion) = match kind {
        0 => (MoveFlag::Quiet, None),
        1 => (MoveFlag::DoublePawn, None),
        2 => (MoveFlag::KingCastle, None),
        3 => (MoveFlag::QueenCastle, None),
        4 => (MoveFlag::Capture, None),
        5 => (MoveFlag::EnPassant, None),
        6..=9 => (
            MoveFlag::Promotion,
            Some([Piece::Queen, Piece::Rook, Piece::Bishop, Piece::Knight][(kind - 6) as usize]),
        ),
        10..=13 => (
            MoveFlag::PromotionCapture,
            Some([Piece::Queen, Piece::Rook, Piece::Bishop, Piece::Knight][(kind - 10) as usize]),
        ),
        _ => (MoveFlag::Quiet, None),
    };

    Move {
        from: types::Square::from_index((packed & 0x3f) as usize),
        to: types::Square::from_index(((packed >> 6) & 0x3f) as usize),
        flag,
        promotion,
    }
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
/// Layout: depth(8) | score(16) | raw_eval(16) | move(16) | node_type(2) | age(5) | pv(1).
#[inline(always)]
fn pack_entry(e: &TTEntry) -> u64 {
    let depth = e.depth.clamp(0, u8::MAX as i32) as u8;
    let score = (e.score as i16) as u16;
    let raw_eval = match e.raw_eval {
        Some(eval) => eval.clamp(i16::MIN as i32 + 1, i16::MAX as i32) as i16,
        None => i16::MIN,
    };
    let packed_move = pack_move(e.best_move);
    let nt = e.node_type as u64;
    let age = u64::from(e.age & 0x1f);
    let pv = if e.was_pv { 1u64 } else { 0u64 };

    u64::from(depth)
        | (u64::from(score) << 8)
        | (u64::from(raw_eval as u16) << 24)
        | (u64::from(packed_move) << 40)
        | (nt << 56)
        | (age << 58)
        | (pv << 63)
}

/// Unpack a u64 back into entry fields.
#[inline(always)]
fn unpack_entry(hash: u64, data: u64) -> TTEntry {
    let depth = (data & 0xff) as i32;
    let score = ((data >> 8) & 0xffff) as u16 as i16 as i32;
    let packed_eval = ((data >> 24) & 0xffff) as u16 as i16;
    let raw_eval = (packed_eval != i16::MIN).then_some(i32::from(packed_eval));
    let best_move = unpack_move(((data >> 40) & 0xffff) as u16);
    let nt = ((data >> 56) & 0x3) as u8;
    let age = ((data >> 58) & 0x1f) as u8;
    let was_pv = (data >> 63) != 0;

    TTEntry {
        hash,
        depth,
        score,
        raw_eval,
        node_type: NodeType::from(nt),
        best_move,
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
// SAFETY: ownership transfer is safe because every shared mutable field is
// accessed exclusively through atomics.
unsafe impl Send for TranspositionTable {}

impl TranspositionTable {
    /// Creates a new TT with the given size in megabytes.
    pub fn new(size_mb: usize) -> Self {
        let bucket_size = std::mem::size_of::<Bucket>();
        let requested_bytes = size_mb.saturating_mul(1024 * 1024);
        let available_buckets = (requested_bytes / bucket_size).max(1);
        let num_buckets = 1usize << available_buckets.ilog2();
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

    /// Approximate occupancy of entries from the current search generation,
    /// expressed in UCI permille units (0..=1000).
    pub fn hashfull_per_mille(&self) -> u16 {
        let sampled_buckets = self.buckets.len().min(250);
        let current_generation = self.current_generation() & 0x1f;
        let used = self.buckets[..sampled_buckets]
            .iter()
            .flat_map(|bucket| &bucket.entries)
            .filter(|entry| {
                let data = entry.data.load(Ordering::Relaxed);
                !entry.is_empty() && ((data >> 58) & 0x1f) as u8 == current_generation
            })
            .count();
        ((used * 1000) / (sampled_buckets * BUCKET_SIZE)) as u16
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
        // SAFETY: `ptr` addresses a live bucket in `self`; `_mm_prefetch` only
        // issues a cache hint and does not expose a Rust reference.
        unsafe {
            std::arch::x86_64::_mm_prefetch(ptr as *const i8, std::arch::x86_64::_MM_HINT_T0);
        }
        #[cfg(target_arch = "aarch64")]
        // SAFETY: `ptr` addresses a live bucket in `self`; `prfm` only issues a
        // non-faulting cache hint and does not dereference it in Rust.
        unsafe {
            // PLDL1KEEP: prefetch for load into L1 cache, temporal locality
            std::arch::asm!("prfm pldl1keep, [{ptr}]", ptr = in(reg) ptr);
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        let _ = ptr;
    }

    /// Stores an entry in the TT with smart replacement.
    #[inline(always)]
    pub fn store(&self, hash: u64, data: TTData) {
        let idx = (hash as usize) & self.mask;
        let bucket = &self.buckets[idx];
        let current_gen = self.current_generation() & 0x1f;

        let mut new_entry = TTEntry {
            hash,
            depth: data.depth,
            score: data.score,
            raw_eval: data.raw_eval,
            node_type: data.node_type,
            best_move: data.best_move,
            age: current_gen,
            was_pv: data.was_pv,
        };

        // Find best slot to replace
        let mut replace_idx = 0;
        let mut replace_score = i32::MAX;

        for (i, entry) in bucket.entries.iter().enumerate() {
            // Same position: depth-preferred replacement.
            if let Some(existing) = entry.load(hash) {
                if new_entry.raw_eval.is_none() {
                    new_entry.raw_eval = existing.raw_eval;
                }
                let should_replace = data.node_type == NodeType::Exact
                    || data.depth + 2 >= existing.depth
                    || existing.age != current_gen
                    || (data.node_type == NodeType::LowerBound
                        && existing.node_type == NodeType::UpperBound)
                    || (data.node_type == NodeType::UpperBound
                        && existing.node_type == NodeType::LowerBound
                        && data.depth >= existing.depth);
                if should_replace {
                    bucket.entries[i].store(&new_entry);
                }
                return;
            }

            // Empty slot
            if entry.is_empty() {
                entry.store(&new_entry);
                return;
            }

            // Try to load any entry for scoring
            let data = entry.data.load(Ordering::Relaxed);
            let entry_depth = (data & 0xff) as i32;
            let entry_age = ((data >> 58) & 0x1f) as u8;

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
        tt.store(
            12345,
            TTData::new(5, 100, NodeType::Exact, mv, false, Some(42)),
        );

        let entry = tt.probe(12345).unwrap();
        assert_eq!(entry.depth, 5);
        assert_eq!(entry.score, 100);
        assert_eq!(entry.raw_eval, Some(42));
        assert_eq!(entry.best_move.from, Square::E2);
    }

    #[test]
    fn test_tt_miss() {
        let tt = TranspositionTable::new(1);
        assert!(tt.probe(99999).is_none());
    }

    #[test]
    fn requested_hash_size_is_never_rounded_up() {
        let bucket_size = std::mem::size_of::<Bucket>();
        for requested_mb in [1usize, 3, 5, 48, 96] {
            let tt = TranspositionTable::new(requested_mb);
            let allocated_bytes = tt.buckets.len() * bucket_size;
            assert!(allocated_bytes <= requested_mb * 1024 * 1024);
            assert!(tt.buckets.len().is_power_of_two());
        }
    }

    #[test]
    fn hashfull_reports_current_generation_only() {
        let tt = TranspositionTable::new(1);
        assert_eq!(tt.hashfull_per_mille(), 0);
        for hash in 0..64 {
            tt.store(
                hash,
                TTData::new(1, 0, NodeType::Exact, NULL_MOVE, false, None),
            );
        }
        assert!(tt.hashfull_per_mille() > 0);
        tt.new_generation();
        assert_eq!(tt.hashfull_per_mille(), 0);
    }

    #[test]
    fn packed_moves_preserve_special_kinds_and_promotions() {
        let moves = [
            Move::quiet(Square::E2, Square::E4),
            Move::en_passant(Square::E5, Square::D6),
            Move::king_castle(Square::E1, Square::G1),
            Move::promotion(Square::A7, Square::A8, types::Piece::Knight),
            Move::promotion_capture(Square::B7, Square::A8, types::Piece::Queen),
        ];

        for mv in moves {
            assert_eq!(unpack_move(pack_move(mv)), mv);
        }
        assert_eq!(std::mem::size_of::<AtomicEntry>(), 16);
    }

    #[test]
    fn test_tt_generation() {
        let tt = TranspositionTable::new(1);
        let mv = Move::quiet(Square::E2, Square::E4);
        tt.store(12345, TTData::new(5, 100, NodeType::Exact, mv, false, None));
        assert_eq!(tt.probe(12345).unwrap().age, 0);

        tt.new_generation();
        tt.store(12345, TTData::new(5, 200, NodeType::Exact, mv, false, None));
        assert_eq!(tt.probe(12345).unwrap().age, 1);
        assert_eq!(tt.probe(12345).unwrap().score, 200);
    }

    #[test]
    fn test_tt_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        let tt = Arc::new(TranspositionTable::new(1));

        let handles: Vec<_> = (0..4)
            .map(|t| {
                let tt = Arc::clone(&tt);
                thread::spawn(move || {
                    let mv = Move::quiet(Square::E2, Square::E4);
                    for i in 0..1000u64 {
                        let hash = t * 100000 + i;
                        tt.store(
                            hash,
                            TTData::new(5, i as i32, NodeType::Exact, mv, false, None),
                        );
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        // Verify some entries are retrievable
        let mut found = 0;
        for t in 0..4u64 {
            for i in 0..1000u64 {
                if tt.probe(t * 100000 + i).is_some() {
                    found += 1;
                }
            }
        }
        assert!(
            found > 0,
            "Should find at least some entries after concurrent writes"
        );
    }

    #[test]
    fn test_tt_same_hash_prefers_deeper_non_exact() {
        let tt = TranspositionTable::new(1);
        let mv = Move::quiet(Square::E2, Square::E4);
        tt.store(
            12345,
            TTData::new(9, 100, NodeType::LowerBound, mv, false, None),
        );
        tt.store(
            12345,
            TTData::new(2, 120, NodeType::UpperBound, mv, false, None),
        );
        let entry = tt.probe(12345).unwrap();
        assert_eq!(entry.depth, 9);
        assert_eq!(entry.score, 100);
    }

    #[test]
    fn test_tt_same_hash_allows_exact_replacement() {
        let tt = TranspositionTable::new(1);
        let mv = Move::quiet(Square::E2, Square::E4);
        tt.store(
            777,
            TTData::new(9, 100, NodeType::LowerBound, mv, false, None),
        );
        tt.store(777, TTData::new(2, 50, NodeType::Exact, mv, false, None));
        let entry = tt.probe(777).unwrap();
        assert_eq!(entry.depth, 2);
        assert_eq!(entry.score, 50);
        assert_eq!(entry.node_type, NodeType::Exact);
    }
}
