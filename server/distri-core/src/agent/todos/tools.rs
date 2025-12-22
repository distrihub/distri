use std::sync::Arc;

use crate::{
    agent::ExecutorContext,
    tools::{ExecutorContextTool, ToolContext},
    types::ToolCall,
    AgentError,
};
use distri_stores::SessionStoreExt;
use distri_types::{AgentEventType, TodoList, TodoParams, Tool};
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct TodosTool;

#[async_trait::async_trait]
impl Tool for TodosTool {
    fn get_name(&self) -> String {
        "write_todos".to_string()
    }
    /// Get tool parameters schema - manual JSON schema for simple todo array
    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "Replace the current TODO list with these entries.",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "Short description of the TODO item."
                            },
                            "status": {
                                "type": "string",
                                "enum": ["open", "in_progress", "done"],
                                "description": "Optional status override. Defaults to 'open'."
                            }
                        },
                        "required": ["content"]
                    }
                }
            }
        })
    }

    /// Get tool description
    fn get_description(&self) -> String {
        "Manage TODOs with efficient bulk operations. Use write_todos for all modifications, list for display. Always keep and recite current TODOs in context."
            .to_string()
    }
    fn needs_executor_context(&self) -> bool {
        true // This tool needs ExecutorContext
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        // This should never be called since needs_executor_context() returns true
        Err(anyhow::anyhow!(
            "TodosTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for TodosTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<distri_types::Part>, AgentError> {
        // Parse the request using Serde
        let params: TodoParams = serde_json::from_value(tool_call.input.clone())
            .map_err(|e| AgentError::ToolExecution(format!("Invalid todo parameters: {}", e)))?;

        let task_id = context.parent_task_id.as_ref().unwrap_or(&context.task_id);

        let mut todo_list = TodoList::new();
        let simple_todos = params.todos.unwrap_or_default();
        todo_list.write_todos(simple_todos);

        let formatted_todos = todo_list.format_display();
        let todo_count = todo_list.items.len();

        context
            .get_session_store()?
            .set(task_id, "todos", &todo_list)
            .await
            .map_err(|e| AgentError::Session(format!("Failed to set todos in session: {}", e)))?;

        context
            .emit(AgentEventType::TodosUpdated {
                formatted_todos,
                action: if todo_count == 0 {
                    "clear".to_string()
                } else {
                    "write_todos".to_string()
                },
                todo_count,
            })
            .await;

        // Convert response to JSON and wrap in Part::Data
        Ok(vec![distri_types::Part::Data(Value::Null)])
    }
}
