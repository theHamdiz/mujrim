//! Obsidian layered NNUE (768→1536→16→32→1, 13 king buckets, 8 output buckets).
//!
//! Layout matches the published `Net` in gab8192/Obsidian `src/nnue.h`.
//! Evaluation is a refresh-from-scratch of the raw on-disk weights (no SIMD
//! packus transpose). Search insights stay in the Obsidian search profile.

use std::path::Path;

use types::{Board, Color, Piece};

pub const L1: usize = 1536;
pub const L2: usize = 16;
pub const L3: usize = 32;
pub const KING_BUCKETS: usize = 13;
pub const OUTPUT_BUCKETS: usize = 8;
pub const FEATURES: usize = 768;
pub const NETWORK_SCALE: i32 = 400;
pub const NETWORK_QA: i32 = 255;
pub const NETWORK_QB: i32 = 128;
const FT_SHIFT: i32 = 9;

/// Packed on-disk size of the published Obsidian `Net` (no trailing pad).
pub const FILE_SIZE: u64 = (KING_BUCKETS * 2 * 6 * 64 * L1 * 2
    + L1 * 2
    + OUTPUT_BUCKETS * L1 * L2
    + OUTPUT_BUCKETS * L2 * 4
    + OUTPUT_BUCKETS * (L2 * 2) * L3 * 4
    + OUTPUT_BUCKETS * L3 * 4
    + OUTPUT_BUCKETS * L3 * 4
    + OUTPUT_BUCKETS * 4) as u64;

#[rustfmt::skip]
const KING_BUCKETS_SCHEME: [usize; 64] = [
    0, 1, 2, 3, 3, 2, 1, 0,
    4, 5, 6, 7, 7, 6, 5, 4,
    8, 8, 9, 9, 9, 9, 8, 8,
    10, 10, 10, 10, 10, 10, 10, 10,
    11, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 11, 11,
    12, 12, 12, 12, 12, 12, 12, 12,
    12, 12, 12, 12, 12, 12, 12, 12,
];

pub struct ObsidianNetwork {
    feature_weights: Box<[i16]>,
    feature_biases: Box<[i16]>,
    l1_weights: Box<[i8]>,
    l1_biases: Box<[f32]>,
    l2_weights: Box<[f32]>,
    l2_biases: Box<[f32]>,
    l3_weights: Box<[f32]>,
    l3_biases: Box<[f32]>,
}

impl ObsidianNetwork {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < FILE_SIZE as usize {
            return Err(format!(
                "Obsidian NNUE too small: expected at least {FILE_SIZE} bytes, found {}",
                bytes.len()
            ));
        }
        let mut offset = 0;
        let feature_weights = read_i16s(bytes, &mut offset, KING_BUCKETS * 2 * 6 * 64 * L1)?;
        let feature_biases = read_i16s(bytes, &mut offset, L1)?;
        let l1_weights = read_i8s(bytes, &mut offset, OUTPUT_BUCKETS * L1 * L2)?;
        let l1_biases = read_f32s(bytes, &mut offset, OUTPUT_BUCKETS * L2)?;
        let l2_weights = read_f32s(bytes, &mut offset, OUTPUT_BUCKETS * (L2 * 2) * L3)?;
        let l2_biases = read_f32s(bytes, &mut offset, OUTPUT_BUCKETS * L3)?;
        let l3_weights = read_f32s(bytes, &mut offset, OUTPUT_BUCKETS * L3)?;
        let l3_biases = read_f32s(bytes, &mut offset, OUTPUT_BUCKETS)?;
        Ok(Self {
            feature_weights,
            feature_biases,
            l1_weights,
            l1_biases,
            l2_weights,
            l2_biases,
            l3_weights,
            l3_biases,
        })
    }

    #[inline(always)]
    pub fn evaluate(&self, board: &Board) -> i32 {
        let mut acc_white = [0i16; L1];
        let mut acc_black = [0i16; L1];
        acc_white.copy_from_slice(&self.feature_biases);
        acc_black.copy_from_slice(&self.feature_biases);
        for color in [Color::White, Color::Black] {
            let acc = if color == Color::White {
                &mut acc_white
            } else {
                &mut acc_black
            };
            let king = board.king_square(color).index();
            for piece in Piece::ALL {
                for piece_color in [Color::White, Color::Black] {
                    let mut bb = board.piece_bb(piece, piece_color);
                    while bb != 0 {
                        let sq = bb.trailing_zeros() as usize;
                        bb &= bb - 1;
                        add_feature(
                            acc,
                            &self.feature_weights,
                            king,
                            color,
                            piece,
                            piece_color,
                            sq,
                        );
                    }
                }
            }
        }

        let pieces = board.all_occupancy().count_ones() as i32;
        let divisor = (32 + OUTPUT_BUCKETS as i32 - 1) / OUTPUT_BUCKETS as i32;
        let bucket = ((pieces - 2) / divisor).clamp(0, OUTPUT_BUCKETS as i32 - 1) as usize;
        if board.side_to_move == Color::White {
            propagate(self, &acc_white, &acc_black, bucket)
        } else {
            propagate(self, &acc_black, &acc_white, bucket)
        }
    }
}

#[inline(always)]
fn add_feature(
    acc: &mut [i16; L1],
    weights: &[i16],
    king_sq: usize,
    side: Color,
    piece: Piece,
    piece_color: Color,
    mut sq: usize,
) {
    if king_sq & 0b100 != 0 {
        sq ^= 7;
    }
    let rel_king = relative_square(side, king_sq);
    let rel_sq = relative_square(side, sq);
    let bucket = KING_BUCKETS_SCHEME[rel_king];
    let them = usize::from(side != piece_color);
    let base = ((((bucket * 2 + them) * 6 + piece.index()) * 64) + rel_sq) * L1;
    debug_assert!(base + L1 <= weights.len());
    let acc_ptr = acc.as_mut_ptr();
    let weight_ptr = unsafe { weights.as_ptr().add(base) };
    for i in 0..L1 {
        unsafe {
            *acc_ptr.add(i) = (*acc_ptr.add(i)).wrapping_add(*weight_ptr.add(i));
        }
    }
}

#[inline(always)]
fn relative_square(side: Color, sq: usize) -> usize {
    if side == Color::Black { sq ^ 56 } else { sq }
}

#[inline(always)]
fn propagate(net: &ObsidianNetwork, us: &[i16; L1], them: &[i16; L1], bucket: usize) -> i32 {
    let mut ft_out = [0u8; L1];
    activate_ft(us, &mut ft_out[..L1 / 2]);
    activate_ft(them, &mut ft_out[L1 / 2..]);

    let scale = 1.0 / ((NETWORK_QA * NETWORK_QA * NETWORK_QB) >> FT_SHIFT) as f32;
    let mut l1 = [0.0f32; L2 * 2];
    let (l1_linear, l1_sqr) = l1.split_at_mut(L2);
    let l1_weight_base = bucket * L1 * L2;
    let l1_bias_base = bucket * L2;
    for (j, (linear, squared)) in l1_linear.iter_mut().zip(l1_sqr.iter_mut()).enumerate() {
        let sum = ft_out.iter().enumerate().fold(0i32, |acc, (i, feature)| {
            acc + i32::from(*feature) * i32::from(net.l1_weights[l1_weight_base + i * L2 + j])
        });
        let biased = sum as f32 * scale + net.l1_biases[l1_bias_base + j];
        *linear = biased.clamp(0.0, 1.0);
        *squared = (biased * biased).clamp(0.0, 1.0);
    }

    let mut l2 = [0.0f32; L3];
    let l2_weight_base = bucket * (L2 * 2) * L3;
    let l2_bias_base = bucket * L3;
    for (j, value) in l2.iter_mut().enumerate() {
        let sum = l1
            .iter()
            .enumerate()
            .fold(net.l2_biases[l2_bias_base + j], |acc, (i, feature)| {
                acc + net.l2_weights[l2_weight_base + i * L3 + j] * *feature
            });
        *value = sum.clamp(0.0, 1.0);
    }

    let l3 = l2
        .iter()
        .enumerate()
        .fold(net.l3_biases[bucket], |acc, (j, feature)| {
            acc + net.l3_weights[bucket * L3 + j] * *feature
        });
    (l3 * NETWORK_SCALE as f32) as i32
}

#[inline(always)]
fn activate_ft(acc: &[i16], out: &mut [u8]) {
    let half = L1 / 2;
    debug_assert_eq!(out.len(), half);
    for i in 0..half {
        let c0 = i32::from(acc[i]).clamp(0, NETWORK_QA);
        let c1 = i32::from(acc[i + half]).clamp(i32::MIN, NETWORK_QA);
        let shifted = c0 << (16 - FT_SHIFT);
        let prod = ((shifted * c1) >> 16).clamp(0, 255);
        out[i] = prod as u8;
    }
}

pub fn load(path: &Path) -> Result<Box<ObsidianNetwork>, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read Obsidian NNUE '{}': {error}", path.display()))?;
    ObsidianNetwork::from_bytes(&bytes).map(Box::new)
}

pub fn is_obsidian_path(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.contains("obsidian")
        || name.contains("net89")
        || name.contains("obs_default")
        || name.ends_with("perm.bin")
}

fn read_i16s(bytes: &[u8], offset: &mut usize, count: usize) -> Result<Box<[i16]>, String> {
    let need = count * 2;
    let slice = bytes
        .get(*offset..*offset + need)
        .ok_or_else(|| "truncated Obsidian i16 weights".to_string())?;
    *offset += need;
    Ok(slice
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect())
}

fn read_i8s(bytes: &[u8], offset: &mut usize, count: usize) -> Result<Box<[i8]>, String> {
    let slice = bytes
        .get(*offset..*offset + count)
        .ok_or_else(|| "truncated Obsidian i8 weights".to_string())?;
    *offset += count;
    Ok(slice.iter().map(|byte| *byte as i8).collect())
}

fn read_f32s(bytes: &[u8], offset: &mut usize, count: usize) -> Result<Box<[f32]>, String> {
    let need = count * 4;
    let slice = bytes
        .get(*offset..*offset + need)
        .ok_or_else(|| "truncated Obsidian f32 weights".to_string())?;
    *offset += need;
    Ok(slice
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero_net() -> ObsidianNetwork {
        let zeros = vec![0u8; FILE_SIZE as usize];
        ObsidianNetwork::from_bytes(&zeros).unwrap()
    }

    #[test]
    fn packed_file_size_is_stable() {
        assert_eq!(FILE_SIZE, 30_905_888);
    }

    #[test]
    fn zero_network_evaluates_startpos_to_zero() {
        types::init();
        let net = zero_net();
        let board = Board::new();
        assert_eq!(net.evaluate(&board), 0);
    }

    #[test]
    fn rejects_truncated_payload() {
        assert!(ObsidianNetwork::from_bytes(&[0u8; 16]).is_err());
    }

    #[test]
    fn detects_obsidian_filenames() {
        assert!(is_obsidian_path(Path::new("net89perm.bin")));
        assert!(is_obsidian_path(Path::new("obs_default.bin")));
        assert!(is_obsidian_path(Path::new("Obsidian-16.bin")));
        assert!(!is_obsidian_path(Path::new("ak_default.bin")));
    }
}
