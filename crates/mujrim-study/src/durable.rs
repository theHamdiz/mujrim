//! Crash-safe file commits: stage, fsync, then rename over the destination.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static STAGE_TICK: AtomicU64 = AtomicU64::new(0);

pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let temporary = staging_path(path);
    let backup = backup_path(path);
    {
        let mut file = File::create(&temporary)
            .map_err(|error| format!("failed to stage {}: {error}", temporary.display()))?;
        file.write_all(contents)
            .map_err(|error| format!("failed to write {}: {error}", temporary.display()))?;
        file.sync_all()
            .map_err(|error| format!("failed to sync {}: {error}", temporary.display()))?;
    }
    let _ = fs::remove_file(&backup);
    if path.exists() {
        let _ = fs::rename(path, &backup);
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        if backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        return Err(format!("failed to commit {}: {error}", path.display()));
    }
    let _ = fs::remove_file(backup);
    Ok(())
}

pub fn atomic_write_text(path: &Path, contents: &str) -> Result<(), String> {
    atomic_write(path, contents.as_bytes())
}

pub fn read_text(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

pub fn remove_file(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(backup_path(path));
    remove_staging_files(path);
}

pub fn sidecar_path(output: &Path, suffix: &str) -> PathBuf {
    let name = output
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "job".to_owned());
    output.with_file_name(format!("{name}{suffix}"))
}

fn staging_path(path: &Path) -> PathBuf {
    let tick = STAGE_TICK.fetch_add(1, Ordering::Relaxed);
    sidecar_path(path, &format!(".tmp.{tick}"))
}

fn remove_staging_files(path: &Path) {
    let Some(name) = path.file_name() else {
        return;
    };
    let prefix = format!("{}.tmp.", name.to_string_lossy());
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn backup_path(path: &Path) -> PathBuf {
    sidecar_path(path, ".bak")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "mujrim-durable-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ))
    }

    #[test]
    fn atomic_write_replaces_the_destination_and_clears_staging() {
        let path = unique_path("replace");
        atomic_write_text(&path, "first").expect("write first");
        atomic_write_text(&path, "second").expect("write second");
        assert_eq!(read_text(&path).as_deref(), Some("second"));
        assert!(!staging_path(&path).exists());
        assert!(!backup_path(&path).exists());
        remove_file(&path);
    }

    #[test]
    fn sidecar_path_keeps_the_original_file_name() {
        assert_eq!(
            sidecar_path(Path::new("ateed_default.bin"), ".job"),
            PathBuf::from("ateed_default.bin.job")
        );
    }

    #[test]
    fn atomic_write_survives_parallel_commits() {
        let path = unique_path("parallel");
        std::thread::scope(|scope| {
            for index in 0..32 {
                let path = path.clone();
                scope.spawn(move || {
                    atomic_write_text(&path, &format!("w{index}")).expect("parallel commit");
                });
            }
        });
        let text = read_text(&path).expect("final sidecar");
        assert!(text.starts_with('w'), "{text}");
        remove_file(&path);
    }
}
