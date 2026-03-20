use std::collections::BTreeMap;

use serde_json::Value;

use crate::agent_tools::tool::{AgentTool, AgentToolSpec};
use crate::agent_tools::{engine, gui, tooling, updater};

struct AgentToolEntry {
    spec: AgentToolSpec,
    tool: Box<dyn AgentTool>,
}

pub struct ToolRegistry {
    tools: BTreeMap<String, AgentToolEntry>,
}

impl ToolRegistry {
    pub fn with_defaults() -> Result<Self, String> {
        let mut registry = Self {
            tools: BTreeMap::new(),
        };

        for tool in engine::tools()
            .into_iter()
            .chain(gui::tools())
            .chain(tooling::tools())
            .chain(updater::tools())
        {
            registry.register(tool)?;
        }

        Ok(registry)
    }

    pub fn list(&self) -> Vec<AgentToolSpec> {
        self.tools
            .values()
            .map(|entry| entry.spec.clone())
            .collect()
    }

    pub fn describe(&self, name: &str) -> Option<AgentToolSpec> {
        self.tools.get(name).map(|entry| entry.spec.clone())
    }

    pub fn call(&self, name: &str, input: &Value) -> Result<Value, String> {
        if !input.is_object() {
            return Err("tool input must be a JSON object".to_string());
        }

        let entry = self
            .tools
            .get(name)
            .ok_or_else(|| format!("unknown tool '{name}'"))?;
        entry.tool.call(input)
    }

    fn register(&mut self, tool: Box<dyn AgentTool>) -> Result<(), String> {
        let spec = tool.spec();
        let key = spec.name.to_string();

        if self.tools.contains_key(&key) {
            return Err(format!("duplicate tool registration '{key}'"));
        }

        self.tools.insert(key, AgentToolEntry { spec, tool });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_contains_all_domains() {
        let registry = ToolRegistry::with_defaults().unwrap();
        let names: Vec<String> = registry
            .list()
            .into_iter()
            .map(|spec| spec.name.to_string())
            .collect();

        assert!(names.contains(&"engine.analyze".to_string()));
        assert!(names.contains(&"engine.perft".to_string()));
        assert!(names.contains(&"gui.settings.path".to_string()));
        assert!(names.contains(&"tooling.release_targets".to_string()));
        assert!(names.contains(&"updater.nnue.catalog".to_string()));
    }

    #[test]
    fn unknown_tool_returns_error() {
        let registry = ToolRegistry::with_defaults().unwrap();
        let err = registry
            .call("missing.tool", &serde_json::json!({}))
            .unwrap_err();
        assert!(err.contains("unknown tool"));
    }
}
