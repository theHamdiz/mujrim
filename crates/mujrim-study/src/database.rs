//! Compact, dependency-free PGN library with a durable searchable index.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

const INDEX_FILE: &str = "games.tsv";
const SQLITE_FILE: &str = "mujrim.sqlite3";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameMetadata {
    pub event: String,
    pub site: String,
    pub date: String,
    pub round: String,
    pub white: String,
    pub black: String,
    pub result: String,
    pub white_elo: Option<u32>,
    pub black_elo: Option<u32>,
    pub eco: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameSummary {
    pub id: String,
    pub metadata: GameMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineMetadata {
    pub path: String,
    pub name: String,
    pub protocol: String,
    pub architecture: String,
    pub author: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub discovered: usize,
    pub imported: usize,
    pub duplicates: usize,
}

#[derive(Clone, Debug, Default)]
pub struct GameQuery {
    /// Case-insensitive text matched across players, event, site, and ECO.
    pub text: Option<String>,
    pub player: Option<String>,
    pub event: Option<String>,
    pub eco_prefix: Option<String>,
    pub min_elo: Option<u32>,
    pub max_elo: Option<u32>,
}

pub struct StudyDatabase {
    root: PathBuf,
    games: BTreeMap<String, GameMetadata>,
    sqlite: Connection,
}

impl StudyDatabase {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("games"))
            .map_err(|error| format!("failed to create study database: {error}"))?;
        let sqlite = Connection::open(root.join(SQLITE_FILE))
            .map_err(|error| format!("failed to open SQLite study database: {error}"))?;
        sqlite
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=NORMAL;
                 CREATE TABLE IF NOT EXISTS games (
                    id TEXT PRIMARY KEY,
                    event TEXT NOT NULL,
                    site TEXT NOT NULL,
                    date TEXT NOT NULL,
                    round TEXT NOT NULL,
                    white TEXT NOT NULL,
                    black TEXT NOT NULL,
                    result TEXT NOT NULL,
                    white_elo INTEGER,
                    black_elo INTEGER,
                    eco TEXT NOT NULL,
                    pgn TEXT NOT NULL,
                    created_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 CREATE INDEX IF NOT EXISTS games_players ON games(white, black);
                 CREATE INDEX IF NOT EXISTS games_event ON games(event);",
            )
            .map_err(|error| format!("failed to initialize SQLite study database: {error}"))?;
        sqlite
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS engines (
                    path TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    protocol TEXT NOT NULL,
                    architecture TEXT NOT NULL,
                    author TEXT NOT NULL,
                    last_seen INTEGER NOT NULL DEFAULT (unixepoch())
                 );",
            )
            .map_err(|error| format!("failed to initialize SQLite engine catalog: {error}"))?;
        crate::tournament_store::ensure_schema(&sqlite)?;
        let mut database = Self {
            root,
            games: BTreeMap::new(),
            sqlite,
        };
        database.load_sqlite_index()?;
        database.load_legacy_index()?;
        Ok(database)
    }

    pub fn len(&self) -> usize {
        self.games.len()
    }

    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }

    pub fn upsert_engine(&mut self, engine: &EngineMetadata) -> Result<(), String> {
        self.sqlite
            .execute(
                "INSERT INTO engines(path,name,protocol,architecture,author,last_seen)
                 VALUES (?1,?2,?3,?4,?5,unixepoch())
                 ON CONFLICT(path) DO UPDATE SET
                    name=excluded.name,
                    protocol=excluded.protocol,
                    architecture=excluded.architecture,
                    author=excluded.author,
                    last_seen=excluded.last_seen",
                params![
                    engine.path,
                    engine.name,
                    engine.protocol,
                    engine.architecture,
                    engine.author,
                ],
            )
            .map(|_| ())
            .map_err(|error| format!("failed to save engine metadata: {error}"))
    }

    pub fn engine_catalog(&self) -> Result<Vec<EngineMetadata>, String> {
        let mut statement = self
            .sqlite
            .prepare(
                "SELECT path,name,protocol,architecture,author
                 FROM engines ORDER BY name COLLATE NOCASE, path",
            )
            .map_err(|error| format!("failed to query engine catalog: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok(EngineMetadata {
                    path: row.get(0)?,
                    name: row.get(1)?,
                    protocol: row.get(2)?,
                    architecture: row.get(3)?,
                    author: row.get(4)?,
                })
            })
            .map_err(|error| format!("failed to read engine catalog: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("invalid engine catalog row: {error}"))
    }

    pub fn save_tournament(
        &mut self,
        tournament: &crate::tournament_store::StoredTournament,
    ) -> Result<(), String> {
        crate::tournament_store::save_tournament(&self.sqlite, tournament)
    }

    pub fn list_tournaments(
        &self,
    ) -> Result<Vec<crate::tournament_store::StoredTournament>, String> {
        crate::tournament_store::list_tournaments(&self.sqlite)
    }

    pub fn load_tournament(
        &self,
        id: &str,
    ) -> Result<Option<crate::tournament_store::StoredTournament>, String> {
        crate::tournament_store::load_tournament(&self.sqlite, id)
    }

    pub fn import_pgn(&mut self, metadata: GameMetadata, pgn: &str) -> Result<String, String> {
        let id = format!("{:016x}", fnv1a64(pgn.as_bytes()));
        if self.games.contains_key(&id) {
            return Ok(id);
        }

        let games_dir = self.root.join("games");
        let destination = games_dir.join(format!("{id}.pgn"));
        let temporary = games_dir.join(format!(".{id}.tmp"));
        fs::write(&temporary, pgn)
            .map_err(|error| format!("failed to write PGN staging file: {error}"))?;
        fs::rename(&temporary, &destination)
            .map_err(|error| format!("failed to commit PGN: {error}"))?;

        let index_path = self.root.join(INDEX_FILE);
        let mut index = OpenOptions::new()
            .create(true)
            .append(true)
            .open(index_path)
            .map_err(|error| format!("failed to open study index: {error}"))?;
        writeln!(index, "{}", encode_index_row(&id, &metadata))
            .map_err(|error| format!("failed to update study index: {error}"))?;
        index
            .sync_data()
            .map_err(|error| format!("failed to flush study index: {error}"))?;
        self.sqlite
            .execute(
                "INSERT OR IGNORE INTO games
                 (id,event,site,date,round,white,black,result,white_elo,black_elo,eco,pgn)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    id,
                    metadata.event,
                    metadata.site,
                    metadata.date,
                    metadata.round,
                    metadata.white,
                    metadata.black,
                    metadata.result,
                    metadata.white_elo,
                    metadata.black_elo,
                    metadata.eco,
                    pgn,
                ],
            )
            .map_err(|error| format!("failed to commit game to SQLite: {error}"))?;
        self.games.insert(id.clone(), metadata);
        Ok(id)
    }

    pub fn import_pgn_text(&mut self, pgn: &str) -> Result<ImportReport, String> {
        let games = crate::pgn::parse_games(pgn)?;
        let mut report = ImportReport {
            discovered: games.len(),
            ..ImportReport::default()
        };
        for game in games {
            let before = self.games.len();
            self.import_pgn(game.metadata, &game.source)?;
            if self.games.len() == before {
                report.duplicates += 1;
            } else {
                report.imported += 1;
            }
        }
        Ok(report)
    }

    pub fn import_pgn_file(&mut self, path: impl AsRef<Path>) -> Result<ImportReport, String> {
        let path = path.as_ref();
        let pgn = fs::read_to_string(path)
            .map_err(|error| format!("failed to read PGN '{}': {error}", path.display()))?;
        self.import_pgn_text(&pgn)
    }

    pub fn load_pgn(&self, id: &str) -> Result<String, String> {
        if !self.games.contains_key(id) {
            return Err(format!("game '{id}' is not in the study database"));
        }
        if let Some(pgn) = self
            .sqlite
            .query_row("SELECT pgn FROM games WHERE id=?1", [id], |row| row.get(0))
            .optional()
            .map_err(|error| format!("failed to read game '{id}' from SQLite: {error}"))?
        {
            return Ok(pgn);
        }
        fs::read_to_string(self.root.join("games").join(format!("{id}.pgn")))
            .map_err(|error| format!("failed to read game '{id}': {error}"))
    }

    pub fn load_game(&self, id: &str) -> Result<crate::pgn::ParsedGame, String> {
        let pgn = self.load_pgn(id)?;
        let mut games = crate::pgn::parse_games(&pgn)?;
        if games.len() != 1 {
            return Err(format!("stored game '{id}' contains multiple PGN games"));
        }
        Ok(games.remove(0))
    }

    pub fn search(&self, query: &GameQuery) -> Vec<GameSummary> {
        let text = query.text.as_deref().map(str::to_lowercase);
        let player = query.player.as_deref().map(str::to_lowercase);
        let event = query.event.as_deref().map(str::to_lowercase);
        let eco = query.eco_prefix.as_deref().map(str::to_lowercase);
        self.games
            .iter()
            .filter(|(_, metadata)| {
                text.as_ref().is_none_or(|needle| {
                    metadata.white.to_lowercase().contains(needle)
                        || metadata.black.to_lowercase().contains(needle)
                        || metadata.event.to_lowercase().contains(needle)
                        || metadata.site.to_lowercase().contains(needle)
                        || metadata.eco.to_lowercase().contains(needle)
                        || metadata
                            .white_elo
                            .is_some_and(|elo| elo.to_string().contains(needle))
                        || metadata
                            .black_elo
                            .is_some_and(|elo| elo.to_string().contains(needle))
                }) && player.as_ref().is_none_or(|needle| {
                    metadata.white.to_lowercase().contains(needle)
                        || metadata.black.to_lowercase().contains(needle)
                }) && event
                    .as_ref()
                    .is_none_or(|needle| metadata.event.to_lowercase().contains(needle))
                    && eco
                        .as_ref()
                        .is_none_or(|prefix| metadata.eco.to_lowercase().starts_with(prefix))
                    && query.min_elo.is_none_or(|minimum| {
                        metadata.white_elo.is_some_and(|elo| elo >= minimum)
                            || metadata.black_elo.is_some_and(|elo| elo >= minimum)
                    })
                    && query.max_elo.is_none_or(|maximum| {
                        metadata.white_elo.is_some_and(|elo| elo <= maximum)
                            || metadata.black_elo.is_some_and(|elo| elo <= maximum)
                    })
            })
            .map(|(id, metadata)| GameSummary {
                id: id.clone(),
                metadata: metadata.clone(),
            })
            .collect()
    }

    fn load_sqlite_index(&mut self) -> Result<(), String> {
        let mut statement = self
            .sqlite
            .prepare(
                "SELECT id,event,site,date,round,white,black,result,white_elo,black_elo,eco
                 FROM games ORDER BY created_at, id",
            )
            .map_err(|error| format!("failed to query SQLite study database: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    GameMetadata {
                        event: row.get(1)?,
                        site: row.get(2)?,
                        date: row.get(3)?,
                        round: row.get(4)?,
                        white: row.get(5)?,
                        black: row.get(6)?,
                        result: row.get(7)?,
                        white_elo: row.get(8)?,
                        black_elo: row.get(9)?,
                        eco: row.get(10)?,
                    },
                ))
            })
            .map_err(|error| format!("failed to read SQLite study database: {error}"))?;
        for row in rows {
            let (id, metadata) =
                row.map_err(|error| format!("invalid SQLite game row: {error}"))?;
            self.games.insert(id, metadata);
        }
        Ok(())
    }

    fn load_legacy_index(&mut self) -> Result<(), String> {
        let path = self.root.join(INDEX_FILE);
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("failed to open study index: {error}")),
        };
        for (line_index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|error| format!("failed to read study index: {error}"))?;
            let (id, metadata) = decode_index_row(&line)
                .map_err(|error| format!("invalid study index row {}: {error}", line_index + 1))?;
            self.games.insert(id, metadata);
        }
        Ok(())
    }
}

fn encode_index_row(id: &str, metadata: &GameMetadata) -> String {
    [
        id.to_owned(),
        escape(&metadata.event),
        escape(&metadata.site),
        escape(&metadata.date),
        escape(&metadata.round),
        escape(&metadata.white),
        escape(&metadata.black),
        escape(&metadata.result),
        metadata
            .white_elo
            .map_or_else(String::new, |elo| elo.to_string()),
        metadata
            .black_elo
            .map_or_else(String::new, |elo| elo.to_string()),
        escape(&metadata.eco),
    ]
    .join("\t")
}

fn decode_index_row(row: &str) -> Result<(String, GameMetadata), String> {
    let fields: Vec<_> = row.split('\t').collect();
    if fields.len() != 11 {
        return Err(format!("expected 11 fields, found {}", fields.len()));
    }
    let parse_elo = |value: &str| {
        if value.is_empty() {
            Ok(None)
        } else {
            value
                .parse::<u32>()
                .map(Some)
                .map_err(|error| format!("invalid Elo '{value}': {error}"))
        }
    };
    Ok((
        fields[0].to_owned(),
        GameMetadata {
            event: unescape(fields[1])?,
            site: unescape(fields[2])?,
            date: unescape(fields[3])?,
            round: unescape(fields[4])?,
            white: unescape(fields[5])?,
            black: unescape(fields[6])?,
            result: unescape(fields[7])?,
            white_elo: parse_elo(fields[8])?,
            black_elo: parse_elo(fields[9])?,
            eco: unescape(fields[10])?,
        },
    ))
}

fn escape(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\t', "%09")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn unescape(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            output.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return Err("truncated percent escape".to_owned());
        }
        let digits = std::str::from_utf8(&bytes[index + 1..index + 3])
            .map_err(|_| "invalid percent escape".to_owned())?;
        output.push(
            u8::from_str_radix(digits, 16)
                .map_err(|_| format!("invalid percent escape '%{digits}'"))?,
        );
        index += 3;
    }
    String::from_utf8(output).map_err(|error| format!("invalid UTF-8 in index: {error}"))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_database() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("mujrim-study-test-{}-{unique}", std::process::id()))
    }

    #[test]
    fn imported_games_are_deduplicated_persistent_and_searchable() {
        let root = temporary_database();
        let metadata = GameMetadata {
            event: "Candidates\tFinal".to_owned(),
            white: "Player One".to_owned(),
            black: "Player Two".to_owned(),
            result: "1-0".to_owned(),
            white_elo: Some(2750),
            black_elo: Some(2700),
            eco: "C65".to_owned(),
            ..Default::default()
        };
        let pgn = "[Event \"Candidates\"]\n\n1. e4 e5 1-0\n";
        let id = {
            let mut database = StudyDatabase::open(&root).unwrap();
            let first = database.import_pgn(metadata.clone(), pgn).unwrap();
            let duplicate = database.import_pgn(metadata, pgn).unwrap();
            assert_eq!(first, duplicate);
            assert_eq!(database.len(), 1);
            first
        };

        assert!(root.join(SQLITE_FILE).is_file());
        fs::remove_file(root.join("games").join(format!("{id}.pgn"))).unwrap();
        let database = StudyDatabase::open(&root).unwrap();
        assert_eq!(database.load_pgn(&id).unwrap(), pgn);
        let matches = database.search(&GameQuery {
            player: Some("player one".to_owned()),
            eco_prefix: Some("C6".to_owned()),
            min_elo: Some(2740),
            ..Default::default()
        });
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].metadata.event, "Candidates\tFinal");

        drop(database);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn collection_import_reports_new_and_duplicate_games() {
        let root = temporary_database();
        let pgn = "[Event \"One\"]\n[White \"A\"]\n[Black \"B\"]\n\n1. e4 e5 1-0\n\n[Event \"Two\"]\n[White \"C\"]\n[Black \"D\"]\n\n1. d4 d5 1/2-1/2\n";
        let mut database = StudyDatabase::open(&root).unwrap();
        let first = database.import_pgn_text(pgn).unwrap();
        assert_eq!(first.discovered, 2);
        assert_eq!(first.imported, 2);
        assert_eq!(first.duplicates, 0);

        let second = database.import_pgn_text(pgn).unwrap();
        assert_eq!(second.imported, 0);
        assert_eq!(second.duplicates, 2);
        let summaries = database.search(&GameQuery {
            player: Some("c".to_owned()),
            ..GameQuery::default()
        });
        assert_eq!(summaries.len(), 1);
        assert_eq!(
            database.load_game(&summaries[0].id).unwrap().moves[0],
            "d2d4"
        );
        drop(database);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn malformed_index_rows_are_rejected() {
        assert!(decode_index_row("too\tfew").is_err());
        assert!(unescape("broken%2").is_err());
    }

    #[test]
    fn free_text_searches_primary_metadata() {
        let root = temporary_database();
        let mut database = StudyDatabase::open(&root).unwrap();
        database
            .import_pgn(
                GameMetadata {
                    event: "Candidates Final".to_owned(),
                    site: "Madrid".to_owned(),
                    white: "Capablanca".to_owned(),
                    black: "Alekhine".to_owned(),
                    eco: "C42".to_owned(),
                    white_elo: Some(2725),
                    ..GameMetadata::default()
                },
                "1. e4 e5 *",
            )
            .unwrap();

        for needle in [
            "candidates",
            "madrid",
            "capablanca",
            "alekhine",
            "c42",
            "2725",
        ] {
            assert_eq!(
                database
                    .search(&GameQuery {
                        text: Some(needle.to_owned()),
                        ..GameQuery::default()
                    })
                    .len(),
                1,
                "needle={needle}"
            );
        }
        drop(database);
        fs::remove_dir_all(&root).unwrap();
    }
}
