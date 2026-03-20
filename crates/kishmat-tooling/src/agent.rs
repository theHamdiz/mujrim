use clap::Subcommand;
use serde_json::{Value, json};

use crate::action::ToolAction;
use crate::agent_tools::registry::ToolRegistry;

#[derive(Subcommand, Debug)]
pub enum AgentCommand {
    /// List available agent tools.
    List {
        #[arg(long)]
        pretty: bool,
    },
    /// Show one tool specification.
    Describe {
        tool: String,
        #[arg(long)]
        pretty: bool,
    },
    /// Call a tool with JSON input.
    Call {
        tool: String,
        #[arg(long, default_value = "{}")]
        input: String,
        #[arg(long)]
        pretty: bool,
    },
}

#[derive(Debug)]
pub struct AgentAction {
    pub command: AgentCommand,
}

impl ToolAction for AgentAction {
    fn run(&self) -> Result<(), String> {
        let registry = ToolRegistry::with_defaults()?;

        match &self.command {
            AgentCommand::List { pretty } => {
                let payload = json!({ "tools": registry.list() });
                print_json(&payload, *pretty)
            }
            AgentCommand::Describe { tool, pretty } => {
                let spec = registry
                    .describe(tool)
                    .ok_or_else(|| format!("unknown tool '{tool}'"))?;
                let payload = json!({ "tool": spec });
                print_json(&payload, *pretty)
            }
            AgentCommand::Call {
                tool,
                input,
                pretty,
            } => {
                let input_json = parse_input_json(input)?;
                let result = registry.call(tool, &input_json)?;
                let payload = json!({
                    "tool": tool,
                    "result": result
                });
                print_json(&payload, *pretty)
            }
        }
    }
}

fn parse_input_json(raw: &str) -> Result<Value, String> {
    let input: Value =
        serde_json::from_str(raw).map_err(|e| format!("invalid JSON in --input: {e}"))?;
    if !input.is_object() {
        return Err("tool input must be a JSON object".to_string());
    }
    Ok(input)
}

fn print_json(value: &Value, pretty: bool) -> Result<(), String> {
    let encoded = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
    .map_err(|e| format!("failed to encode JSON output: {e}"))?;
    println!("{encoded}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_input_rejects_non_object_json() {
        let err = parse_input_json("[1,2,3]").unwrap_err();
        assert!(err.contains("JSON object"));
    }

    #[test]
    fn parse_input_accepts_object_json() {
        let value = parse_input_json("{\"depth\":2}").unwrap();
        assert_eq!(value["depth"], 2);
    }
}
