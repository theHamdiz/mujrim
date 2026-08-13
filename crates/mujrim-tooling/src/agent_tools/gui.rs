use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::agent_tools::tool::{AgentTool, AgentToolSpec};

pub fn tools() -> Vec<Box<dyn AgentTool>> {
    vec![
        Box::new(GuiSettingsPathTool),
        Box::new(GuiSettingsReadTool),
        Box::new(GuiPieceSetsListTool),
    ]
}

struct GuiSettingsPathTool;

impl AgentTool for GuiSettingsPathTool {
    fn spec(&self) -> AgentToolSpec {
        AgentToolSpec {
            name: "gui.settings.path",
            domain: "gui",
            description: "Return the GUI settings TOML path.",
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    fn call(&self, _input: &Value) -> Result<Value, String> {
        let path = default_settings_path();
        Ok(json!({ "path": path.display().to_string() }))
    }
}

struct GuiSettingsReadTool;

impl AgentTool for GuiSettingsReadTool {
    fn spec(&self) -> AgentToolSpec {
        AgentToolSpec {
            name: "gui.settings.read",
            domain: "gui",
            description: "Read the GUI settings TOML file.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "additionalProperties": false
            }),
        }
    }

    fn call(&self, input: &Value) -> Result<Value, String> {
        let path = optional_path(input, "path")?.unwrap_or_else(default_settings_path);
        let exists = path.exists();
        let contents = if exists {
            Some(
                fs::read_to_string(&path)
                    .map_err(|e| format!("failed to read {}: {e}", path.display()))?,
            )
        } else {
            None
        };

        Ok(json!({
            "path": path.display().to_string(),
            "exists": exists,
            "contents": contents
        }))
    }
}

struct GuiPieceSetsListTool;

impl AgentTool for GuiPieceSetsListTool {
    fn spec(&self) -> AgentToolSpec {
        AgentToolSpec {
            name: "gui.piece_sets.list",
            domain: "gui",
            description: "List available GUI piece sets under assets/pieces.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pieces_root": { "type": "string" }
                },
                "additionalProperties": false
            }),
        }
    }

    fn call(&self, input: &Value) -> Result<Value, String> {
        let pieces_root = optional_path(input, "pieces_root")?
            .unwrap_or_else(|| workspace_root().join("crates/mujrim-ui/assets/pieces"));
        let sets = collect_piece_sets(&pieces_root)?;
        Ok(json!({
            "pieces_root": pieces_root.display().to_string(),
            "piece_sets": sets
        }))
    }
}

fn default_settings_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("mujrim");
    path.push("settings.toml");
    path
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn optional_path(input: &Value, key: &str) -> Result<Option<PathBuf>, String> {
    match input.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(PathBuf::from)
            .map(Some)
            .ok_or_else(|| format!("'{key}' must be a string path")),
    }
}

fn collect_piece_sets(pieces_root: &Path) -> Result<Vec<String>, String> {
    if !pieces_root.exists() {
        return Ok(Vec::new());
    }

    let mut sets = Vec::new();
    let entries = fs::read_dir(pieces_root)
        .map_err(|e| format!("failed to read {}: {e}", pieces_root.display()))?;

    let has_flat_set = directory_has_complete_piece_set(pieces_root);
    for entry in entries {
        let entry = entry.map_err(|e| format!("failed to read directory entry: {e}"))?;
        let path = entry.path();
        if path.is_dir() && directory_has_complete_piece_set(&path) {
            sets.push(entry.file_name().to_string_lossy().to_string());
        }
    }

    if has_flat_set {
        sets.push("default".to_string());
    }

    sets.sort_unstable();
    sets.dedup();
    Ok(sets)
}

const PIECE_NAMES: [&str; 12] = [
    "wK", "wQ", "wR", "wB", "wN", "wP", "bK", "bQ", "bR", "bB", "bN", "bP",
];

fn directory_has_complete_piece_set(dir: &Path) -> bool {
    PIECE_NAMES.iter().all(|name| {
        ["svg", "png"]
            .iter()
            .any(|extension| dir.join(format!("{name}.{extension}")).is_file())
    })
}

#[cfg(test)]
fn create_test_piece_set(dir: &Path, extension: &str) {
    fs::create_dir_all(dir).unwrap();
    for piece in PIECE_NAMES {
        fs::write(dir.join(format!("{piece}.{extension}")), b"asset").unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn settings_path_looks_like_mujrim_config_file() {
        let path = default_settings_path();
        let text = path.to_string_lossy();
        assert!(text.contains("mujrim"));
        assert!(text.ends_with("settings.toml"));
    }

    #[test]
    fn collect_piece_sets_detects_default_and_named_sets() {
        let base = unique_temp_dir("piece-sets");
        create_test_piece_set(&base, "png");

        let alpha = base.join("alpha");
        create_test_piece_set(&alpha, "svg");

        let empty = base.join("empty");
        fs::create_dir_all(&empty).unwrap();

        let sets = collect_piece_sets(&base).unwrap();
        assert!(sets.contains(&"default".to_string()));
        assert!(sets.contains(&"alpha".to_string()));
        assert!(!sets.contains(&"empty".to_string()));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn piece_set_tool_allows_custom_root() {
        let base = unique_temp_dir("piece-tool");
        create_test_piece_set(&base, "png");

        let tool = GuiPieceSetsListTool;
        let out = tool
            .call(&json!({ "pieces_root": base.display().to_string() }))
            .unwrap();
        assert_eq!(out["piece_sets"][0], "default");

        fs::remove_dir_all(base).ok();
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("mujrim-tooling-{label}-{now}"))
    }
}
