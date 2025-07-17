use crate::agent::code::executor::{CodeExecutor, execute_code_with_tools};
use crate::tools::BuiltInToolContext;
use crate::stores::memory::MemoryAgentStore;
use crate::agent::ExecutorContext;
use std::sync::Arc;

#[tokio::test]
async fn test_code_executor_basic() {
    let executor = CodeExecutor::default();
    assert!(executor.tools_context.is_none());
}

#[tokio::test] 
async fn test_execute_code_with_print() {
    // Create a mock context for testing
    let agent_store = Arc::new(MemoryAgentStore::new()) as Arc<dyn crate::stores::AgentStore>;
    let context = BuiltInToolContext {
        agent_id: "test_agent".to_string(),
        agent_store,
        context: Arc::new(ExecutorContext::default()),
        event_tx: None,
        coordinator_tx: tokio::sync::mpsc::channel(1).0,
        tool_sessions: None,
        registry: Arc::new(tokio::sync::RwLock::new(crate::servers::registry::McpServerRegistry::new())),
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
    let result = execute_code_with_tools(code, context).await.unwrap();
    
    assert!(result.contains("Hello, world!"));
}

#[tokio::test]
async fn test_execute_code_with_final_answer() {
    // Create a mock context for testing
    let agent_store = Arc::new(MemoryAgentStore::new()) as Arc<dyn crate::stores::AgentStore>;
    let context = BuiltInToolContext {
        agent_id: "test_agent".to_string(),
        agent_store,
        context: Arc::new(ExecutorContext::default()),
        event_tx: None,
        coordinator_tx: tokio::sync::mpsc::channel(1).0,
        tool_sessions: None,
        registry: Arc::new(tokio::sync::RwLock::new(crate::servers::registry::McpServerRegistry::new())),
    };

    // Register a test agent
    let agent_def = crate::types::AgentDefinition {
        name: "test_agent".to_string(),
        description: "Test agent".to_string(),
        ..Default::default()
    };
    context.agent_store.register(agent_def).await.unwrap();

    // Test code with final_answer
    let code = r#"final_answer("The answer is 42")"#;
    let result = execute_code_with_tools(code, context).await.unwrap();
    
    assert!(result.contains("The answer is 42"));
}
