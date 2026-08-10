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
            home.join(".local/bin/mujrim"),
            home.join(".local/bin/mujrim-ui"),
            home.join(".local/bin/mujrim-updater"),
            home.join("Applications/Mujrim.app"),
            home.join(".local/share/applications/mujrim.desktop"),
        ];

        for path in paths {
            if path.is_dir() {
                let _ = fs::remove_dir_all(&path);
            } else {
                let _ = fs::remove_file(&path);
            }
        }

        if let Ok(local) = env::var("LOCALAPPDATA") {
            let _ = fs::remove_dir_all(PathBuf::from(local).join("Mujrim"));
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
