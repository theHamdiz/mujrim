//! Durable tournament records backed by the study SQLite database.

use rusqlite::{Connection, OptionalExtension, params};

use crate::tournament::{
    Entrant, Pairing, Standing, TournamentFormat, TournamentResult, schedule, standings,
};

#[derive(Clone, Debug, PartialEq)]
pub struct StoredTournament {
    pub id: String,
    pub name: String,
    pub format: TournamentFormat,
    pub created_at: i64,
    pub status: String,
    pub entrants: Vec<Entrant>,
    pub results: Vec<TournamentResult>,
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
    Ok(())
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
        let tournament = load_tournament(sqlite, &id)?
            .ok_or_else(|| format!("tournament '{id}' listed but missing during load"))?;
        tournaments.push(tournament);
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
    Ok(Some(StoredTournament {
        id,
        name,
        format: parse_format(&format)?,
        created_at,
        status,
        entrants,
        results,
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
        };
        database.save_tournament(&tournament).unwrap();
        let loaded = database.load_tournament("t1").unwrap().unwrap();
        assert_eq!(loaded.entrants.len(), 2);
        assert_eq!(loaded.results.len(), 1);
        assert_eq!(loaded.standings()[0].points, 1.0);
        assert_eq!(database.list_tournaments().unwrap().len(), 1);
        drop(database);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
