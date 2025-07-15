use crate::{
    agent::{ExecutorContext, StepResult},
    coding::{executor::JsExecutor, js_agent::JsAgent, js_tools::JsToolRegistry},
    memory::TaskStep,
    tools::LlmToolsRegistry,
    types::AgentDefinition,
    SessionStore,
};
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::test]
async fn test_js_executor_basic() {
    // Create a simple tool registry
    let mut tools = HashMap::new();
    tools.insert(
        "hello".to_string(),
        Box::new(HelloTool) as Box<dyn crate::tools::Tool>,
    );

    let tool_registry = Arc::new(LlmToolsRegistry::new(tools));
    let js_tool_registry = Arc::new(JsToolRegistry::new(tool_registry.tools.clone()));

    // Create JavaScript executor
    let executor = JsExecutor::new(js_tool_registry).expect("Failed to create executor");

    // Test basic JavaScript execution
    let code = r#"
        console.log("Hello from JavaScript!");
        const result = "Hello World";
        setOutput(result);
    "#;

    let output = executor
        .execute(code)
        .await
        .expect("Failed to execute code");

    assert_eq!(output.output, "Hello World");
    assert!(!output.is_final_answer);
    assert!(output.logs.contains("Hello from JavaScript!"));
}

#[tokio::test]
async fn test_js_executor_final_answer() {
    // Create a simple tool registry
    let mut tools = HashMap::new();
    tools.insert(
        "hello".to_string(),
        Box::new(HelloTool) as Box<dyn crate::tools::Tool>,
    );

    let tool_registry = Arc::new(LlmToolsRegistry::new(tools));
    let js_tool_registry = Arc::new(JsToolRegistry::new(tool_registry.tools.clone()));

    // Create JavaScript executor
    let executor = JsExecutor::new(js_tool_registry).expect("Failed to create executor");

    // Test final answer
    let code = r#"
        console.log("Processing...");
        const answer = 42;
        finalAnswer(answer);
    "#;

    let output = executor
        .execute(code)
        .await
        .expect("Failed to execute code");

    assert_eq!(output.output, "42");
    assert!(output.is_final_answer);
    assert!(output.logs.contains("Processing..."));
}

#[tokio::test]
async fn test_js_executor_variables() {
    // Create a simple tool registry
    let mut tools = HashMap::new();
    tools.insert(
        "hello".to_string(),
        Box::new(HelloTool) as Box<dyn crate::tools::Tool>,
    );

    let tool_registry = Arc::new(LlmToolsRegistry::new(tools));
    let js_tool_registry = Arc::new(JsToolRegistry::new(tool_registry.tools.clone()));

    // Create JavaScript executor
    let executor = JsExecutor::new(js_tool_registry).expect("Failed to create executor");

    // Test variable persistence
    let code1 = r#"
        setVariable("name", "Alice");
        setVariable("age", 30);
        setOutput("Variables set");
    "#;

    let output1 = executor
        .execute(code1)
        .await
        .expect("Failed to execute code");
    assert_eq!(output1.output, "Variables set");

    let code2 = r#"
        const name = global_variables.name;
        const age = global_variables.age;
        setOutput(`Name: ${name}, Age: ${age}`);
    "#;

    let output2 = executor
        .execute(code2)
        .await
        .expect("Failed to execute code");
    assert_eq!(output2.output, "Name: Alice, Age: 30");
}

#[tokio::test]
async fn test_js_executor_error_handling() {
    // Create a simple tool registry
    let mut tools = HashMap::new();
    tools.insert(
        "hello".to_string(),
        Box::new(HelloTool) as Box<dyn crate::tools::Tool>,
    );

    let tool_registry = Arc::new(LlmToolsRegistry::new(tools));
    let js_tool_registry = Arc::new(JsToolRegistry::new(tool_registry.tools.clone()));

    // Create JavaScript executor
    let executor = JsExecutor::new(js_tool_registry).expect("Failed to create executor");

    // Test error handling
    let code = r#"
        try {
            const result = undefined.someMethod();
        } catch (error) {
            console.error("Caught error:", error.message);
            setOutput("Error handled gracefully");
        }
    "#;

    let output = executor
        .execute(code)
        .await
        .expect("Failed to execute code");

    assert_eq!(output.output, "Error handled gracefully");
    assert!(output.logs.contains("Caught error:"));
}

// Simple test tool
struct HelloTool;

#[async_trait::async_trait]
impl crate::tools::Tool for HelloTool {
    fn get_name(&self) -> String {
        "hello".to_string()
    }

    fn get_description(&self) -> String {
        "A simple hello tool".to_string()
    }

    fn get_tool_definition(&self) -> async_openai::types::ChatCompletionTool {
        async_openai::types::ChatCompletionTool {
            r#type: async_openai::types::ChatCompletionToolType::Function,
            function: async_openai::types::FunctionObject {
                name: "hello".to_string(),
                description: Some("A simple hello tool".to_string()),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Name to greet"
                        }
                    },
                    "required": ["name"]
                })),
                strict: None,
            },
        }
    }

    async fn execute(
        &self,
        _tool_call: crate::types::ToolCall,
        _context: crate::tools::BuiltInToolContext,
    ) -> Result<String, crate::error::AgentError> {
        Ok("Hello from tool!".to_string())
    }

    fn clone_box(&self) -> Box<dyn crate::tools::Tool> {
        Box::new(Self)
    }
}

#[tokio::test]
async fn test_js_agent_creation() {
    // Create agent definition
    let definition = AgentDefinition {
        name: "test_js_agent".to_string(),
        description: "Test JavaScript agent".to_string(),
        agent_type: Some("js_agent".to_string()),
        system_prompt: Some("You are a JavaScript coding agent.".to_string()),
        model_settings: crate::types::ModelSettings::default(),
        mcp_servers: vec![],
        history_size: Some(10),
        plan: None,
        icon_url: None,
        max_iterations: Some(5),
        skills: vec![],
        sub_agents: vec![],
        version: None,
    };

    // Create tool registry
    let mut tools = HashMap::new();
    tools.insert(
        "hello".to_string(),
        Box::new(HelloTool) as Box<dyn crate::tools::Tool>,
    );
    let tool_registry = Arc::new(LlmToolsRegistry::new(tools));

    // Create session store
    let session_store = Arc::new(Box::new(MockSessionStore) as Box<dyn SessionStore>);

    // Create context
    let context = Arc::new(ExecutorContext::new(
        "test_thread".to_string(),
        Some("test_run".to_string()),
        true,
        None,
        None,
        None,
    ));

    // Create JavaScript agent
    let agent = JsAgent::new(definition, tool_registry, session_store, context)
        .expect("Failed to create JsAgent");

    assert_eq!(agent.get_name(), "test_js_agent");
    assert_eq!(agent.get_description(), "Test JavaScript agent");
}
