//! The tools a run may call, by name — dispatch over the [`Tool`] port.
//!
//! Which tools are in it is wiring (`hexa_exec::default_tools`); this only
//! looks them up, describes them to the model, and runs them.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;

use crate::ports::{Tool, ToolResult};

pub struct ToolRegistry {
    tools: HashMap<&'static str, Arc<dyn Tool>>,
}

// No `Default`: `ToolRegistry::default()` used to mean "every tool". An empty
// Default would keep compiling at an old call site and hand the model no tools
// — the silent failure a direct error avoids. The full set is
// `hexa_exec::default_tools()`.
#[allow(clippy::new_without_default)]
impl ToolRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name(), tool);
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.tools.keys().copied().collect()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Build the `tools` array Anthropic expects in a /messages request.
    /// Each entry: { name, description, input_schema }.
    pub fn anthropic_schema(&self) -> Value {
        let arr: Vec<Value> = self
            .tools
            .values()
            .map(|t| {
                serde_json::json!({
                    "name": t.name(),
                    "description": t.description(),
                    "input_schema": t.input_schema(),
                })
            })
            .collect();
        Value::Array(arr)
    }

    pub async fn execute(&self, name: &str, input: Value) -> ToolResult {
        let start = Instant::now();
        let tool = match self.get(name) {
            Some(t) => t,
            None => {
                return ToolResult::err(
                    format!("unknown tool: {}", name),
                    start.elapsed().as_millis() as u64,
                );
            }
        };
        tool.execute(input).await
    }
}
