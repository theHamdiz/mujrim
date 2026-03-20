use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct AgentToolSpec {
    pub name: &'static str,
    pub domain: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

pub trait AgentTool: Send + Sync {
    fn spec(&self) -> AgentToolSpec;
    fn call(&self, input: &Value) -> Result<Value, String>;
}
