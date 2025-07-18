use serde_json::Value;

use crate::agent::code::executor::execute_code_with_tools;
use crate::agent::code::tools::{CodeResponse, ConsoleLogTool, FinalAnswerTool};
use crate::agent::ExecutorContext;
use crate::tools::{Tool, ToolContext};
use crate::InMemoryAgentStore;
use std::sync::Arc;

#[tokio::test]
async fn test_execute_code_with_final_answer() {
    // Create a mock context for testing
    let agent_store = Arc::new(InMemoryAgentStore::new()) as Arc<dyn crate::stores::AgentStore>;
    let context = ToolContext {
        agent_id: "test_agent".to_string(),
        agent_store,
        memory_store: None,
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
    let code = r#" final_answer(42)"#;
    let (tx, rx) = crossbeam_channel::bounded::<CodeResponse>(100);
    let result = execute_code_with_tools(
        code,
        context,
        vec![Arc::new(FinalAnswerTool(tx)) as Arc<dyn Tool>],
    )
    .await
    .unwrap();

    // The function should return null, but we should receive the value through the channel
    assert_eq!(result, Value::Null);

    // Check that we received the final answer through the channel
    let response = rx.recv().unwrap();
    match response {
        CodeResponse::FinalAnswer(value) => {
            assert_eq!(value, Value::Number(42.into()));
        }
        _ => panic!("Expected FinalAnswer response"),
    }
}

#[tokio::test]
async fn test_execute_code_with_console_log() {
    tracing_subscriber::fmt::init();
    // Create a mock context for testing
    let agent_store = Arc::new(InMemoryAgentStore::new()) as Arc<dyn crate::stores::AgentStore>;
    let context = ToolContext {
        agent_id: "test_agent".to_string(),
        agent_store,
        memory_store: None,
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

    // Test code with console.log
    let code = r#"
        console.log("Hello, world!");
        console.log("Test observation");
        final_answer("Success");
    "#;
    let (tx, rx) = crossbeam_channel::bounded::<CodeResponse>(100);
    let result = execute_code_with_tools(
        code,
        context,
        vec![
            Arc::new(ConsoleLogTool(tx.clone())) as Arc<dyn Tool>,
            Arc::new(FinalAnswerTool(tx)) as Arc<dyn Tool>,
        ],
    )
    .await
    .unwrap();

    // The function should return null, but we should receive messages through the channel
    assert_eq!(result, Value::Null);

    let mut msgs = Vec::new();

    // Collect all messages from the channel
    while let Ok(response) = rx.recv() {
        msgs.push(response);
    }

    println!("msgs: {:?}", msgs);
    assert!(msgs[0].as_value().to_string().contains("Hello, world!"));
    assert!(msgs[1].as_value().to_string().contains("Test observation"));
    assert_eq!(
        msgs[2].as_value().clone(),
        Value::String("Success".to_string())
    );
}
