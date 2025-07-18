use serde_json::Value;

use crate::agent::code::executor::execute_code_with_tools;
use crate::agent::code::tools::{FinalAnswerTool, PrintTool};
use crate::agent::ExecutorContext;
use crate::tools::{Tool, ToolContext};
use crate::InMemoryAgentStore;
use std::sync::Arc;

#[tokio::test]
async fn test_execute_code_with_print() {
    // Create a mock context for testing
    let agent_store = Arc::new(InMemoryAgentStore::new()) as Arc<dyn crate::stores::AgentStore>;
    let context = ToolContext {
        agent_id: "test_agent".to_string(),
        agent_store,
        context: Arc::new(ExecutorContext::default()),
        event_tx: None,
        coordinator_tx: tokio::sync::mpsc::channel(1).0,
        tool_sessions: None,
        registry: Arc::new(tokio::sync::RwLock::new(
            crate::servers::registry::McpServerRegistry::new(),
        )),
    };

    // Register a test agent
    let agent_def = crate::types::AgentDefinition {
        name: "test_agent".to_string(),
        description: "Test agent".to_string(),
        ..Default::default()
    };
    context.agent_store.register(agent_def).await.unwrap();

    // Test code with print statement
    let code = r#"print("Hello, world!")"#;
    let result = execute_code_with_tools(code, context, vec![Arc::new(PrintTool) as Arc<dyn Tool>])
        .await
        .unwrap();

    assert!(result.to_string().contains("Hello, world!"));
}

#[tokio::test]
async fn test_execute_code_with_final_answer() {
    // Create a mock context for testing
    let agent_store = Arc::new(InMemoryAgentStore::new()) as Arc<dyn crate::stores::AgentStore>;
    let context = ToolContext {
        agent_id: "test_agent".to_string(),
        agent_store,
        context: Arc::new(ExecutorContext::default()),
        event_tx: None,
        coordinator_tx: tokio::sync::mpsc::channel(1).0,
        tool_sessions: None,
        registry: Arc::new(tokio::sync::RwLock::new(
            crate::servers::registry::McpServerRegistry::new(),
        )),
    };

    // Register a test agent
    let agent_def = crate::types::AgentDefinition {
        name: "test_agent".to_string(),
        description: "Test agent".to_string(),
        ..Default::default()
    };
    context.agent_store.register(agent_def).await.unwrap();

    // Test code with final_answer
    let code = r#"final_answer(42)"#;
    let result = execute_code_with_tools(
        code,
        context,
        vec![Arc::new(FinalAnswerTool) as Arc<dyn Tool>],
    )
    .await
    .unwrap();

    assert_eq!(result, Value::Number(42.into()));
}
