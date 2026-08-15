//! Public training-data offers the CLI and GUI can fetch by id.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetKind {
    Stockfish,
    Lc0,
    SelfPlay,
}

impl DatasetKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stockfish => "stockfish",
            Self::Lc0 => "lc0",
            Self::SelfPlay => "selfplay",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatasetOffer {
    pub id: &'static str,
    pub kind: DatasetKind,
    pub url: &'static str,
    pub filename: &'static str,
    pub notes: &'static str,
}

pub const DATASETS: &[DatasetOffer] = &[
    DatasetOffer {
        id: "stockfish-plain",
        kind: DatasetKind::Stockfish,
        url: "https://data.stockfishchess.org/nnue/",
        filename: "training.plain.gz",
        notes: "Stockfish self-play as .plain or .plain.gz only. Pass --url for a concrete file; fetch decompresses and decodes it.",
    },
    DatasetOffer {
        id: "lc0-training",
        kind: DatasetKind::Lc0,
        url: "https://storage.lczero.org/files/training_data/",
        filename: "training.gz",
        notes: "Lc0 self-play v3–v6 chunks. Pass --url for a dated .gz; fetch decompresses and decodes to text.",
    },
    DatasetOffer {
        id: "selfplay-gz",
        kind: DatasetKind::SelfPlay,
        url: "",
        filename: "selfplay.txt.gz",
        notes: "Compressed Mujrim self-play from `mujrim train datagen`. Use a local path or --url.",
    },
];

pub fn find_dataset(id: &str) -> Option<&'static DatasetOffer> {
    match id {
        "stockfish-binpack" | "stockfish-plain" => DATASETS
            .iter()
            .find(|offer| offer.kind == DatasetKind::Stockfish),
        other => DATASETS.iter().find(|offer| offer.id == other),
    }
}

fn stockfish_url_is_plain(url: &str) -> bool {
    let name = url.rsplit('/').next().unwrap_or(url);
    name.ends_with(".plain") || name.ends_with(".plain.gz") || name.ends_with(".plain.zst")
}

pub fn resolve_fetch_url(id: Option<&str>, url: &str) -> Result<(String, String), String> {
    let url = url.trim();
    if let Some(offer) = id.and_then(find_dataset) {
        let chosen = if url.is_empty() { offer.url } else { url };
        if chosen.is_empty() {
            return Err(format!(
                "dataset `{}` is local-only; pass --url or a filesystem path",
                offer.id
            ));
        }
        if chosen.ends_with('/') {
            return Err(format!(
                "dataset `{}` is a directory root ({chosen}); pass --url for a specific file",
                offer.id
            ));
        }
        if offer.kind == DatasetKind::Stockfish && !stockfish_url_is_plain(chosen) {
            return Err(
                "Stockfish dumps must be .plain, .plain.gz, or .plain.zst (not BINP/binpack)"
                    .into(),
            );
        }
        return Ok((chosen.to_string(), offer.filename.to_string()));
    }
    if url.is_empty() {
        return Err("pass --url or a catalog --id".to_string());
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("dataset URL must be http(s)".to_string());
    }
    if url.ends_with('/') {
        return Err("pass a file URL, not a directory".to_string());
    }
    let filename = url
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("data.bin")
        .to_string();
    Ok((url.to_string(), filename))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_stockfish_lc0_and_selfplay() {
        assert_eq!(DATASETS.len(), 3);
        assert_eq!(
            find_dataset("stockfish-plain").map(|offer| offer.kind),
            Some(DatasetKind::Stockfish)
        );
        assert_eq!(
            find_dataset("stockfish-binpack").map(|offer| offer.id),
            Some("stockfish-plain")
        );
        assert_eq!(
            find_dataset("lc0-training").map(|offer| offer.kind),
            Some(DatasetKind::Lc0)
        );
        assert!(find_dataset("missing").is_none());
    }

    #[test]
    fn resolve_fetch_url_rejects_directory_roots() {
        assert!(
            resolve_fetch_url(Some("stockfish-plain"), "")
                .unwrap_err()
                .contains("directory")
        );
        assert!(
            resolve_fetch_url(
                Some("stockfish-plain"),
                "https://example.test/games.binpack"
            )
            .unwrap_err()
            .contains(".plain")
        );
        let (url, name) = resolve_fetch_url(
            Some("stockfish-plain"),
            "https://example.test/games.plain.gz",
        )
        .unwrap();
        assert!(url.ends_with(".plain.gz"));
        assert_eq!(name, "training.plain.gz");
        assert!(
            resolve_fetch_url(Some("selfplay-gz"), "")
                .unwrap_err()
                .contains("local-only")
        );
        let (url, name) = resolve_fetch_url(
            Some("lc0-training"),
            "https://storage.lczero.org/files/training_data/run1/training.1.gz",
        )
        .unwrap();
        assert!(url.ends_with(".gz"));
        assert_eq!(name, "training.gz");
        assert!(resolve_fetch_url(None, "file:///tmp/x").is_err());
    }
}
