//! NNUE Feature Mapping — Akimbo-compatible king-relative HalfKP indexing.
//!
//! Feature index = bucket * 768 + relative_color * 384 + piece * 64 + square
//!
//! The feature mapping is handled by `network::get_base_index`, which does:
//! - Horizontal mirroring (king files E-H → A-D via XOR 7)
//! - Rank flip for black perspective (XOR 56)
//! - Color-relative mapping (friendly=0, enemy=384)
//! - Bucket selection via the BUCKETS table
//!
//! This module provides utility functions for feature manipulation.

use super::network::{BUCKETS, NUM_BUCKETS};

/// Total effective buckets (including the mirrored king-side values 4-7).
pub const TOTAL_BUCKETS: usize = NUM_BUCKETS * 2;

/// Check if a king move changes the bucket (requires accumulator refresh).
#[inline(always)]
pub fn king_bucket_changed(perspective: usize, from_sq: usize, to_sq: usize) -> bool {
    let from = if perspective == 1 {
        from_sq ^ 56
    } else {
        from_sq
    };
    let to = if perspective == 1 { to_sq ^ 56 } else { to_sq };
    BUCKETS[from] != BUCKETS[to]
}

/// Mirror a square horizontally (flip file): a↔h, b↔g, c↔f, d↔e.
#[inline(always)]
pub const fn mirror_horizontal(sq: usize) -> usize {
    sq ^ 7
}

/// Check if we need to mirror based on king position.
#[inline(always)]
pub const fn king_needs_mirror(king_sq: usize) -> bool {
    (king_sq % 8) > 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mirror_involution() {
        for sq in 0..64 {
            assert_eq!(mirror_horizontal(mirror_horizontal(sq)), sq);
        }
    }

    #[test]
    fn test_bucket_range() {
        for sq in 0..64 {
            assert!(
                BUCKETS[sq] < TOTAL_BUCKETS,
                "Bucket {} out of range for sq={}",
                BUCKETS[sq],
                sq
            );
        }
    }

    #[test]
    fn test_king_bucket_changed() {
        // Moving king within same bucket region should not change
        // Square 0 and 1 are both bucket 0
        assert!(!king_bucket_changed(0, 0, 1));
        // Square 0 (bucket 0) and 16 (bucket 3) should change
        assert!(king_bucket_changed(0, 0, 16));
    }
}
