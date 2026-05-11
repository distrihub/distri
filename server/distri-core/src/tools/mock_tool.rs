//! `MockTool` — synthetic tool used to populate test agents with a
//! large, realistic-looking universe of tools without hitting any
//! real APIs. Built so the deferred-loading + `tool_search` path can
//! be exercised end-to-end.
//!
//! Materialised by the `"mock"` `DynamicToolFactory` from a
//! [`MockFactoryConfig`](distri_types::mock_tool::MockFactoryConfig).
//! The agent author supplies description / parameters / response
//! inline in their `[[tools.dynamic]]` block; nothing about the mock
//! tool universe is hardcoded on the server. This keeps the test
//! variety where it belongs (in test fixtures) and the engine
//! agnostic to which scenarios any given test needs.
//!
//! On every invocation the tool returns its `response` verbatim as a
//! single `Part::Data`. Inputs are ignored — but the LLM doesn't know
//! that, so a well-shaped `parameters` schema still drives the model
//! to produce realistic-looking arguments.

use std::sync::Arc;

use distri_types::mock_tool::MockFactoryConfig;
use distri_types::{Part, Tool, ToolCall, ToolContext};
use serde_json::Value;

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::AgentError;

/// Build a `MockTool` from a typed config. The factory description on
/// `DynamicToolFactory` is allowed to override the inline description
/// — convenience for cases where the factory wrapper supplies the
/// human-readable label and the config carries the data shape.
pub fn build_mock_tool(
    name: String,
    cfg: MockFactoryConfig,
    factory_description: Option<String>,
) -> Arc<MockTool> {
    let description = factory_description.unwrap_or(cfg.description);
    Arc::new(MockTool {
        name,
        description,
        parameters: cfg.parameters,
        response: cfg.response,
    })
}

/// Materialised mock tool. Returns its canned `response` on every
/// call regardless of input.
#[derive(Debug)]
pub struct MockTool {
    name: String,
    description: String,
    parameters: Value,
    response: Value,
}

#[async_trait::async_trait]
impl Tool for MockTool {
    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn get_description(&self) -> String {
        self.description.clone()
    }

    fn get_parameters(&self) -> Value {
        self.parameters.clone()
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!("MockTool requires ExecutorContext"))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for MockTool {
    async fn execute_with_executor_context(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        Ok(vec![Part::Data(self.response.clone())])
    }
}
