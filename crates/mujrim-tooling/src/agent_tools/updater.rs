use std::path::PathBuf;

use serde_json::{Value, json};

use crate::agent_tools::tool::{AgentTool, AgentToolSpec};
use mujrim_updater::{nnue, tuning};

pub fn tools() -> Vec<Box<dyn AgentTool>> {
    vec![
        Box::new(NnueCatalogTool),
        Box::new(NnueStatusTool),
        Box::new(TuningReadTool),
    ]
}

struct NnueCatalogTool;

impl AgentTool for NnueCatalogTool {
    fn spec(&self) -> AgentToolSpec {
        AgentToolSpec {
            name: "updater.nnue.catalog",
            domain: "updater",
            description: "Return the built-in NNUE network catalog from the updater crate.",
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    fn call(&self, _input: &Value) -> Result<Value, String> {
        let nets: Vec<Value> = nnue::NETWORKS
            .iter()
            .map(|net| {
                json!({
                    "id": net.id,
                    "name": net.name,
                    "engine": net.engine,
                    "architecture": net.architecture,
                    "filename": net.filename,
                    "upstream_name": net.upstream_name,
                    "approx_size": net.approx_size,
                    "search_preset": net.search_preset,
                    "elo": net.elo
                })
            })
            .collect();

        Ok(json!({ "networks": nets }))
    }
}

struct NnueStatusTool;

impl AgentTool for NnueStatusTool {
    fn spec(&self) -> AgentToolSpec {
        AgentToolSpec {
            name: "updater.nnue.status",
            domain: "updater",
            description: "Return installation and update status for NNUE files in a directory.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "dir": { "type": "string" }
                },
                "additionalProperties": false
            }),
        }
    }

    fn call(&self, input: &Value) -> Result<Value, String> {
        let dir = optional_path(input, "dir")?.unwrap_or_else(nnue::resources_dir);
        let statuses = nnue::check_installed(&dir);

        let mut counts = NnueCounts::default();
        let networks: Vec<Value> = statuses
            .into_iter()
            .map(|(net, status)| {
                counts.bump(status);
                json!({
                    "id": net.id,
                    "filename": net.filename,
                    "status": status_label(status)
                })
            })
            .collect();

        let files: Vec<Value> = nnue::list_network_files(&dir)
            .into_iter()
            .map(|(name, size)| json!({ "name": name, "size": size }))
            .collect();

        Ok(json!({
            "dir": dir.display().to_string(),
            "counts": {
                "current": counts.current,
                "missing": counts.missing,
                "update_available": counts.update_available
            },
            "disk_usage_bytes": nnue::disk_usage(&dir),
            "networks": networks,
            "files": files
        }))
    }
}

struct TuningReadTool;

impl AgentTool for TuningReadTool {
    fn spec(&self) -> AgentToolSpec {
        AgentToolSpec {
            name: "updater.tuning.read",
            domain: "updater",
            description: "Read updater tuning parameters from a TOML file.",
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
        let path =
            optional_path(input, "path")?.unwrap_or_else(tuning::TunableParams::default_path);
        if !path.exists() {
            return Ok(json!({
                "path": path.display().to_string(),
                "exists": false,
                "params": serde_json::Value::Null
            }));
        }

        let params = tuning::TunableParams::load(&path)?;
        let params_json =
            serde_json::to_value(params).map_err(|e| format!("failed to encode params: {e}"))?;
        Ok(json!({
            "path": path.display().to_string(),
            "exists": true,
            "params": params_json
        }))
    }
}

#[derive(Default)]
struct NnueCounts {
    current: usize,
    missing: usize,
    update_available: usize,
}

impl NnueCounts {
    fn bump(&mut self, status: nnue::NetStatus) {
        match status {
            nnue::NetStatus::Current => self.current += 1,
            nnue::NetStatus::Missing => self.missing += 1,
            nnue::NetStatus::UpdateAvailable => self.update_available += 1,
        }
    }
}

fn status_label(status: nnue::NetStatus) -> &'static str {
    match status {
        nnue::NetStatus::Current => "current",
        nnue::NetStatus::Missing => "missing",
        nnue::NetStatus::UpdateAvailable => "update_available",
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn catalog_contains_networks() {
        let tool = NnueCatalogTool;
        let out = tool.call(&json!({})).unwrap();
        let networks = out["networks"].as_array().unwrap();
        assert!(!networks.is_empty());
        assert!(networks[0]["id"].is_string());
    }

    #[test]
    fn status_reports_missing_for_empty_directory() {
        let root = unique_temp_dir("nnue-status");
        fs::create_dir_all(&root).unwrap();

        let tool = NnueStatusTool;
        let out = tool
            .call(&json!({ "dir": root.display().to_string() }))
            .unwrap();
        let missing = out["counts"]["missing"].as_u64().unwrap();
        assert_eq!(missing as usize, nnue::NETWORKS.len());

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn tuning_read_handles_missing_file() {
        let root = unique_temp_dir("tuning-read");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("missing.toml");

        let tool = TuningReadTool;
        let out = tool
            .call(&json!({ "path": path.display().to_string() }))
            .unwrap();
        assert_eq!(out["exists"], false);

        fs::remove_dir_all(root).ok();
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("mujrim-tooling-{label}-{now}"))
    }
}
