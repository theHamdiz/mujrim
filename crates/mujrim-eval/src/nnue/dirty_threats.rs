use types::board::attack_tables::{
    bishop_attacks, king_attacks, knight_attacks, pawn_attacks, queen_attacks, rook_attacks,
};
use types::chess_move::MoveFlag;
use types::{Board, Color, Move, Piece, Square};

pub(super) const MAX_DIRTY_THREAT_DELTAS: usize = 96;

#[derive(Copy, Clone, Default)]
pub(super) struct ThreatDelta(u32);

impl ThreatDelta {
    #[inline(always)]
    pub(super) const fn new(
        attacker: u8,
        source: usize,
        attacked: u8,
        target: usize,
        add: bool,
    ) -> Self {
        Self(
            u32::from_le_bytes([attacker, source as u8, attacked, target as u8])
                | ((add as u32) << 31),
        )
    }

    #[inline(always)]
    pub(super) const fn attacker(self) -> usize {
        (self.0 & 0xff) as usize
    }

    #[inline(always)]
    pub(super) const fn source(self) -> usize {
        ((self.0 >> 8) & 0xff) as usize
    }

    #[inline(always)]
    pub(super) const fn attacked(self) -> usize {
        ((self.0 >> 16) & 0xff) as usize
    }

    #[inline(always)]
    pub(super) const fn target(self) -> usize {
        ((self.0 >> 24) & 0x7f) as usize
    }

    #[inline(always)]
    pub(super) const fn add(self) -> bool {
        self.0 >> 31 != 0
    }
}

pub(super) trait ThreatDeltaSink {
    fn push_threat_delta(&mut self, delta: ThreatDelta);
}

#[derive(Clone, Copy)]
struct ThreatPosition {
    pieces: [u64; 12],
    piece_at: [u8; 64],
    occupancy: u64,
}

#[derive(Clone, Copy)]
pub(super) struct ThreatSnapshot {
    position: ThreatPosition,
    color: usize,
}

impl ThreatSnapshot {
    #[inline(always)]
    pub(super) fn from_board(board: &Board) -> Self {
        Self {
            position: ThreatPosition::from_board(board),
            color: board.side_to_move.index(),
        }
    }

    #[inline(always)]
    pub(super) fn mailbox(self) -> [u8; 64] {
        self.position.piece_at
    }

    #[inline(always)]
    pub(super) const fn color(self) -> usize {
        self.color
    }
}

pub(super) trait ThreatView {
    fn occupancy(&self) -> u64;
    fn piece_id(&self, square: usize) -> Option<u8>;
    fn pieces_of(&self, piece: usize) -> u64;
    fn pieces_for(&self, color: usize, piece: usize) -> u64;
}

impl ThreatPosition {
    fn from_board(board: &Board) -> Self {
        let mut pieces = [0; 12];
        for color in 0..2 {
            for piece in 0..Piece::COUNT {
                pieces[color * Piece::COUNT + piece] = board.pieces[color][piece];
            }
        }
        Self {
            pieces,
            occupancy: board.all_occupancy(),
            piece_at: *board.piece_ids(),
        }
    }

    #[inline(always)]
    fn remove(&mut self, id: u8, square: usize) {
        let piece = usize::from(id) / 2;
        let color = usize::from(id) & 1;
        self.pieces[color * Piece::COUNT + piece] &= !(1u64 << square);
        self.piece_at[square] = u8::MAX;
        self.occupancy &= !(1u64 << square);
    }

    #[inline(always)]
    fn add(&mut self, id: u8, square: usize) {
        let piece = usize::from(id) / 2;
        let color = usize::from(id) & 1;
        self.pieces[color * Piece::COUNT + piece] |= 1u64 << square;
        self.piece_at[square] = id;
        self.occupancy |= 1u64 << square;
    }
}

impl ThreatView for ThreatPosition {
    #[inline(always)]
    fn occupancy(&self) -> u64 {
        self.occupancy
    }

    #[inline(always)]
    fn piece_id(&self, square: usize) -> Option<u8> {
        (self.piece_at[square] != u8::MAX).then_some(self.piece_at[square])
    }

    #[inline(always)]
    fn pieces_of(&self, piece: usize) -> u64 {
        self.pieces[piece] | self.pieces[Piece::COUNT + piece]
    }

    #[inline(always)]
    fn pieces_for(&self, color: usize, piece: usize) -> u64 {
        self.pieces[color * Piece::COUNT + piece]
    }
}

impl ThreatView for Board {
    #[inline(always)]
    fn occupancy(&self) -> u64 {
        self.all_occupancy()
    }

    #[inline(always)]
    fn piece_id(&self, square: usize) -> Option<u8> {
        let id = self.piece_ids()[square];
        (id != u8::MAX).then_some(id)
    }

    #[inline(always)]
    fn pieces_of(&self, piece: usize) -> u64 {
        self.pieces[0][piece] | self.pieces[1][piece]
    }

    #[inline(always)]
    fn pieces_for(&self, color: usize, piece: usize) -> u64 {
        self.pieces[color][piece]
    }
}

#[cfg(feature = "reckless-nnue")]
pub(super) fn collect_move_deltas(sink: &mut impl ThreatDeltaSink, board: &Board, mv: Move) {
    collect_snapshot_move_deltas(sink, ThreatSnapshot::from_board(board), mv);
}

pub(super) fn collect_snapshot_move_deltas(
    sink: &mut impl ThreatDeltaSink,
    snapshot: ThreatSnapshot,
    mv: Move,
) {
    let mut position = snapshot.position;
    let color = snapshot.color;
    let from = mv.from.index();
    let to = mv.to.index();
    let mover = position
        .piece_id(from)
        .expect("legal move has a source piece");
    debug_assert_eq!(usize::from(mover) & 1, color);

    if mv.is_capture() && mv.flag != MoveFlag::EnPassant {
        let captured = position
            .piece_id(to)
            .expect("ordinary capture has a target piece");
        position.remove(mover, from);
        push_threats_on_change(sink, &position, mover, from, false);
        position.remove(captured, to);
        position.add(mover, to);
        push_threats_on_mutate(sink, &position, captured, mover, to);
    } else if !mv.is_castling() {
        position.remove(mover, from);
        position.add(mover, to);
        push_threats_on_move(sink, &position, mover, from, to);
    }

    match mv.flag {
        MoveFlag::EnPassant => {
            let captured_square = Square::from_file_rank(mv.to.file(), mv.from.rank()).index();
            let captured = (Piece::Pawn.index() * 2 + (color ^ 1)) as u8;
            position.remove(captured, captured_square);
            push_threats_on_change(sink, &position, captured, captured_square, false);
        }
        MoveFlag::KingCastle | MoveFlag::QueenCastle => {
            let (rook_from, rook_to) = match (color, mv.flag) {
                (0, MoveFlag::KingCastle) => (Square::H1.index(), Square::F1.index()),
                (0, MoveFlag::QueenCastle) => (Square::A1.index(), Square::D1.index()),
                (1, MoveFlag::KingCastle) => (Square::H8.index(), Square::F8.index()),
                (1, MoveFlag::QueenCastle) => (Square::A8.index(), Square::D8.index()),
                _ => unreachable!(),
            };
            let rook = (Piece::Rook.index() * 2 + color) as u8;
            position.remove(rook, rook_from);
            push_threats_on_change(sink, &position, rook, rook_from, false);
            position.remove(mover, from);
            push_threats_on_change(sink, &position, mover, from, false);
            position.add(rook, rook_to);
            push_threats_on_change(sink, &position, rook, rook_to, true);
            position.add(mover, to);
            push_threats_on_change(sink, &position, mover, to, true);
        }
        MoveFlag::Promotion | MoveFlag::PromotionCapture => {
            let promoted =
                (mv.promotion.expect("promotion move has a piece").index() * 2 + color) as u8;
            position.remove(mover, to);
            push_threats_on_change(sink, &position, mover, to, false);
            position.add(promoted, to);
            push_threats_on_change(sink, &position, promoted, to, true);
        }
        _ => {}
    }
}

pub(super) fn push_threats_on_move(
    sink: &mut impl ThreatDeltaSink,
    position: &impl ThreatView,
    piece: u8,
    from: usize,
    to: usize,
) {
    let occupancy_without_destination = position.occupancy() ^ (1u64 << to);
    push_threats_single(
        sink,
        position,
        occupancy_without_destination,
        piece,
        from,
        false,
    );
    push_threats_single(
        sink,
        position,
        occupancy_without_destination,
        piece,
        to,
        true,
    );
}

pub(super) fn push_threats_on_change(
    sink: &mut impl ThreatDeltaSink,
    position: &impl ThreatView,
    piece: u8,
    square: usize,
    add: bool,
) {
    push_threats_single(sink, position, position.occupancy(), piece, square, add);
}

fn push_threats_single(
    sink: &mut impl ThreatDeltaSink,
    position: &impl ThreatView,
    occupancy: u64,
    piece: u8,
    square: usize,
    add: bool,
) {
    let piece_index = usize::from(piece) / 2;
    let color = usize::from(piece) & 1;
    if piece_index == Piece::King.index() {
        return;
    }
    let kings = position.pieces_of(Piece::King.index());
    let mut attacked = piece_attacks(piece_index, color, square, occupancy) & occupancy & !kings;
    while attacked != 0 {
        let target = attacked.trailing_zeros() as usize;
        attacked &= attacked - 1;
        if let Some(target_piece) = position.piece_id(target) {
            sink.push_threat_delta(ThreatDelta::new(piece, square, target_piece, target, add));
        }
    }

    let diagonal = (position.pieces_of(Piece::Bishop.index())
        | position.pieces_of(Piece::Queen.index()))
        & bishop_attacks(square, occupancy);
    let orthogonal = (position.pieces_of(Piece::Rook.index())
        | position.pieces_of(Piece::Queen.index()))
        & rook_attacks(square, occupancy);
    let mut sliders = (diagonal | orthogonal) & occupancy;
    while sliders != 0 {
        let source = sliders.trailing_zeros() as usize;
        sliders &= sliders - 1;
        let slider = position
            .piece_id(source)
            .expect("visible slider is occupied");
        if let Some(target) = next_occupied_beyond(source, square, occupancy)
            && let Some(target_piece) = position.piece_id(target)
        {
            sink.push_threat_delta(ThreatDelta::new(slider, source, target_piece, target, !add));
        }
        sink.push_threat_delta(ThreatDelta::new(slider, source, piece, square, add));
    }

    let black_pawns = position.pieces_for(Color::Black.index(), Piece::Pawn.index())
        & pawn_attacks(Color::White.index(), square);
    let white_pawns = position.pieces_for(Color::White.index(), Piece::Pawn.index())
        & pawn_attacks(Color::Black.index(), square);
    let knights = position.pieces_of(Piece::Knight.index()) & knight_attacks(square);
    let mut attackers = (black_pawns | white_pawns | knights) & occupancy;
    while attackers != 0 {
        let source = attackers.trailing_zeros() as usize;
        attackers &= attackers - 1;
        if let Some(attacker) = position.piece_id(source) {
            sink.push_threat_delta(ThreatDelta::new(attacker, source, piece, square, add));
        }
    }
}

pub(super) fn push_threats_on_mutate(
    sink: &mut impl ThreatDeltaSink,
    position: &impl ThreatView,
    old_piece: u8,
    new_piece: u8,
    square: usize,
) {
    let occupancy = position.occupancy();
    for (piece, add) in [(old_piece, false), (new_piece, true)] {
        let mut attacked = piece_attacks(
            usize::from(piece) / 2,
            usize::from(piece) & 1,
            square,
            occupancy,
        ) & occupancy;
        while attacked != 0 {
            let target = attacked.trailing_zeros() as usize;
            attacked &= attacked - 1;
            if let Some(target_piece) = position.piece_id(target) {
                sink.push_threat_delta(ThreatDelta::new(piece, square, target_piece, target, add));
            }
        }
    }

    let diagonal = (position.pieces_of(Piece::Bishop.index())
        | position.pieces_of(Piece::Queen.index()))
        & bishop_attacks(square, occupancy);
    let orthogonal = (position.pieces_of(Piece::Rook.index())
        | position.pieces_of(Piece::Queen.index()))
        & rook_attacks(square, occupancy);
    let black_pawns = position.pieces_for(Color::Black.index(), Piece::Pawn.index())
        & pawn_attacks(Color::White.index(), square);
    let white_pawns = position.pieces_for(Color::White.index(), Piece::Pawn.index())
        & pawn_attacks(Color::Black.index(), square);
    let knights = position.pieces_of(Piece::Knight.index()) & knight_attacks(square);
    let kings = position.pieces_of(Piece::King.index()) & king_attacks(square);
    let mut attackers = diagonal | orthogonal | black_pawns | white_pawns | knights | kings;
    while attackers != 0 {
        let source = attackers.trailing_zeros() as usize;
        attackers &= attackers - 1;
        if let Some(attacker) = position.piece_id(source) {
            sink.push_threat_delta(ThreatDelta::new(attacker, source, old_piece, square, false));
            sink.push_threat_delta(ThreatDelta::new(attacker, source, new_piece, square, true));
        }
    }
}

fn next_occupied_beyond(source: usize, through: usize, occupancy: u64) -> Option<usize> {
    let source_file = (source & 7) as i32;
    let source_rank = (source >> 3) as i32;
    let through_file = (through & 7) as i32;
    let through_rank = (through >> 3) as i32;
    let file_step = (through_file - source_file).signum();
    let rank_step = (through_rank - source_rank).signum();
    let mut file = through_file + file_step;
    let mut rank = through_rank + rank_step;
    while (0..8).contains(&file) && (0..8).contains(&rank) {
        let square = (rank * 8 + file) as usize;
        if occupancy & (1u64 << square) != 0 {
            return Some(square);
        }
        file += file_step;
        rank += rank_step;
    }
    None
}

#[inline(always)]
fn piece_attacks(piece: usize, color: usize, square: usize, occupancy: u64) -> u64 {
    match Piece::from_index(piece).expect("piece index is in range") {
        Piece::Pawn => pawn_attacks(color, square),
        Piece::Knight => knight_attacks(square),
        Piece::Bishop => bishop_attacks(square, occupancy),
        Piece::Rook => rook_attacks(square, occupancy),
        Piece::Queen => queen_attacks(square, occupancy),
        Piece::King => king_attacks(square),
    }
}
