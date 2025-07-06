use std::sync::Arc;

use crate::{
    agent::{BaseAgent, FilteringAgent, LoggingAgent, StandardAgent},
    memory::TaskStep,
    tests::utils::init_executor,
    tools::Tool,
    types::{AgentDefinition, ModelSettings},
};
use anyhow::Result;
use tokio::sync::Mutex;
use tracing::info;

#[tokio::test]
async fn test_agent_creation_and_metadata() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let agent_def = AgentDefinition {
        name: "test_logging_agent".to_string(),
        description: "A test logging agent".to_string(),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        plan: None,
        icon_url: None,
        max_iterations: Some(3),
        sub_agents: vec![],
        skills: vec![],
        version: None,
    };

    let executor = init_executor().await;
    let session_store = executor.session_store.clone();

    // Create a LoggingAgent
    let logging_agent = LoggingAgent::new(
        agent_def.clone(),
        Arc::default(),
        executor.clone(),
        Arc::new(crate::agent::ExecutorContext::default()),
        session_store.clone(),
    );

    // Test that the agent has the correct metadata
    assert_eq!(logging_agent.get_name(), "test_logging_agent");
    assert_eq!(logging_agent.get_description(), "A test logging agent");
    assert_eq!(logging_agent.get_definition().name, "test_logging_agent");

    info!("✅ LoggingAgent created successfully with correct metadata");

    // Create a FilteringAgent
    let filtering_agent = FilteringAgent::new(
        agent_def.clone(),
        Arc::default(),
        executor.clone(),
        Arc::new(crate::agent::ExecutorContext::default()),
        session_store.clone(),
        vec!["badword".to_string(), "inappropriate".to_string()],
    );

    // Test that the filtering agent has the correct metadata
    assert_eq!(filtering_agent.get_name(), "test_logging_agent");
    assert_eq!(filtering_agent.get_description(), "A test logging agent");

    info!("✅ FilteringAgent created successfully with correct metadata");

    Ok(())
}

#[tokio::test]
async fn test_standard_agent_hook_mechanism() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Custom agent that tracks hook calls without making LLM calls
    #[derive(Clone)]
    struct MockHookTrackingAgent {
        inner: StandardAgent,
        hook_calls: Arc<Mutex<Vec<String>>>,
    }

    impl std::fmt::Debug for MockHookTrackingAgent {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MockHookTrackingAgent").finish()
        }
    }

    #[async_trait::async_trait]
    impl BaseAgent for MockHookTrackingAgent {
        fn get_definition(&self) -> crate::types::AgentDefinition {
            self.inner.get_definition()
        }

        fn get_description(&self) -> &str {
            self.inner.get_description()
        }

        fn get_tools(&self) -> Vec<&Box<dyn Tool>> {
            self.inner.get_tools()
        }

        fn get_name(&self) -> &str {
            self.inner.get_name()
        }

        fn clone_box(&self) -> Box<dyn BaseAgent> {
            Box::new(self.clone())
        }

        // Don't implement invoke to avoid LLM calls - just test hooks
        async fn invoke(
            &self,
            _task: TaskStep,
            _params: Option<serde_json::Value>,
            _context: Arc<crate::agent::ExecutorContext>,
            _event_tx: Option<tokio::sync::mpsc::Sender<crate::agent::AgentEvent>>,
        ) -> Result<String, crate::error::AgentError> {
            // Mock implementation that just calls hooks without LLM
            Ok("Mock response".to_string())
        }

        async fn invoke_stream(
            &self,
            _task: TaskStep,
            _params: Option<serde_json::Value>,
            _context: Arc<crate::agent::ExecutorContext>,
            _event_tx: tokio::sync::mpsc::Sender<crate::agent::AgentEvent>,
        ) -> Result<(), crate::error::AgentError> {
            Ok(())
        }

        async fn after_task_step(
            &self,
            task: TaskStep,
            context: Arc<crate::agent::ExecutorContext>,
        ) -> Result<(), crate::error::AgentError> {
            self.hook_calls
                .lock()
                .await
                .push("after_task_step".to_string());
            info!("🎯 after_task_step called for task: {}", task.task);
            self.inner.after_task_step(task, context).await
        }

        async fn before_tool_calls(
            &self,
            tool_calls: &[crate::types::ToolCall],
            context: Arc<crate::agent::ExecutorContext>,
        ) -> Result<Vec<crate::types::ToolCall>, crate::error::AgentError> {
            self.hook_calls
                .lock()
                .await
                .push("before_tool_calls".to_string());
            info!(
                "🎯 before_tool_calls called with {} tool calls",
                tool_calls.len()
            );
            self.inner.before_tool_calls(tool_calls, context).await
        }

        async fn after_tool_calls(
            &self,
            tool_responses: &[String],
            context: Arc<crate::agent::ExecutorContext>,
        ) -> Result<(), crate::error::AgentError> {
            self.hook_calls
                .lock()
                .await
                .push("after_tool_calls".to_string());
            info!(
                "🎯 after_tool_calls called with {} responses",
                tool_responses.len()
            );
            self.inner.after_tool_calls(tool_responses, context).await
        }

        async fn after_finish(
            &self,
            content: &str,
            context: Arc<crate::agent::ExecutorContext>,
        ) -> Result<(), crate::error::AgentError> {
            self.hook_calls
                .lock()
                .await
                .push("after_finish".to_string());
            info!(
                "🎯 after_finish called with content length: {}",
                content.len()
            );
            self.inner.after_finish(content, context).await
        }
    }

    let agent_def = AgentDefinition {
        name: "test_hook_tracking_agent".to_string(),
        description: "A test hook tracking agent".to_string(),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        plan: None,
        icon_url: None,
        max_iterations: Some(3),
        sub_agents: vec![],
        skills: vec![],
        version: None,
    };

    let executor = init_executor().await;
    let session_store = executor.session_store.clone();

    let hook_calls = Arc::new(Mutex::new(Vec::new()));
    let tracking_agent = MockHookTrackingAgent {
        inner: StandardAgent::new(
            agent_def.clone(),
            Arc::default(),
            executor.clone(),
            Arc::new(crate::agent::ExecutorContext::default()),
            session_store.clone(),
        ),
        hook_calls: hook_calls.clone(),
    };

    // Test hook methods directly
    let context = Arc::new(crate::agent::ExecutorContext::default());
    let task = TaskStep {
        task: "Test task".to_string(),
        task_images: None,
    };

    // Test after_task_step hook
    tracking_agent
        .after_task_step(task.clone(), context.clone())
        .await?;

    // Test before_llm_step hook
    tracking_agent
        .before_llm_step(&[], &None, context.clone())
        .await?;

    // Test after_finish hook
    tracking_agent
        .after_finish("Test content", context.clone())
        .await?;

    let hooks_called = hook_calls.lock().await.clone();
    info!("Hooks called: {:?}", hooks_called);

    // Verify that our hooks were called
    assert!(hooks_called.contains(&"after_task_step".to_string()));
    assert!(hooks_called.contains(&"before_llm_step".to_string()));
    assert!(hooks_called.contains(&"after_finish".to_string()));

    info!("✅ All hook methods are working correctly");

    Ok(())
}

#[tokio::test]
async fn test_filtering_agent_content_filtering() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let agent_def = AgentDefinition {
        name: "test_filtering_agent".to_string(),
        description: "A test filtering agent".to_string(),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        plan: None,
        icon_url: None,
        max_iterations: Some(3),
        sub_agents: vec![],
        skills: vec![],
        version: None,
    };

    let executor = init_executor().await;
    let session_store = executor.session_store.clone();

    // Create a FilteringAgent with banned words
    let filtering_agent = FilteringAgent::new(
        agent_def.clone(),
        Arc::default(),
        executor.clone(),
        Arc::new(crate::agent::ExecutorContext::default()),
        session_store.clone(),
        vec!["badword".to_string(), "inappropriate".to_string()],
    );

    // Test the filtering logic by accessing the filter_content method
    // Note: We can't directly access private methods, so we test through the public interface

    info!("✅ FilteringAgent created with banned words");

    // Test message filtering through before_llm_step
    let test_message = crate::types::Message {
        role: crate::types::MessageRole::User,
        name: Some("user".to_string()),
        content: vec![crate::types::MessageContent {
            content_type: "text".to_string(),
            text: Some("This is a badword and inappropriate content".to_string()),
            image: None,
        }],
        tool_calls: vec![],
    };

    let context = Arc::new(crate::agent::ExecutorContext::default());
    let filtered_messages = filtering_agent
        .before_llm_step(&[test_message], &None, context)
        .await?;

    // Check that the content was filtered
    if let Some(first_message) = filtered_messages.first() {
        if let Some(first_content) = first_message.content.first() {
            if let Some(text) = &first_content.text {
                info!(
                    "Original vs Filtered: 'This is a badword and inappropriate content' -> '{}'",
                    text
                );
                assert!(text.contains("*******")); // Should contain asterisks where badword was
                assert!(!text.contains("badword")); // Should not contain the original word
                assert!(!text.contains("inappropriate")); // Should not contain the original word
            }
        }
    }

    info!("✅ Content filtering is working correctly");

    Ok(())
}
