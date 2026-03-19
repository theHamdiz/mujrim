//! Tunable parameter management — loads/saves params.toml.
//!
//! Each parameter has: name, current value, min, max, step.
//! The GUI can display sliders for each parameter and save changes.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A single tunable parameter with its range.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunableParam {
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

impl TunableParam {
    /// Value as integer (for display and setting).
    pub fn value_i32(&self) -> i32 {
        self.value as i32
    }
    pub fn min_i32(&self) -> i32 {
        self.min as i32
    }
    pub fn max_i32(&self) -> i32 {
        self.max as i32
    }
}

/// A group of tunable parameters (e.g., "null_move", "lmr").
pub type ParamGroup = BTreeMap<String, TunableParam>;

/// Full parameter set organized by section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunableParams {
    #[serde(default)]
    pub search: BTreeMap<String, ParamGroup>,
    #[serde(default)]
    pub history: ParamGroup,
    #[serde(default)]
    pub correction: ParamGroup,
    #[serde(default)]
    pub time: ParamGroup,
}

impl Default for TunableParams {
    fn default() -> Self {
        Self {
            search: BTreeMap::new(),
            history: BTreeMap::new(),
            correction: BTreeMap::new(),
            time: BTreeMap::new(),
        }
    }
}

impl TunableParams {
    /// Default path for the params.toml file.
    pub fn default_path() -> PathBuf {
        PathBuf::from("sprt").join("params.toml")
    }

    /// Load from a TOML file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        toml::from_str(&contents).map_err(|e| format!("Failed to parse {}: {e}", path.display()))
    }

    /// Save to a TOML file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory: {e}"))?;
        }
        let toml_str =
            toml::to_string_pretty(self).map_err(|e| format!("Failed to serialize: {e}"))?;
        std::fs::write(path, toml_str)
            .map_err(|e| format!("Failed to write {}: {e}", path.display()))
    }

    /// Get all parameters as a flat list of (section, name, param).
    pub fn flat_list(&self) -> Vec<(String, String, TunableParam)> {
        let mut result = Vec::new();
        for (group_name, group) in &self.search {
            for (param_name, param) in group {
                result.push((
                    format!("search.{group_name}"),
                    param_name.clone(),
                    param.clone(),
                ));
            }
        }
        for (name, param) in &self.history {
            result.push(("history".to_string(), name.clone(), param.clone()));
        }
        for (name, param) in &self.correction {
            result.push(("correction".to_string(), name.clone(), param.clone()));
        }
        for (name, param) in &self.time {
            result.push(("time".to_string(), name.clone(), param.clone()));
        }
        result
    }

    /// Update a parameter value by section and name.
    pub fn set_value(&mut self, section: &str, name: &str, value: f64) -> bool {
        if section.starts_with("search.") {
            let group_name = &section["search.".len()..];
            if let Some(group) = self.search.get_mut(group_name) {
                if let Some(param) = group.get_mut(name) {
                    param.value = value.clamp(param.min, param.max);
                    return true;
                }
            }
        } else if section == "history" {
            if let Some(param) = self.history.get_mut(name) {
                param.value = value.clamp(param.min, param.max);
                return true;
            }
        } else if section == "correction" {
            if let Some(param) = self.correction.get_mut(name) {
                param.value = value.clamp(param.min, param.max);
                return true;
            }
        } else if section == "time" {
            if let Some(param) = self.time.get_mut(name) {
                param.value = value.clamp(param.min, param.max);
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_params() {
        // Try loading the actual params.toml if it exists
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("sprt")
            .join("params.toml");
        if path.exists() {
            let params = TunableParams::load(&path).expect("Failed to load params.toml");
            let flat = params.flat_list();
            assert!(!flat.is_empty(), "Should have parsed some parameters");
            // Check that we got search.null_move.base_r
            assert!(
                flat.iter()
                    .any(|(s, n, _)| s == "search.null_move" && n == "base_r"),
                "Should have search.null_move.base_r"
            );
        }
    }

    #[test]
    fn test_set_value() {
        let mut params = TunableParams::default();
        let mut group = ParamGroup::new();
        group.insert(
            "test_param".to_string(),
            TunableParam {
                value: 10.0,
                min: 0.0,
                max: 100.0,
                step: 5.0,
            },
        );
        params.search.insert("test_group".to_string(), group);

        assert!(params.set_value("search.test_group", "test_param", 50.0));
        let flat = params.flat_list();
        let found = flat.iter().find(|(_, n, _)| n == "test_param").unwrap();
        assert_eq!(found.2.value, 50.0);
    }
}
