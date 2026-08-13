use clap::ValueEnum;
use serde_json::{Value, json};

use crate::agent_tools::tool::{AgentTool, AgentToolSpec};
use crate::build_variant::BuildVariant;
use crate::release::ReleaseTarget;

pub fn tools() -> Vec<Box<dyn AgentTool>> {
    vec![Box::new(BuildVariantsTool), Box::new(ReleaseTargetsTool)]
}

struct BuildVariantsTool;

impl AgentTool for BuildVariantsTool {
    fn spec(&self) -> AgentToolSpec {
        AgentToolSpec {
            name: "tooling.build_variants",
            domain: "tooling",
            description: "List supported build variants for mujrim-tooling build-variant.",
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    fn call(&self, _input: &Value) -> Result<Value, String> {
        let variants = value_enum_names::<BuildVariant>();
        Ok(json!({
            "variants": variants,
            "default": "full"
        }))
    }
}

struct ReleaseTargetsTool;

impl AgentTool for ReleaseTargetsTool {
    fn spec(&self) -> AgentToolSpec {
        AgentToolSpec {
            name: "tooling.release_targets",
            domain: "tooling",
            description: "List supported release targets for mujrim-tooling release.",
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    fn call(&self, _input: &Value) -> Result<Value, String> {
        let targets = value_enum_names::<ReleaseTarget>();
        Ok(json!({
            "targets": targets,
            "default": "native"
        }))
    }
}

fn value_enum_names<T: ValueEnum>() -> Vec<String> {
    T::value_variants()
        .iter()
        .filter_map(|variant| variant.to_possible_value())
        .map(|value| value.get_name().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_variant_tool_contains_expected_values() {
        let tool = BuildVariantsTool;
        let out = tool.call(&json!({})).unwrap();
        let variants = out["variants"].as_array().unwrap();
        assert!(variants.contains(&json!("full")));
        assert!(variants.contains(&json!("minimal")));
    }

    #[test]
    fn release_target_tool_contains_expected_values() {
        let tool = ReleaseTargetsTool;
        let out = tool.call(&json!({})).unwrap();
        let targets = out["targets"].as_array().unwrap();
        assert!(targets.contains(&json!("native")));
        assert!(targets.contains(&json!("full")));
    }
}
