use distri_types::{AgentError, LlmDefinition, Message, Tool, ToolCallFormat};
use std::sync::Arc;

use crate::{agent::ExecutorContext, llm::LLMExecutor, tools::FinalTool};

#[tokio::test]
async fn test_request_using_provider_tool_format() {
    let executor = LLMExecutor::new(
        LlmDefinition {
            tool_format: ToolCallFormat::Provider,
            ..Default::default()
        },
        vec![Arc::new(FinalTool) as Arc<dyn Tool>],
        Arc::new(ExecutorContext::default()),
        None,
        None,
    );

    let message = Message::system(
        "You are  a specialist agent that says hello using final tool".to_string(),
        None,
    );

    let context = Arc::new(ExecutorContext::default());
    let response = executor.execute_stream(&[message], context).await;

    match response {
        Ok(response) => {
            println!("Response: {:?}", response);

            assert!(!response.content.is_empty());
        }
        Err(e) => {
            println!("Error: {:?}", e);
            match e {
                AgentError::OpenAIError(e) => {
                    println!("OpenAI error message: {:?}", e);
                }
                _ => {
                    println!("Other error: {:?}", e);
                }
            }
            assert!(false);
        }
    }
}
