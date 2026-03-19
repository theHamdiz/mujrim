use std::env;
use std::fs;
use std::path::PathBuf;

use crate::action::ToolAction;

#[derive(Debug)]
pub struct UninstallAction;

impl ToolAction for UninstallAction {
    fn run(&self) -> Result<(), String> {
        let home = env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/"));
        let paths = [
            home.join(".local/bin/kishmat"),
            home.join(".local/bin/kishmat-ui"),
            home.join(".local/bin/kishmat-updater"),
            home.join("Applications/KishMat.app"),
            home.join(".local/share/applications/kishmat.desktop"),
        ];

        for path in paths {
            if path.is_dir() {
                let _ = fs::remove_dir_all(&path);
            } else {
                let _ = fs::remove_file(&path);
            }
        }

        if let Ok(local) = env::var("LOCALAPPDATA") {
            let _ = fs::remove_dir_all(PathBuf::from(local).join("KishMat"));
        }

        println!("uninstall complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninstall_action_constructs() {
        let action = UninstallAction;
        let _ = format!("{action:?}");
    }
}
