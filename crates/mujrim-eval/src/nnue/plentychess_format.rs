//! PlentyChess 0179r NNUE (PSQ + pawn-pair + threat FT → 1024 → 16 → 32 → 1 ×8).
//!
//! Published `0179r.bin` files are SLEB128/ULEB128-compressed `tmp` weights from
//! PlentyChess `process_net` (`infile_is_floats=false`). Evaluation uses that
//! pre-AVX-packus layout: L1/L2/L3 are transposed into `[bucket][in * out + out]`
//! for a refresh-from-scratch forward pass. Threat / pawn-pair indices reuse the
//! Stockfish 59808+4560 scheme already in this crate.

use std::path::Path;

use types::{Board, Color, Piece};

use super::stockfish_format::{
    PAIR_FEATURES, THREAT_FEATURES, visit_pawn_pair_features, visit_threat_features,
};

pub const L1: usize = 1024;
pub const L2: usize = 16;
pub const L3: usize = 32;
pub const KING_BUCKETS: usize = 12;
pub const OUTPUT_BUCKETS: usize = 8;
pub const FEATURES: usize = 768;
pub const NETWORK_SCALE: i32 = 287;
pub const NETWORK_QA: i32 = 255;
pub const NETWORK_QB: i32 = 64;
const FT_SHIFT: i32 = 9;
const L1_NORMALISATION: f32 =
    ((1 << FT_SHIFT) as f32) / ((NETWORK_QA * NETWORK_QA * NETWORK_QB) as f32);

#[rustfmt::skip]
const KING_BUCKET_LAYOUT: [usize; 64] = [
    0, 1, 2, 3, 3, 2, 1, 0,
    4, 5, 6, 7, 7, 6, 5, 4,
    8, 8, 9, 9, 9, 9, 8, 8,
    10, 10, 10, 10, 10, 10, 10, 10,
    11, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 11, 11,
];

pub struct PlentyChessNetwork {
    psq_weights: Box<[i16]>,
    pawn_pair_weights: Box<[i8]>,
    threat_weights: Box<[i8]>,
    feature_biases: Box<[i16]>,
    l1_weights: Box<[i8]>,
    l1_biases: Box<[f32]>,
    l2_weights: Box<[f32]>,
    l2_biases: Box<[f32]>,
    l3_weights: Box<[f32]>,
    l3_biases: Box<[f32]>,
}

impl PlentyChessNetwork {
    pub fn from_compressed_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut pos = 0;
        let psq_weights = read_sleb_i16s(bytes, &mut pos, FEATURES * KING_BUCKETS * L1)?;
        let pawn_tmp = read_sleb_i8s(bytes, &mut pos, PAIR_FEATURES * L1)?;
        let threat_tmp = read_sleb_i8s(bytes, &mut pos, THREAT_FEATURES * L1)?;
        let feature_biases = read_sleb_i16s(bytes, &mut pos, L1)?;
        let l1_tmp = read_sleb_i8s(bytes, &mut pos, OUTPUT_BUCKETS * L1 * L2)?;
        let l1_biases = read_uleb_f32s(bytes, &mut pos, OUTPUT_BUCKETS * L2)?;
        let l2_tmp = read_uleb_f32s(bytes, &mut pos, OUTPUT_BUCKETS * (L2 * 2) * L3)?;
        let l2_biases = read_uleb_f32s(bytes, &mut pos, OUTPUT_BUCKETS * L3)?;
        let l3_tmp = read_uleb_f32s(bytes, &mut pos, OUTPUT_BUCKETS * (L3 + 2 * L2))?;
        let l3_biases = read_uleb_f32s(bytes, &mut pos, OUTPUT_BUCKETS)?;
        if pos != bytes.len() {
            return Err(format!(
                "PlentyChess NNUE leftover bytes: decoded {pos}, file {}",
                bytes.len()
            ));
        }

        let mut l1_weights = vec![0i8; OUTPUT_BUCKETS * L1 * L2].into_boxed_slice();
        for bucket in 0..OUTPUT_BUCKETS {
            for l1 in 0..L1 {
                for l2 in 0..L2 {
                    l1_weights[bucket * L1 * L2 + l1 * L2 + l2] =
                        l1_tmp[l1 * OUTPUT_BUCKETS * L2 + bucket * L2 + l2];
                }
            }
        }

        let mut l2_weights = vec![0f32; OUTPUT_BUCKETS * (L2 * 2) * L3].into_boxed_slice();
        for bucket in 0..OUTPUT_BUCKETS {
            for l2 in 0..(L2 * 2) {
                for l3 in 0..L3 {
                    l2_weights[bucket * (L2 * 2) * L3 + l2 * L3 + l3] =
                        l2_tmp[l2 * OUTPUT_BUCKETS * L3 + bucket * L3 + l3];
                }
            }
        }

        let mut l3_weights = vec![0f32; OUTPUT_BUCKETS * (L3 + 2 * L2)].into_boxed_slice();
        for bucket in 0..OUTPUT_BUCKETS {
            for l3 in 0..(L3 + 2 * L2) {
                l3_weights[bucket * (L3 + 2 * L2) + l3] = l3_tmp[l3 * OUTPUT_BUCKETS + bucket];
            }
        }

        Ok(Self {
            psq_weights,
            pawn_pair_weights: pawn_tmp,
            threat_weights: threat_tmp,
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
        refresh_perspective(self, board, Color::White, &mut acc_white);
        refresh_perspective(self, board, Color::Black, &mut acc_black);

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

fn refresh_perspective(net: &PlentyChessNetwork, board: &Board, pov: Color, acc: &mut [i16; L1]) {
    let king = board.king_square(pov).index();
    let bucket = KING_BUCKET_LAYOUT[king ^ (56 * pov.index())];
    let mirror = king & 7 >= 4;
    for piece in Piece::ALL {
        for piece_color in [Color::White, Color::Black] {
            let mut bb = board.piece_bb(piece, piece_color);
            while bb != 0 {
                let sq = bb.trailing_zeros() as usize;
                bb &= bb - 1;
                let oriented = sq ^ (7 * usize::from(mirror)) ^ (56 * pov.index());
                let them = usize::from(piece_color != pov);
                let feature = bucket * FEATURES + them * 384 + piece.index() * 64 + oriented;
                add_i16_row(acc, &net.psq_weights, feature);
            }
        }
    }

    let perspective = pov.index();
    visit_pawn_pair_features(board, perspective, |feature| {
        add_i8_row(acc, &net.pawn_pair_weights, feature - THREAT_FEATURES);
    });
    visit_threat_features(board, perspective, |feature| {
        add_i8_row(acc, &net.threat_weights, feature);
    });
}

#[inline(always)]
fn add_i16_row(acc: &mut [i16; L1], weights: &[i16], feature: usize) {
    let base = feature * L1;
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
fn add_i8_row(acc: &mut [i16; L1], weights: &[i8], feature: usize) {
    let base = feature * L1;
    debug_assert!(base + L1 <= weights.len());
    let acc_ptr = acc.as_mut_ptr();
    let weight_ptr = unsafe { weights.as_ptr().add(base) };
    for i in 0..L1 {
        unsafe {
            *acc_ptr.add(i) = (*acc_ptr.add(i)).wrapping_add(i16::from(*weight_ptr.add(i)));
        }
    }
}

#[inline(always)]
fn propagate(net: &PlentyChessNetwork, us: &[i16; L1], them: &[i16; L1], bucket: usize) -> i32 {
    let mut ft_out = [0u8; L1];
    activate_ft(us, &mut ft_out[..L1 / 2]);
    activate_ft(them, &mut ft_out[L1 / 2..]);

    let mut l1_sum = [0i32; L2];
    let l1_weight_base = bucket * L1 * L2;
    for (i, feature) in ft_out.iter().enumerate() {
        if *feature == 0 {
            continue;
        }
        let row = l1_weight_base + i * L2;
        let value = i32::from(*feature);
        for (j, sum) in l1_sum.iter_mut().enumerate() {
            *sum += value * i32::from(net.l1_weights[row + j]);
        }
    }

    let mut l1 = [0.0f32; L2 * 2];
    let l1_bias_base = bucket * L2;
    for j in 0..L2 {
        let biased = l1_sum[j] as f32 * L1_NORMALISATION + net.l1_biases[l1_bias_base + j];
        l1[j] = biased.clamp(0.0, 1.0);
        l1[j + L2] = (biased * biased).clamp(0.0, 1.0);
    }

    let mut l2 = [0.0f32; L3];
    let l2_weight_base = bucket * (L2 * 2) * L3;
    let l2_bias_base = bucket * L3;
    l2.copy_from_slice(&net.l2_biases[l2_bias_base..l2_bias_base + L3]);
    for (i, feature) in l1.iter().enumerate() {
        let row = l2_weight_base + i * L3;
        for (j, value) in l2.iter_mut().enumerate() {
            *value += net.l2_weights[row + j] * *feature;
        }
    }
    for value in &mut l2 {
        let activated = value.clamp(0.0, 1.0);
        *value = activated * activated;
    }

    let l3_base = bucket * (L3 + 2 * L2);
    let mut result = net.l3_biases[bucket];
    for (j, feature) in l2.iter().enumerate() {
        result += net.l3_weights[l3_base + j] * *feature;
    }
    for (j, feature) in l1.iter().enumerate() {
        result += net.l3_weights[l3_base + L3 + j] * *feature;
    }
    (result * NETWORK_SCALE as f32) as i32
}

#[inline(always)]
fn activate_ft(acc: &[i16], out: &mut [u8]) {
    let half = L1 / 2;
    debug_assert_eq!(out.len(), half);
    for i in 0..half {
        let c0 = i32::from(acc[i]).clamp(0, NETWORK_QA);
        let c1 = i32::from(acc[i + half]).min(NETWORK_QA);
        let shifted = c0 << (16 - FT_SHIFT);
        let prod = ((shifted * c1) >> 16).clamp(0, 255);
        out[i] = prod as u8;
    }
}

pub fn is_plentychess_path(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.contains("plenty") || name.contains("0179") || name.contains("plenty_default")
}

fn read_sleb128(bytes: &[u8], pos: &mut usize) -> Result<i64, String> {
    let mut result = 0i64;
    let mut shift = 0;
    loop {
        let byte = *bytes
            .get(*pos)
            .ok_or_else(|| "truncated PlentyChess SLEB128".to_string())?;
        *pos += 1;
        result |= i64::from(byte & 0x7F) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            if shift < 64 && byte & 0x40 != 0 {
                result |= -1i64 << shift;
            }
            return Ok(result);
        }
        if shift >= 64 {
            return Err("PlentyChess SLEB128 overflow".to_string());
        }
    }
}

fn read_uleb128(bytes: &[u8], pos: &mut usize) -> Result<u64, String> {
    let mut result = 0u64;
    let mut shift = 0;
    loop {
        let byte = *bytes
            .get(*pos)
            .ok_or_else(|| "truncated PlentyChess ULEB128".to_string())?;
        *pos += 1;
        result |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift >= 64 {
            return Err("PlentyChess ULEB128 overflow".to_string());
        }
    }
}

fn read_sleb_i16s(bytes: &[u8], pos: &mut usize, count: usize) -> Result<Box<[i16]>, String> {
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(
            i16::try_from(read_sleb128(bytes, pos)?)
                .map_err(|_| "PlentyChess i16 weight out of range".to_string())?,
        );
    }
    Ok(values.into_boxed_slice())
}

fn read_sleb_i8s(bytes: &[u8], pos: &mut usize, count: usize) -> Result<Box<[i8]>, String> {
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(
            i8::try_from(read_sleb128(bytes, pos)?)
                .map_err(|_| "PlentyChess i8 weight out of range".to_string())?,
        );
    }
    Ok(values.into_boxed_slice())
}

fn read_uleb_f32s(bytes: &[u8], pos: &mut usize, count: usize) -> Result<Box<[f32]>, String> {
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let bits = u32::try_from(read_uleb128(bytes, pos)?)
            .map_err(|_| "PlentyChess f32 payload out of range".to_string())?;
        values.push(f32::from_bits(bits));
    }
    Ok(values.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_plentychess_filenames() {
        assert!(is_plentychess_path(Path::new("plenty_default.bin")));
        assert!(is_plentychess_path(Path::new("0179r.bin")));
        assert!(is_plentychess_path(Path::new("PlentyChess-0179.bin")));
        assert!(!is_plentychess_path(Path::new("obs_default.bin")));
        assert!(!is_plentychess_path(Path::new("ak_default.bin")));
    }

    #[test]
    fn sleb128_roundtrip_values() {
        // 0, -1, 127, -128 encoded the same way process_net writes them.
        let encoded = [0x00, 0x7F, 0xFF, 0x00, 0x80, 0x7F];
        let mut pos = 0;
        assert_eq!(read_sleb128(&encoded, &mut pos).unwrap(), 0);
        assert_eq!(read_sleb128(&encoded, &mut pos).unwrap(), -1);
        assert_eq!(read_sleb128(&encoded, &mut pos).unwrap(), 127);
        assert_eq!(read_sleb128(&encoded, &mut pos).unwrap(), -128);
        assert_eq!(pos, encoded.len());
    }

    #[test]
    fn rejects_truncated_payload() {
        assert!(PlentyChessNetwork::from_compressed_bytes(&[0u8; 16]).is_err());
    }

    #[test]
    fn king_bucket_layout_has_twelve_ids() {
        let mut seen = [false; KING_BUCKETS];
        for bucket in KING_BUCKET_LAYOUT {
            seen[bucket] = true;
        }
        assert!(seen.iter().all(|used| *used));
        assert!(KING_BUCKET_LAYOUT[32..].iter().all(|bucket| *bucket == 11));
    }
}
