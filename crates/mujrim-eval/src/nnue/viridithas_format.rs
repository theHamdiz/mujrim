//! Viridithas NNUE loader (zstd `.nnue.zst`) and a documented piece-feature
//! evaluator used when the decompressed payload matches a known layout.
//!
//! Latest threat-input / layered nets (velarised-2, sandhi, …) are rejected
//! with a clear error rather than silently evaluated through the wrong
//! architecture. The matching search profile still applies with the
//! implemented fallback evaluator.

use std::io::Read;
use std::path::Path;

use types::{Board, Color, Piece};

pub const KING_BUCKETS: usize = 16;
pub const FEATURES: usize = 768;
pub const HIDDEN: usize = 1024;
pub const QA: i32 = 255;
pub const SCALE: i32 = 400;
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

pub const FILE_SIZE: u64 = simple_size(HIDDEN) as u64;

pub const fn simple_size(hidden: usize) -> usize {
    KING_BUCKETS * FEATURES * hidden * 2 + hidden * 2 + 2 * hidden * 2 + 4
}

pub struct ViridithasNetwork {
    hidden: usize,
    features_per_bucket: usize,
    feature_weights: Box<[i16]>,
    feature_biases: Box<[i16]>,
    output_weights: Box<[i16]>,
    output_bias: i32,
}

impl ViridithasNetwork {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let decoded = maybe_decompress(bytes)?;
        for hidden in [HIDDEN, 768, 512, 256] {
            if decoded.len() == simple_size(hidden) {
                return parse_layout(&decoded, hidden, FEATURES);
            }
        }
        Err(format!(
            "Viridithas NNUE size {} is not a supported piece-feature layout (expected {} for H={HIDDEN}; threat-input / layered nets stay on the fallback evaluator)",
            decoded.len(),
            simple_size(HIDDEN)
        ))
    }

    #[inline(always)]
    pub fn features_per_bucket(&self) -> usize {
        self.features_per_bucket
    }

    #[inline(always)]
    pub fn hidden(&self) -> usize {
        self.hidden
    }

    #[inline(always)]
    pub fn evaluate(&self, board: &Board) -> i32 {
        match self.hidden {
            1024 => self.evaluate_hidden::<1024>(board),
            768 => self.evaluate_hidden::<768>(board),
            512 => self.evaluate_hidden::<512>(board),
            256 => self.evaluate_hidden::<256>(board),
            hidden => self.evaluate_capped(board, hidden),
        }
    }

    #[inline(always)]
    fn evaluate_hidden<const H: usize>(&self, board: &Board) -> i32 {
        let us = accumulate::<H>(self, board, board.side_to_move);
        let them = accumulate::<H>(self, board, board.side_to_move.opponent());
        finish_forward(self, &us, &them, H)
    }

    #[inline(always)]
    fn evaluate_capped(&self, board: &Board, hidden: usize) -> i32 {
        let us = accumulate::<HIDDEN>(self, board, board.side_to_move);
        let them = accumulate::<HIDDEN>(self, board, board.side_to_move.opponent());
        finish_forward(self, &us, &them, hidden)
    }
}

#[inline(always)]
fn finish_forward(net: &ViridithasNetwork, us: &[i16], them: &[i16], hidden: usize) -> i32 {
    let mut sum = net.output_bias;
    let weights = net.output_weights.as_ptr();
    unsafe {
        for i in 0..hidden {
            sum += screlu(*us.get_unchecked(i)) * i32::from(*weights.add(i));
            sum += screlu(*them.get_unchecked(i)) * i32::from(*weights.add(hidden + i));
        }
    }
    (sum / (QA * QA)) * SCALE / 64
}

#[inline(always)]
fn accumulate<const H: usize>(
    net: &ViridithasNetwork,
    board: &Board,
    perspective: Color,
) -> [i16; H] {
    let mut acc = [0i16; H];
    let hidden = net.hidden.min(H);
    acc[..hidden].copy_from_slice(&net.feature_biases[..hidden]);
    let king = relative_square(perspective, board.king_square(perspective).index());
    let bucket = king / 4;
    let stride = net.features_per_bucket;
    for piece in Piece::ALL {
        for color in [Color::White, Color::Black] {
            let mut bb = board.piece_bb(piece, color);
            while bb != 0 {
                let sq = bb.trailing_zeros() as usize;
                bb &= bb - 1;
                let rel_sq = relative_square(perspective, sq);
                let them = usize::from(color != perspective);
                let local = (them * 6 + piece.index()) * 64 + rel_sq;
                let index = bucket * stride + local;
                add_feature(&mut acc[..hidden], &net.feature_weights, index * hidden);
            }
        }
    }
    acc
}

#[inline(always)]
fn add_feature(acc: &mut [i16], weights: &[i16], base: usize) {
    let len = acc.len();
    debug_assert!(base + len <= weights.len());
    let acc_ptr = acc.as_mut_ptr();
    let weight_ptr = unsafe { weights.as_ptr().add(base) };
    for i in 0..len {
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
fn screlu(value: i16) -> i32 {
    let clipped = i32::from(value).clamp(0, QA);
    clipped * clipped
}

pub const fn wide_ft_size(hidden: usize, features_per_bucket: usize) -> usize {
    KING_BUCKETS * features_per_bucket * hidden * 2
}

pub const fn one_layer_head_size(hidden: usize) -> usize {
    hidden * 2 + hidden * 2 * 2 + 4
}

fn parse_layout(
    bytes: &[u8],
    hidden: usize,
    features_per_bucket: usize,
) -> Result<ViridithasNetwork, String> {
    let mut offset = 0;
    let ft = KING_BUCKETS * features_per_bucket * hidden;
    let feature_weights = read_i16s(bytes, &mut offset, ft)?;
    let feature_biases = read_i16s(bytes, &mut offset, hidden)?;
    let output_weights = read_i16s(bytes, &mut offset, hidden * 2)?;
    let output_bias = read_i32(bytes, &mut offset)?;
    Ok(ViridithasNetwork {
        hidden,
        features_per_bucket,
        feature_weights,
        feature_biases,
        output_weights,
        output_bias,
    })
}

fn maybe_decompress(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.starts_with(&ZSTD_MAGIC) {
        let mut decoder = zstd::stream::read::Decoder::new(bytes)
            .map_err(|error| format!("Viridithas zstd header: {error}"))?;
        let mut decoded = Vec::new();
        decoder
            .read_to_end(&mut decoded)
            .map_err(|error| format!("Viridithas zstd decode: {error}"))?;
        return Ok(decoded);
    }
    Ok(bytes.to_vec())
}

pub fn load(path: &Path) -> Result<Box<ViridithasNetwork>, String> {
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "failed to read Viridithas NNUE '{}': {error}",
            path.display()
        )
    })?;
    ViridithasNetwork::from_bytes(&bytes).map(Box::new)
}

pub fn looks_like_viridithas(path: &Path, bytes: &[u8]) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.contains("viri")
        || name.contains("velarised")
        || name.ends_with(".nnue.zst")
        || name.ends_with(".zst")
        || bytes.starts_with(&ZSTD_MAGIC)
}

fn read_i16s(bytes: &[u8], offset: &mut usize, count: usize) -> Result<Box<[i16]>, String> {
    let need = count * 2;
    let slice = bytes
        .get(*offset..*offset + need)
        .ok_or_else(|| "truncated Viridithas i16 weights".to_string())?;
    *offset += need;
    Ok(slice
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect())
}

fn read_i32(bytes: &[u8], offset: &mut usize) -> Result<i32, String> {
    let slice = bytes
        .get(*offset..*offset + 4)
        .ok_or_else(|| "truncated Viridithas bias".to_string())?;
    *offset += 4;
    Ok(i32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_layout_size_is_stable() {
        assert_eq!(simple_size(1024), 25_171_972);
        assert_eq!(simple_size(256), 6_292_996);
        assert_eq!(
            wide_ft_size(1024, 1770) + one_layer_head_size(1024),
            58_005_508
        );
    }

    #[test]
    fn zero_hidden256_network_evaluates_to_zero() {
        types::init();
        let bytes = vec![0u8; simple_size(256)];
        let net = ViridithasNetwork::from_bytes(&bytes).unwrap();
        assert_eq!(net.evaluate(&Board::new()), 0);
        assert_eq!(net.features_per_bucket(), FEATURES);
    }

    #[test]
    fn wide_or_layered_payload_is_rejected() {
        let features = 800;
        let hidden = 256;
        let bytes = vec![0u8; wide_ft_size(hidden, features) + one_layer_head_size(hidden) + 64];
        let error = match ViridithasNetwork::from_bytes(&bytes) {
            Ok(_) => panic!("wide payload must be rejected"),
            Err(error) => error,
        };
        assert!(
            error.contains("not a supported piece-feature layout"),
            "{error}"
        );
    }

    #[test]
    fn rejects_unknown_payload_size() {
        assert!(ViridithasNetwork::from_bytes(&[1, 2, 3, 4]).is_err());
    }

    #[test]
    fn downloaded_velarised_file_is_rejected_as_unsupported_layout() {
        let candidates = [
            Path::new("dist/nnue/viri_default.nnue.zst"),
            Path::new("nnue/viri_default.nnue.zst"),
        ];
        let Some(path) = candidates.iter().copied().find(|path| path.is_file()) else {
            return;
        };
        let error = match load(path) {
            Ok(_) => panic!("velarised-2 is a threat-input net"),
            Err(error) => error,
        };
        assert!(
            error.contains("not a supported piece-feature layout"),
            "{error}"
        );
    }

    #[test]
    fn detects_viridithas_names_and_magic() {
        assert!(looks_like_viridithas(
            Path::new("viri_default.nnue.zst"),
            &[]
        ));
        assert!(looks_like_viridithas(Path::new("net.bin"), &ZSTD_MAGIC));
        assert!(!looks_like_viridithas(
            Path::new("ak_default.bin"),
            &[0, 1, 2]
        ));
    }
}
