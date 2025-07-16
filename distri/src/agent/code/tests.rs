use std::sync::Arc;

use crate::agent::code::{CodeExecutor, FunctionDefinition};
use crate::agent::types::ExecutorContext;
use serde_json::Value;

#[tokio::test]
async fn test_echo_async() -> Result<(), Box<dyn std::error::Error>> {
    let context = Arc::new(ExecutorContext::default());
    let mut executor = CodeExecutor::new(context);
    
    // Add a simple echo function
    let echo_function = FunctionDefinition::new("echo".to_string())
        .with_description("Echo a message".to_string())
        .with_parameters(serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The message to echo"
                }
            },
            "required": ["message"]
        }))
        .with_returns("The echoed message".to_string());
    
    executor.add_function(echo_function);

    // Test code execution
    let code = r#"
        const result = "Hello, world!";
    "#;

    let result: Value = executor.execute(code).await?;

    assert_eq!(result, "Hello, world!");

    Ok(())
}

#[tokio::test]
async fn test_code_execution_with_context() -> Result<(), Box<dyn std::error::Error>> {
    let context = Arc::new(ExecutorContext::default());
    let executor = CodeExecutor::new(context);
    
    let mut context_map = std::collections::HashMap::new();
    context_map.insert("name".to_string(), Value::String("World".to_string()));
    context_map.insert("number".to_string(), Value::Number(42.into()));

    let code = r#"
        const result = {
            greeting: `Hello, ${context.name}!`,
            number: context.number * 2
        };
    "#;

    let result: Value = executor.execute_with_context(code, &context_map).await?;

    let expected = serde_json::json!({
        "greeting": "Hello, World!",
        "number": 84
    });

    assert_eq!(result, expected);

    Ok(())
}

#[tokio::test]
async fn test_code_validation() -> Result<(), Box<dyn std::error::Error>> {
    use crate::agent::code::CodeValidator;
    use crate::tools::{BuiltInToolContext, Tool, ToolCall};
    
    let validator = CodeValidator;
    
    // Test valid code
    let valid_code = r#"
        function hello() {
            return "Hello, World!";
        }
        const result = hello();
    "#;
    
    let valid_call = ToolCall {
        tool_id: "validate_1".to_string(),
        tool_name: "validate_code".to_string(),
        input: serde_json::to_string(&serde_json::json!({
            "code": valid_code
        }))?,
    };

    let context = BuiltInToolContext {
        agent_id: "test_agent".to_string(),
        agent_store: Arc::new(crate::stores::memory::InMemoryAgentStore::new()),
        context: Arc::new(ExecutorContext::default()),
        event_tx: None,
        coordinator_tx: tokio::sync::mpsc::channel(100).0,
        tool_sessions: None,
        registry: Arc::new(tokio::sync::RwLock::new(crate::servers::registry::McpServerRegistry::default())),
    };

    let result = validator.execute(valid_call, context).await?;
    let validation: Value = serde_json::from_str(&result)?;
    
    assert_eq!(validation["valid"], true);

    // Test invalid code (unmatched brace)
    let invalid_code = r#"
        function hello() {
            return "Hello, World!";
        // Missing closing brace
    "#;
    
    let invalid_call = ToolCall {
        tool_id: "validate_2".to_string(),
        tool_name: "validate_code".to_string(),
        input: serde_json::to_string(&serde_json::json!({
            "code": invalid_code
        }))?,
    };

    let context = BuiltInToolContext {
        agent_id: "test_agent".to_string(),
        agent_store: Arc::new(crate::stores::memory::InMemoryAgentStore::new()),
        context: Arc::new(ExecutorContext::default()),
        event_tx: None,
        coordinator_tx: tokio::sync::mpsc::channel(100).0,
        tool_sessions: None,
        registry: Arc::new(tokio::sync::RwLock::new(crate::servers::registry::McpServerRegistry::default())),
    };

    let result = validator.execute(invalid_call, context).await?;
    let validation: Value = serde_json::from_str(&result)?;
    
    assert_eq!(validation["valid"], false);
    assert!(!validation["errors"].as_array().unwrap().is_empty());

    Ok(())
}