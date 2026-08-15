//! Crash-safe file commits: stage, fsync, then rename over the destination.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

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
    if backup.exists() {
        fs::remove_file(&backup)
            .map_err(|error| format!("failed to clear backup {}: {error}", backup.display()))?;
    }
    if path.exists() {
        fs::rename(path, &backup)
            .map_err(|error| format!("failed to stage replacement {}: {error}", path.display()))?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        return Err(format!("failed to commit {}: {error}", path.display()));
    }
    if backup.exists() {
        let _ = fs::remove_file(backup);
    }
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
    let _ = fs::remove_file(staging_path(path));
    let _ = fs::remove_file(backup_path(path));
}

pub fn sidecar_path(output: &Path, suffix: &str) -> PathBuf {
    let name = output
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "job".to_owned());
    output.with_file_name(format!("{name}{suffix}"))
}

fn staging_path(path: &Path) -> PathBuf {
    sidecar_path(path, ".tmp")
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
}
