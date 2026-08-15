//! Lc0 v3–v6 training chunks → `FEN|score|wdl`.
//!
//! Layout matches the public Lc0 training-data wiki. Planes are STM-relative;
//! on-disk bitboards are stored with bits reversed inside each byte.

use std::io::{self, Read};

use types::{Board, Color, Piece, Square};

use crate::datagen::TrainingPosition;

pub const V3_SIZE: usize = 8276;
pub const V4_SIZE: usize = 8292;
pub const V5_SIZE: usize = 8308;
pub const V6_SIZE: usize = 8356;

const FLIP: u8 = 1;
const MIRROR: u8 = 2;
const TRANSPOSE: u8 = 4;

pub fn record_size(version: u32) -> Option<usize> {
    match version {
        3 => Some(V3_SIZE),
        4 => Some(V4_SIZE),
        5 => Some(V5_SIZE),
        6 => Some(V6_SIZE),
        _ => None,
    }
}

pub fn looks_like_lc0(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    let version = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    record_size(version).is_some_and(|size| bytes.len() >= size && bytes.len().is_multiple_of(size))
}

pub fn decode_records(bytes: &[u8]) -> Result<Vec<TrainingPosition>, String> {
    if bytes.len() < 4 {
        return Err("Lc0 chunk is truncated".into());
    }
    let version = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    let size = record_size(version).ok_or_else(|| format!("unsupported Lc0 version {version}"))?;
    if !bytes.len().is_multiple_of(size) {
        return Err(format!(
            "Lc0 v{version} chunk length {} is not a multiple of {size}",
            bytes.len()
        ));
    }
    types::init();
    let mut positions = Vec::with_capacity(bytes.len() / size);
    for record in bytes.chunks_exact(size) {
        if let Some(position) = decode_record(record)? {
            positions.push(position);
        }
    }
    Ok(positions)
}

pub fn decode_stream(
    first4: [u8; 4],
    mut reader: impl Read,
) -> Result<Vec<TrainingPosition>, String> {
    let version = u32::from_le_bytes(first4);
    let size = record_size(version).ok_or_else(|| format!("unsupported Lc0 version {version}"))?;
    types::init();
    let mut record = vec![0u8; size];
    record[..4].copy_from_slice(&first4);
    reader
        .read_exact(&mut record[4..])
        .map_err(|error| format!("Lc0 record truncated: {error}"))?;
    let mut positions = Vec::new();
    if let Some(position) = decode_record(&record)? {
        positions.push(position);
    }
    loop {
        match reader.read_exact(&mut record) {
            Ok(()) => {
                if let Some(position) = decode_record(&record)? {
                    positions.push(position);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(format!("Lc0 stream failed: {error}")),
        }
    }
    Ok(positions)
}

fn decode_record(record: &[u8]) -> Result<Option<TrainingPosition>, String> {
    let version = u32::from_le_bytes(record[0..4].try_into().unwrap());
    let (input_format, planes_at, meta_at) = match version {
        3 | 4 => (1u32, 4 + 1858 * 4, 4 + 1858 * 4 + 104 * 8),
        5 | 6 => (
            u32::from_le_bytes(record[4..8].try_into().unwrap()),
            8 + 1858 * 4,
            8 + 1858 * 4 + 104 * 8,
        ),
        other => return Err(format!("unsupported Lc0 version {other}")),
    };
    if record.len() < meta_at + 8 {
        return Err("Lc0 record is truncated".into());
    }
    let us_ooo = record[meta_at] != 0;
    let us_oo = record[meta_at + 1] != 0;
    let them_ooo = record[meta_at + 2] != 0;
    let them_oo = record[meta_at + 3] != 0;
    let stm_or_ep = record[meta_at + 4];
    let rule50 = record[meta_at + 5];
    let invariance = record[meta_at + 6];
    if version >= 6 && invariance & (1 << 6) != 0 {
        return Ok(None);
    }
    let black = if input_format >= 3 {
        invariance & (1 << 7) != 0
    } else {
        stm_or_ep != 0
    };
    let transform = if input_format >= 3 {
        invariance & (FLIP | MIRROR | TRANSPOSE)
    } else {
        0
    };
    let (score, wdl) = targets(version, record, meta_at)?;
    let fen = fen_from_planes(
        &record[planes_at..planes_at + 104 * 8],
        PlaneMeta {
            black,
            transform,
            us_oo,
            us_ooo,
            them_oo,
            them_ooo,
            rule50,
        },
    )?;
    Ok(Some(TrainingPosition { fen, score, wdl }))
}

fn targets(version: u32, record: &[u8], meta_at: usize) -> Result<(i32, f32), String> {
    match version {
        3 => {
            let result = record[meta_at + 7] as i8;
            Ok((
                i32::from(result) * 200,
                match result {
                    1 => 1.0,
                    -1 => 0.0,
                    _ => 0.5,
                },
            ))
        }
        4 => {
            let best_q = read_f32(record, meta_at + 12)?;
            let result = record[meta_at + 7] as i8;
            Ok((
                q_to_cp(best_q),
                match result {
                    1 => 1.0,
                    -1 => 0.0,
                    _ => 0.5,
                },
            ))
        }
        5 => {
            let best_q = read_f32(record, meta_at + 12)?;
            let result = record[meta_at + 7] as i8;
            Ok((
                q_to_cp(best_q),
                match result {
                    1 => 1.0,
                    -1 => 0.0,
                    _ => 0.5,
                },
            ))
        }
        6 => {
            let best_q = read_f32(record, meta_at + 12)?;
            let result_q = read_f32(record, meta_at + 36)?;
            Ok((q_to_cp(best_q), ((result_q + 1.0) * 0.5).clamp(0.0, 1.0)))
        }
        other => Err(format!("unsupported Lc0 version {other}")),
    }
}

struct PlaneMeta {
    black: bool,
    transform: u8,
    us_oo: bool,
    us_ooo: bool,
    them_oo: bool,
    them_ooo: bool,
    rule50: u8,
}

fn fen_from_planes(plane_bytes: &[u8], meta: PlaneMeta) -> Result<String, String> {
    let mut board = Board::empty();
    board.side_to_move = if meta.black {
        Color::Black
    } else {
        Color::White
    };
    board.halfmove_clock = u32::from(meta.rule50);
    board.fullmove_number = 1;
    let us = board.side_to_move;
    let them = if meta.black {
        Color::White
    } else {
        Color::Black
    };
    for (index, piece) in Piece::ALL.into_iter().enumerate() {
        place_plane(
            &mut board,
            read_plane(plane_bytes, index),
            piece,
            us,
            meta.black,
            meta.transform,
        )?;
        place_plane(
            &mut board,
            read_plane(plane_bytes, index + 6),
            piece,
            them,
            meta.black,
            meta.transform,
        )?;
    }
    if board.piece_bb(Piece::King, Color::White) == 0
        || board.piece_bb(Piece::King, Color::Black) == 0
    {
        return Err("Lc0 record is missing a king".into());
    }
    let mut rights = 0u8;
    if meta.us_oo {
        rights |= if meta.black {
            types::board::BLACK_KING_CASTLE
        } else {
            types::board::WHITE_KING_CASTLE
        };
    }
    if meta.us_ooo {
        rights |= if meta.black {
            types::board::BLACK_QUEEN_CASTLE
        } else {
            types::board::WHITE_QUEEN_CASTLE
        };
    }
    if meta.them_oo {
        rights |= if meta.black {
            types::board::WHITE_KING_CASTLE
        } else {
            types::board::BLACK_KING_CASTLE
        };
    }
    if meta.them_ooo {
        rights |= if meta.black {
            types::board::WHITE_QUEEN_CASTLE
        } else {
            types::board::BLACK_QUEEN_CASTLE
        };
    }
    board.castling_rights = rights;
    Ok(board.to_fen())
}

fn place_plane(
    board: &mut Board,
    plane: u64,
    piece: Piece,
    color: Color,
    black: bool,
    transform: u8,
) -> Result<(), String> {
    let mut bits = plane;
    while bits != 0 {
        let plane_sq = bits.trailing_zeros() as usize;
        bits &= bits - 1;
        let mut sq = plane_sq;
        if black {
            sq ^= 56;
        }
        sq = inverse_transform(sq, transform);
        board.put_piece(piece, color, Square::from_index(sq));
    }
    Ok(())
}

fn read_plane(plane_bytes: &[u8], index: usize) -> u64 {
    let start = index * 8;
    let raw = u64::from_le_bytes(plane_bytes[start..start + 8].try_into().unwrap());
    reverse_bits_in_bytes(raw)
}

fn inverse_transform(mut sq: usize, transform: u8) -> usize {
    if transform & TRANSPOSE != 0 {
        sq = (sq % 8) * 8 + sq / 8;
    }
    if transform & MIRROR != 0 {
        sq ^= 7;
    }
    if transform & FLIP != 0 {
        sq ^= 56;
    }
    sq
}

fn reverse_bits_in_bytes(mut value: u64) -> u64 {
    value = ((value >> 1) & 0x5555_5555_5555_5555) | ((value & 0x5555_5555_5555_5555) << 1);
    value = ((value >> 2) & 0x3333_3333_3333_3333) | ((value & 0x3333_3333_3333_3333) << 2);
    value = ((value >> 4) & 0x0f0f_0f0f_0f0f_0f0f) | ((value & 0x0f0f_0f0f_0f0f_0f0f) << 4);
    value
}

fn read_f32(record: &[u8], offset: usize) -> Result<f32, String> {
    let bytes: [u8; 4] = record
        .get(offset..offset + 4)
        .ok_or("Lc0 float field is truncated")?
        .try_into()
        .unwrap();
    Ok(f32::from_le_bytes(bytes))
}

fn q_to_cp(q: f32) -> i32 {
    if !q.is_finite() {
        return 0;
    }
    let q = q.clamp(-0.999, 0.999);
    (90.0 * (1.563_754_2 * q).tan()).round() as i32
}

#[cfg(test)]
pub(crate) fn encode_v6_startpos(best_q: f32, result_q: f32) -> Vec<u8> {
    let mut record = vec![0u8; V6_SIZE];
    record[0..4].copy_from_slice(&6u32.to_le_bytes());
    record[4..8].copy_from_slice(&1u32.to_le_bytes());
    let planes_at = 8 + 1858 * 4;
    let planes = startpos_planes();
    for (index, plane) in planes.into_iter().enumerate() {
        let start = planes_at + index * 8;
        record[start..start + 8].copy_from_slice(&reverse_bits_in_bytes(plane).to_le_bytes());
    }
    let meta = planes_at + 104 * 8;
    record[meta..meta + 4].copy_from_slice(&[1, 1, 1, 1]);
    record[meta + 12..meta + 16].copy_from_slice(&best_q.to_le_bytes());
    record[meta + 36..meta + 40].copy_from_slice(&result_q.to_le_bytes());
    record
}

#[cfg(test)]
fn startpos_planes() -> [u64; 13] {
    [
        0x0000_0000_0000_ff00,
        (1 << 1) | (1 << 6),
        (1 << 2) | (1 << 5),
        (1 << 0) | (1 << 7),
        1 << 3,
        1 << 4,
        0x00ff_0000_0000_0000,
        (1 << 57) | (1 << 62),
        (1 << 58) | (1 << 61),
        (1 << 56) | (1 << 63),
        1 << 59,
        1 << 60,
        0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v6_startpos_roundtrips_fen_score_and_wdl() {
        let record = encode_v6_startpos(0.0, 1.0);
        assert!(looks_like_lc0(&record));
        let positions = decode_records(&record).expect("lc0");
        assert_eq!(positions.len(), 1);
        assert_eq!(
            positions[0].fen,
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        );
        assert_eq!(positions[0].score, 0);
        assert_eq!(positions[0].wdl, 1.0);
    }

    #[test]
    fn stream_reads_two_v6_records() {
        let mut bytes = encode_v6_startpos(0.0, 0.0);
        bytes.extend_from_slice(&encode_v6_startpos(0.0, -1.0));
        let first4: [u8; 4] = bytes[..4].try_into().unwrap();
        let positions = decode_stream(first4, &bytes[4..]).expect("stream");
        assert_eq!(positions.len(), 2);
        assert_eq!(positions[1].wdl, 0.0);
    }

    #[test]
    fn deleted_v6_records_are_skipped() {
        let mut record = encode_v6_startpos(0.0, 0.0);
        let meta = 8 + 1858 * 4 + 104 * 8;
        record[meta + 6] |= 1 << 6;
        assert!(decode_records(&record).unwrap().is_empty());
    }
}
