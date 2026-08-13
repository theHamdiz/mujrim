//! Durable puzzle library and spaced-repetition progress.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::training::{Puzzle, ReviewSchedule};

const STORE_FILE: &str = "training.tsv";
const BACKUP_FILE: &str = ".training.tsv.backup";
const LIST_SEPARATOR: char = '\u{1f}';

#[derive(Clone, Debug, PartialEq)]
pub struct TrainingItem {
    pub puzzle: Puzzle,
    pub schedule: ReviewSchedule,
}

pub struct TrainingStore {
    root: PathBuf,
    items: BTreeMap<String, TrainingItem>,
}

impl TrainingStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)
            .map_err(|error| format!("failed to create training database: {error}"))?;
        let mut store = Self {
            root,
            items: BTreeMap::new(),
        };
        store.recover_interrupted_commit()?;
        store.load()?;
        Ok(store)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn add(&mut self, puzzle: Puzzle) -> Result<bool, String> {
        puzzle.validate()?;
        if self.items.contains_key(&puzzle.id) {
            return Ok(false);
        }
        let id = puzzle.id.clone();
        self.items.insert(
            id.clone(),
            TrainingItem {
                puzzle,
                schedule: ReviewSchedule::default(),
            },
        );
        if let Err(error) = self.persist() {
            self.items.remove(&id);
            return Err(error);
        }
        Ok(true)
    }

    pub fn due(&self, today: u64, limit: usize) -> Vec<TrainingItem> {
        let mut due = self
            .items
            .values()
            .filter(|item| item.schedule.is_due(today))
            .cloned()
            .collect::<Vec<_>>();
        due.sort_by(|left, right| {
            left.schedule
                .due_day
                .cmp(&right.schedule.due_day)
                .then_with(|| right.puzzle.rating.cmp(&left.puzzle.rating))
                .then_with(|| left.puzzle.id.cmp(&right.puzzle.id))
        });
        due.truncate(limit);
        due
    }

    pub fn get(&self, id: &str) -> Option<&TrainingItem> {
        self.items.get(id)
    }

    pub fn review(&mut self, id: &str, grade: u8, today: u64) -> Result<ReviewSchedule, String> {
        let item = self
            .items
            .get_mut(id)
            .ok_or_else(|| format!("puzzle '{id}' is not in the training database"))?;
        let previous = item.schedule;
        item.schedule = item.schedule.review(grade, today);
        let schedule = item.schedule;
        if let Err(error) = self.persist() {
            self.items
                .get_mut(id)
                .expect("reviewed puzzle exists")
                .schedule = previous;
            return Err(error);
        }
        Ok(schedule)
    }

    fn recover_interrupted_commit(&self) -> Result<(), String> {
        let path = self.root.join(STORE_FILE);
        let backup = self.root.join(BACKUP_FILE);
        if !path.exists() && backup.exists() {
            fs::rename(&backup, &path)
                .map_err(|error| format!("failed to recover training database: {error}"))?;
        } else if path.exists() && backup.exists() {
            fs::remove_file(backup)
                .map_err(|error| format!("failed to remove stale training backup: {error}"))?;
        }
        Ok(())
    }

    fn load(&mut self) -> Result<(), String> {
        let path = self.root.join(STORE_FILE);
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("failed to read training database: {error}")),
        };
        for (index, line) in contents.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let item = decode_item(line)
                .map_err(|error| format!("invalid training row {}: {error}", index + 1))?;
            item.puzzle.validate().map_err(|error| {
                format!(
                    "invalid training puzzle '{}' at row {}: {error}",
                    item.puzzle.id,
                    index + 1
                )
            })?;
            self.items.insert(item.puzzle.id.clone(), item);
        }
        Ok(())
    }

    fn persist(&self) -> Result<(), String> {
        let path = self.root.join(STORE_FILE);
        let temporary = self.root.join(format!(".{STORE_FILE}.tmp"));
        let backup = self.root.join(BACKUP_FILE);
        let mut contents = String::new();
        for item in self.items.values() {
            contents.push_str(&encode_item(item));
            contents.push('\n');
        }
        fs::write(&temporary, contents)
            .map_err(|error| format!("failed to stage training database: {error}"))?;
        if backup.exists() {
            fs::remove_file(&backup)
                .map_err(|error| format!("failed to clear training backup: {error}"))?;
        }
        if path.exists() {
            fs::rename(&path, &backup)
                .map_err(|error| format!("failed to stage training replacement: {error}"))?;
        }
        if let Err(error) = fs::rename(&temporary, &path) {
            if backup.exists() {
                let _ = fs::rename(&backup, &path);
            }
            return Err(format!("failed to commit training database: {error}"));
        }
        if backup.exists() {
            fs::remove_file(backup)
                .map_err(|error| format!("failed to finalize training database: {error}"))?;
        }
        Ok(())
    }
}

fn encode_item(item: &TrainingItem) -> String {
    let puzzle = &item.puzzle;
    let schedule = item.schedule;
    [
        escape(&puzzle.id),
        escape(&puzzle.fen),
        escape(&puzzle.solution.join(&LIST_SEPARATOR.to_string())),
        escape(&puzzle.themes.join(&LIST_SEPARATOR.to_string())),
        puzzle.rating.to_string(),
        schedule.repetitions.to_string(),
        schedule.interval_days.to_string(),
        schedule.ease_factor.to_string(),
        schedule.due_day.to_string(),
    ]
    .join("\t")
}

fn decode_item(row: &str) -> Result<TrainingItem, String> {
    let fields = row.split('\t').collect::<Vec<_>>();
    if fields.len() != 9 {
        return Err(format!("expected 9 fields, found {}", fields.len()));
    }
    let solution = unescape(fields[2])?;
    let themes = unescape(fields[3])?;
    Ok(TrainingItem {
        puzzle: Puzzle {
            id: unescape(fields[0])?,
            fen: unescape(fields[1])?,
            solution: split_values(&solution),
            themes: split_values(&themes),
            rating: parse_field(fields[4], "rating")?,
        },
        schedule: ReviewSchedule {
            repetitions: parse_field(fields[5], "repetitions")?,
            interval_days: parse_field(fields[6], "interval")?,
            ease_factor: parse_field(fields[7], "ease factor")?,
            due_day: parse_field(fields[8], "due day")?,
        },
    })
}

fn split_values(value: &str) -> Vec<String> {
    if value.is_empty() {
        Vec::new()
    } else {
        value.split(LIST_SEPARATOR).map(str::to_owned).collect()
    }
}

fn parse_field<T: std::str::FromStr>(value: &str, name: &str) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|error| format!("invalid {name} '{value}': {error}"))
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
        let code = bytes
            .get(index + 1..index + 3)
            .ok_or_else(|| "truncated escape".to_owned())?;
        let code = std::str::from_utf8(code).map_err(|error| error.to_string())?;
        output.push(u8::from_str_radix(code, 16).map_err(|error| error.to_string())?);
        index += 3;
    }
    String::from_utf8(output).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_store() -> PathBuf {
        std::env::temp_dir().join(format!(
            "mujrim-training-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn puzzle(id: &str, rating: u32) -> Puzzle {
        Puzzle {
            id: id.to_owned(),
            fen: crate::opening::START_FEN.to_owned(),
            solution: vec!["e2e4".to_owned(), "e7e5".to_owned()],
            themes: vec!["development".to_owned()],
            rating,
        }
    }

    #[test]
    fn puzzles_and_progress_survive_reopen() {
        let root = temporary_store();
        {
            let mut store = TrainingStore::open(&root).unwrap();
            assert!(store.add(puzzle("starter", 900)).unwrap());
            assert!(!store.add(puzzle("starter", 900)).unwrap());
            let schedule = store.review("starter", 5, 100).unwrap();
            assert_eq!(schedule.due_day, 101);
        }
        let store = TrainingStore::open(&root).unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(store.get("starter").unwrap().schedule.repetitions, 1);
        assert!(store.due(100, 10).is_empty());
        assert_eq!(store.due(101, 10)[0].puzzle.id, "starter");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn due_queue_prioritizes_overdue_then_rating() {
        let root = temporary_store();
        let mut store = TrainingStore::open(&root).unwrap();
        store.add(puzzle("easy", 800)).unwrap();
        store.add(puzzle("hard", 1800)).unwrap();
        let due = store.due(0, 1);
        assert_eq!(due[0].puzzle.id, "hard");
        assert!(store.review("missing", 5, 0).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_rows_are_rejected() {
        assert!(decode_item("too\tfew").is_err());
        assert!(unescape("bad%2").is_err());
    }

    #[test]
    fn interrupted_replacement_recovers_backup() {
        let root = temporary_store();
        {
            let mut store = TrainingStore::open(&root).unwrap();
            store.add(puzzle("recover", 1200)).unwrap();
        }
        fs::rename(root.join(STORE_FILE), root.join(BACKUP_FILE)).unwrap();
        let recovered = TrainingStore::open(&root).unwrap();
        assert!(recovered.get("recover").is_some());
        assert!(root.join(STORE_FILE).exists());
        assert!(!root.join(BACKUP_FILE).exists());
        fs::remove_dir_all(root).unwrap();
    }
}
