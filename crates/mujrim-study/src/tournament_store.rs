//! Durable tournament records backed by the study SQLite database.

use rusqlite::{Connection, OptionalExtension, params};

use crate::tournament::{
    Entrant, Pairing, Standing, TournamentFormat, TournamentResult, schedule, standings,
};

#[derive(Clone, Debug, PartialEq)]
pub struct StoredTournamentGame {
    pub game_index: usize,
    pub round: usize,
    pub white: String,
    pub black: String,
    pub white_score: f64,
    pub initial_fen: String,
    pub moves: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredTournament {
    pub id: String,
    pub name: String,
    pub format: TournamentFormat,
    pub created_at: i64,
    pub status: String,
    pub entrants: Vec<Entrant>,
    pub results: Vec<TournamentResult>,
    pub games: Vec<StoredTournamentGame>,
}

impl StoredTournament {
    pub fn pairings(&self) -> Vec<Pairing> {
        schedule(self.entrants.len(), self.format)
    }

    pub fn standings(&self) -> Vec<Standing> {
        standings(&self.entrants, &self.results)
    }
}

pub fn ensure_schema(sqlite: &Connection) -> Result<(), String> {
    sqlite
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS tournaments (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                format TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
             );
             CREATE TABLE IF NOT EXISTS tournament_entrants (
                tournament_id TEXT NOT NULL,
                ord INTEGER NOT NULL,
                entrant_id TEXT NOT NULL,
                name TEXT NOT NULL,
                seed_elo REAL,
                PRIMARY KEY (tournament_id, ord),
                FOREIGN KEY (tournament_id) REFERENCES tournaments(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS tournament_results (
                tournament_id TEXT NOT NULL,
                round INTEGER NOT NULL,
                white INTEGER NOT NULL,
                black INTEGER NOT NULL,
                white_score REAL NOT NULL,
                PRIMARY KEY (tournament_id, round, white, black),
                FOREIGN KEY (tournament_id) REFERENCES tournaments(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS tournament_games (
                tournament_id TEXT NOT NULL,
                game_index INTEGER NOT NULL,
                round INTEGER NOT NULL,
                white TEXT NOT NULL,
                black TEXT NOT NULL,
                white_score REAL NOT NULL,
                initial_fen TEXT NOT NULL,
                moves TEXT NOT NULL,
                PRIMARY KEY (tournament_id, game_index),
                FOREIGN KEY (tournament_id) REFERENCES tournaments(id) ON DELETE CASCADE
             );",
        )
        .map_err(|error| format!("failed to initialize tournament schema: {error}"))
}

pub fn save_tournament(sqlite: &Connection, tournament: &StoredTournament) -> Result<(), String> {
    sqlite
        .execute(
            "INSERT INTO tournaments(id,name,format,status,created_at)
             VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(id) DO UPDATE SET
                name=excluded.name,
                format=excluded.format,
                status=excluded.status",
            params![
                tournament.id,
                tournament.name,
                format_key(tournament.format),
                tournament.status,
                tournament.created_at,
            ],
        )
        .map_err(|error| format!("failed to save tournament: {error}"))?;
    sqlite
        .execute(
            "DELETE FROM tournament_entrants WHERE tournament_id=?1",
            params![tournament.id],
        )
        .map_err(|error| format!("failed to clear tournament entrants: {error}"))?;
    sqlite
        .execute(
            "DELETE FROM tournament_results WHERE tournament_id=?1",
            params![tournament.id],
        )
        .map_err(|error| format!("failed to clear tournament results: {error}"))?;
    for (ord, entrant) in tournament.entrants.iter().enumerate() {
        sqlite
            .execute(
                "INSERT INTO tournament_entrants(tournament_id,ord,entrant_id,name,seed_elo)
                 VALUES (?1,?2,?3,?4,?5)",
                params![
                    tournament.id,
                    ord as i64,
                    entrant.id,
                    entrant.name,
                    entrant.seed_elo,
                ],
            )
            .map_err(|error| format!("failed to save tournament entrant: {error}"))?;
    }
    for result in &tournament.results {
        sqlite
            .execute(
                "INSERT INTO tournament_results(tournament_id,round,white,black,white_score)
                 VALUES (?1,?2,?3,?4,?5)",
                params![
                    tournament.id,
                    result.pairing.round as i64,
                    result.pairing.white as i64,
                    result.pairing.black as i64,
                    result.white_score,
                ],
            )
            .map_err(|error| format!("failed to save tournament result: {error}"))?;
    }
    sqlite
        .execute(
            "DELETE FROM tournament_games WHERE tournament_id=?1",
            params![tournament.id],
        )
        .map_err(|error| format!("failed to clear tournament games: {error}"))?;
    for game in &tournament.games {
        sqlite
            .execute(
                "INSERT INTO tournament_games(
                    tournament_id,game_index,round,white,black,white_score,initial_fen,moves
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    tournament.id,
                    game.game_index as i64,
                    game.round as i64,
                    game.white,
                    game.black,
                    game.white_score,
                    game.initial_fen,
                    game.moves.join(" "),
                ],
            )
            .map_err(|error| format!("failed to save tournament game: {error}"))?;
    }
    Ok(())
}

pub fn delete_tournament(sqlite: &Connection, id: &str) -> Result<(), String> {
    sqlite
        .execute(
            "DELETE FROM tournament_games WHERE tournament_id=?1",
            params![id],
        )
        .map_err(|error| format!("failed to delete tournament games: {error}"))?;
    sqlite
        .execute(
            "DELETE FROM tournament_results WHERE tournament_id=?1",
            params![id],
        )
        .map_err(|error| format!("failed to delete tournament results: {error}"))?;
    sqlite
        .execute(
            "DELETE FROM tournament_entrants WHERE tournament_id=?1",
            params![id],
        )
        .map_err(|error| format!("failed to delete tournament entrants: {error}"))?;
    sqlite
        .execute("DELETE FROM tournaments WHERE id=?1", params![id])
        .map_err(|error| format!("failed to delete tournament: {error}"))?;
    Ok(())
}

pub const STATUS_PAUSED: &str = "paused";
pub const STATUS_IN_PROGRESS: &str = "in-progress";
pub const STATUS_FINISHED: &str = "finished";
pub const STATUS_CANCELLED: &str = "cancelled";

pub fn is_resumable_status(status: &str) -> bool {
    matches!(lifecycle_key(status), STATUS_PAUSED | STATUS_IN_PROGRESS)
}

pub fn lifecycle_key(status: &str) -> &'static str {
    let lowered = status.trim().to_ascii_lowercase();
    if lowered == STATUS_PAUSED || lowered.starts_with("paused") {
        STATUS_PAUSED
    } else if lowered == STATUS_IN_PROGRESS || lowered.starts_with("in-progress") {
        STATUS_IN_PROGRESS
    } else if lowered == STATUS_CANCELLED || lowered.contains("cancelled") {
        STATUS_CANCELLED
    } else if lowered == STATUS_FINISHED || lowered.contains("finished") {
        STATUS_FINISHED
    } else if lowered.starts_with("playing") || lowered.starts_with("starting") {
        STATUS_IN_PROGRESS
    } else {
        STATUS_FINISHED
    }
}

pub fn lifecycle_status(
    paused: bool,
    running: bool,
    cancelled: bool,
    finished: bool,
) -> &'static str {
    if cancelled {
        STATUS_CANCELLED
    } else if finished {
        STATUS_FINISHED
    } else if paused {
        STATUS_PAUSED
    } else if running {
        STATUS_IN_PROGRESS
    } else {
        STATUS_FINISHED
    }
}

pub fn list_tournaments(sqlite: &Connection) -> Result<Vec<StoredTournament>, String> {
    let mut statement = sqlite
        .prepare("SELECT id FROM tournaments ORDER BY created_at DESC")
        .map_err(|error| format!("failed to query tournaments: {error}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| format!("failed to read tournaments: {error}"))?;
    let mut tournaments = Vec::new();
    for row in rows {
        let id = row.map_err(|error| format!("invalid tournament row: {error}"))?;
        match load_tournament(sqlite, &id) {
            Ok(Some(tournament)) => tournaments.push(tournament),
            Ok(None) | Err(_) => continue,
        }
    }
    Ok(tournaments)
}

pub fn load_tournament(sqlite: &Connection, id: &str) -> Result<Option<StoredTournament>, String> {
    let meta = sqlite
        .query_row(
            "SELECT id,name,format,status,created_at FROM tournaments WHERE id=?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("failed to load tournament: {error}"))?;
    let Some((id, name, format, status, created_at)) = meta else {
        return Ok(None);
    };
    let mut entrant_stmt = sqlite
        .prepare(
            "SELECT entrant_id,name,seed_elo FROM tournament_entrants
             WHERE tournament_id=?1 ORDER BY ord",
        )
        .map_err(|error| format!("failed to query entrants: {error}"))?;
    let entrants = entrant_stmt
        .query_map(params![id], |row| {
            Ok(Entrant {
                id: row.get(0)?,
                name: row.get(1)?,
                seed_elo: row.get(2)?,
            })
        })
        .map_err(|error| format!("failed to read entrants: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("invalid entrant row: {error}"))?;
    let mut result_stmt = sqlite
        .prepare(
            "SELECT round,white,black,white_score FROM tournament_results
             WHERE tournament_id=?1 ORDER BY round, white, black",
        )
        .map_err(|error| format!("failed to query results: {error}"))?;
    let results = result_stmt
        .query_map(params![id], |row| {
            Ok(TournamentResult {
                pairing: Pairing {
                    round: row.get::<_, i64>(0)? as usize,
                    white: row.get::<_, i64>(1)? as usize,
                    black: row.get::<_, i64>(2)? as usize,
                },
                white_score: row.get(3)?,
            })
        })
        .map_err(|error| format!("failed to read results: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("invalid result row: {error}"))?;
    let mut game_stmt = sqlite
        .prepare(
            "SELECT game_index,round,white,black,white_score,initial_fen,moves
             FROM tournament_games WHERE tournament_id=?1 ORDER BY game_index",
        )
        .map_err(|error| format!("failed to query tournament games: {error}"))?;
    let games = game_stmt
        .query_map(params![id], |row| {
            let moves: String = row.get(6)?;
            Ok(StoredTournamentGame {
                game_index: row.get::<_, i64>(0)? as usize,
                round: row.get::<_, i64>(1)? as usize,
                white: row.get(2)?,
                black: row.get(3)?,
                white_score: row.get(4)?,
                initial_fen: row.get(5)?,
                moves: if moves.is_empty() {
                    Vec::new()
                } else {
                    moves.split_whitespace().map(str::to_owned).collect()
                },
            })
        })
        .map_err(|error| format!("failed to read tournament games: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("invalid tournament game row: {error}"))?;
    Ok(Some(StoredTournament {
        id,
        name,
        format: parse_format(&format)?,
        created_at,
        status,
        entrants,
        results,
        games,
    }))
}

fn format_key(format: TournamentFormat) -> &'static str {
    match format {
        TournamentFormat::RoundRobin => "round_robin",
        TournamentFormat::DoubleRoundRobin => "double_round_robin",
        TournamentFormat::Swiss => "swiss",
        TournamentFormat::Knockout => "knockout",
    }
}

fn parse_format(value: &str) -> Result<TournamentFormat, String> {
    match value {
        "round_robin" => Ok(TournamentFormat::RoundRobin),
        "double_round_robin" => Ok(TournamentFormat::DoubleRoundRobin),
        "swiss" => Ok(TournamentFormat::Swiss),
        "knockout" => Ok(TournamentFormat::Knockout),
        other => Err(format!("unknown tournament format '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::StudyDatabase;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_root() -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mujrim-tournament-store-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn tournament_round_trip_persists_standings_inputs() {
        let root = temporary_root();
        let mut database = StudyDatabase::open(&root).unwrap();
        let tournament = StoredTournament {
            id: "t1".to_owned(),
            name: "Quick RR".to_owned(),
            format: TournamentFormat::RoundRobin,
            created_at: 1,
            status: "finished".to_owned(),
            entrants: vec![
                Entrant {
                    id: "a".into(),
                    name: "Alpha".into(),
                    seed_elo: Some(2400.0),
                },
                Entrant {
                    id: "b".into(),
                    name: "Beta".into(),
                    seed_elo: Some(2300.0),
                },
            ],
            results: vec![TournamentResult {
                pairing: Pairing {
                    round: 1,
                    white: 0,
                    black: 1,
                },
                white_score: 1.0,
            }],
            games: vec![StoredTournamentGame {
                game_index: 0,
                round: 1,
                white: "Alpha".into(),
                black: "Beta".into(),
                white_score: 1.0,
                initial_fen: crate::opening::START_FEN.into(),
                moves: vec!["e2e4".into(), "e7e5".into()],
            }],
        };
        database.save_tournament(&tournament).unwrap();
        let loaded = database.load_tournament("t1").unwrap().unwrap();
        assert_eq!(loaded.entrants.len(), 2);
        assert_eq!(loaded.results.len(), 1);
        assert_eq!(loaded.games.len(), 1);
        assert_eq!(loaded.games[0].moves, ["e2e4", "e7e5"]);
        assert_eq!(loaded.standings()[0].points, 1.0);
        assert_eq!(database.list_tournaments().unwrap().len(), 1);
        database.delete_tournament("t1").unwrap();
        assert!(database.list_tournaments().unwrap().is_empty());
        drop(database);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resumable_status_detects_paused_and_in_progress() {
        assert!(is_resumable_status("paused"));
        assert!(is_resumable_status("Paused — clocks frozen"));
        assert!(is_resumable_status("in-progress"));
        assert!(is_resumable_status("Playing 1/4 · Round 1"));
        assert!(!is_resumable_status("finished"));
        assert!(!is_resumable_status(
            "Round Robin finished without completed games."
        ));
        assert!(!is_resumable_status("cancelled"));
        assert_eq!(lifecycle_status(true, true, false, false), STATUS_PAUSED);
    }

    #[test]
    fn list_tournaments_skips_corrupt_rows() {
        let root = temporary_root();
        let mut database = StudyDatabase::open(&root).unwrap();
        let tournament = StoredTournament {
            id: "good".to_owned(),
            name: "Keep Me".to_owned(),
            format: TournamentFormat::RoundRobin,
            created_at: 2,
            status: STATUS_PAUSED.to_owned(),
            entrants: vec![
                Entrant {
                    id: "a".into(),
                    name: "Alpha".into(),
                    seed_elo: None,
                },
                Entrant {
                    id: "b".into(),
                    name: "Beta".into(),
                    seed_elo: None,
                },
            ],
            results: Vec::new(),
            games: Vec::new(),
        };
        database.save_tournament(&tournament).unwrap();
        rusqlite::Connection::open(root.join("mujrim.sqlite3"))
            .unwrap()
            .execute(
                "INSERT INTO tournaments(id,name,format,status,created_at)
                 VALUES ('bad','Broken','not_a_format','finished',1)",
                [],
            )
            .unwrap();
        let listed = database.list_tournaments().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "good");
        drop(database);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
