//! Embedded vector chess-piece catalog.

use iced::widget::svg::Handle;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PieceSet {
    #[default]
    #[serde(alias = "Classic")]
    Cburnett,
    #[serde(alias = "Tournament")]
    Merida,
    Alpha,
    Staunty,
    Cardinal,
    Caliente,
    Fresca,
    Gioco,
    Tatiana,
    Maestro,
}

impl PieceSet {
    pub const ALL: [Self; 10] = [
        Self::Cburnett,
        Self::Merida,
        Self::Alpha,
        Self::Staunty,
        Self::Cardinal,
        Self::Caliente,
        Self::Fresca,
        Self::Gioco,
        Self::Tatiana,
        Self::Maestro,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Cburnett => 0,
            Self::Merida => 1,
            Self::Alpha => 2,
            Self::Staunty => 3,
            Self::Cardinal => 4,
            Self::Caliente => 5,
            Self::Fresca => 6,
            Self::Gioco => 7,
            Self::Tatiana => 8,
            Self::Maestro => 9,
        }
    }
}

impl std::fmt::Display for PieceSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cburnett => "Cburnett",
            Self::Merida => "Merida",
            Self::Alpha => "Alpha",
            Self::Staunty => "Staunty",
            Self::Cardinal => "Cardinal",
            Self::Caliente => "Caliente",
            Self::Fresca => "Fresca",
            Self::Gioco => "Gioco",
            Self::Tatiana => "Tatiana",
            Self::Maestro => "Maestro",
        })
    }
}

struct PieceAssetSet {
    white_king: Handle,
    white_queen: Handle,
    white_rook: Handle,
    white_bishop: Handle,
    white_knight: Handle,
    white_pawn: Handle,
    black_king: Handle,
    black_queen: Handle,
    black_rook: Handle,
    black_bishop: Handle,
    black_knight: Handle,
    black_pawn: Handle,
}

macro_rules! embedded_set {
    ($folder:literal) => {
        PieceAssetSet {
            white_king: Handle::from_memory(include_bytes!(concat!(
                "../assets/pieces/",
                $folder,
                "/wK.svg"
            ))),
            white_queen: Handle::from_memory(include_bytes!(concat!(
                "../assets/pieces/",
                $folder,
                "/wQ.svg"
            ))),
            white_rook: Handle::from_memory(include_bytes!(concat!(
                "../assets/pieces/",
                $folder,
                "/wR.svg"
            ))),
            white_bishop: Handle::from_memory(include_bytes!(concat!(
                "../assets/pieces/",
                $folder,
                "/wB.svg"
            ))),
            white_knight: Handle::from_memory(include_bytes!(concat!(
                "../assets/pieces/",
                $folder,
                "/wN.svg"
            ))),
            white_pawn: Handle::from_memory(include_bytes!(concat!(
                "../assets/pieces/",
                $folder,
                "/wP.svg"
            ))),
            black_king: Handle::from_memory(include_bytes!(concat!(
                "../assets/pieces/",
                $folder,
                "/bK.svg"
            ))),
            black_queen: Handle::from_memory(include_bytes!(concat!(
                "../assets/pieces/",
                $folder,
                "/bQ.svg"
            ))),
            black_rook: Handle::from_memory(include_bytes!(concat!(
                "../assets/pieces/",
                $folder,
                "/bR.svg"
            ))),
            black_bishop: Handle::from_memory(include_bytes!(concat!(
                "../assets/pieces/",
                $folder,
                "/bB.svg"
            ))),
            black_knight: Handle::from_memory(include_bytes!(concat!(
                "../assets/pieces/",
                $folder,
                "/bN.svg"
            ))),
            black_pawn: Handle::from_memory(include_bytes!(concat!(
                "../assets/pieces/",
                $folder,
                "/bP.svg"
            ))),
        }
    };
}

/// Preloaded, in-memory handles for all embedded vector piece sets.
pub struct PieceAssets {
    sets: [PieceAssetSet; PieceSet::ALL.len()],
}

impl PieceAssets {
    pub fn load() -> Self {
        Self {
            sets: [
                embedded_set!("cburnett"),
                embedded_set!("merida"),
                embedded_set!("alpha"),
                embedded_set!("staunty"),
                embedded_set!("cardinal"),
                embedded_set!("caliente"),
                embedded_set!("fresca"),
                embedded_set!("gioco"),
                embedded_set!("tatiana"),
                embedded_set!("maestro"),
            ],
        }
    }

    pub fn get(&self, set: PieceSet, piece: types::Piece, color: types::Color) -> &Handle {
        let assets = &self.sets[set.index()];
        match (piece, color) {
            (types::Piece::King, types::Color::White) => &assets.white_king,
            (types::Piece::Queen, types::Color::White) => &assets.white_queen,
            (types::Piece::Rook, types::Color::White) => &assets.white_rook,
            (types::Piece::Bishop, types::Color::White) => &assets.white_bishop,
            (types::Piece::Knight, types::Color::White) => &assets.white_knight,
            (types::Piece::Pawn, types::Color::White) => &assets.white_pawn,
            (types::Piece::King, types::Color::Black) => &assets.black_king,
            (types::Piece::Queen, types::Color::Black) => &assets.black_queen,
            (types::Piece::Rook, types::Color::Black) => &assets.black_rook,
            (types::Piece::Bishop, types::Color::Black) => &assets.black_bishop,
            (types::Piece::Knight, types::Color::Black) => &assets.black_knight,
            (types::Piece::Pawn, types::Color::Black) => &assets.black_pawn,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cburnett_is_the_default_and_legacy_classic_alias() {
        #[derive(serde::Deserialize)]
        struct Settings {
            piece_set: PieceSet,
        }

        assert_eq!(PieceSet::default(), PieceSet::Cburnett);
        assert_eq!(
            toml::from_str::<Settings>("piece_set = \"Classic\"")
                .unwrap()
                .piece_set,
            PieceSet::Cburnett
        );
    }

    #[test]
    fn catalog_contains_ten_named_vector_sets() {
        assert_eq!(PieceSet::ALL.len(), 10);
        let names = PieceSet::ALL.map(|set| set.to_string());
        assert_eq!(names[0], "Cburnett");
        assert!(names.contains(&"Tatiana".to_owned()));
        assert!(names.contains(&"Maestro".to_owned()));
    }

    #[test]
    fn every_embedded_set_resolves_all_twelve_pieces() {
        let assets = PieceAssets::load();
        for set in PieceSet::ALL {
            for piece in types::Piece::ALL {
                for color in [types::Color::White, types::Color::Black] {
                    let _ = assets.get(set, piece, color);
                }
            }
        }
    }
}
