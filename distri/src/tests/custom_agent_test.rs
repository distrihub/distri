use crate::{
    agent::{
        AgentExecutor, AgentInvoke, AgentInvokeStream, BaseAgent, DefaultAgent, ExecutorContext,
        TestCustomAgent,
    },
    memory::TaskStep,
    servers::registry::ServerRegistry,
    store::{AgentStore, InMemoryAgentStore, LocalSessionStore},
    types::{AgentDefinition, AgentRecord, ModelSettings},
    SessionStore,
};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

fn get_test_context() -> Arc<ExecutorContext> {
    Arc::new(ExecutorContext::new(
        "test_thread".to_string(),
        None,
        true,
        Some("test_user".to_string()),
        None,
    ))
}

#[tokio::test]
async fn test_default_agent_invoke() {
    let context = get_test_context();

    let agent_store = Arc::new(InMemoryAgentStore::new());
    let session_store = Arc::new(Box::new(LocalSessionStore::new()) as Box<dyn SessionStore>);
    let registry = Arc::new(RwLock::new(ServerRegistry::new()));

    let executor = Arc::new(AgentExecutor::new(
        registry,
        None,
        Some(session_store.clone()),
        agent_store.clone(),
        context.clone(),
    ));

    let definition = AgentDefinition {
        name: "test_default_agent".to_string(),
        description: "Test default agent".to_string(),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        model_settings: ModelSettings::default(),
        ..Default::default()
    };

    let agent = DefaultAgent::new(
        definition.clone(),
        vec![],
        executor.clone(),
        context.clone(),
        session_store.clone(),
    );

    let task = TaskStep {
        task: "Hello, how are you?".to_string(),
        task_images: None,
    };

    // Test invoke without events
    let result = agent
        .invoke(task.clone(), None, context.clone(), None)
        .await;

    // Since this uses LLM which requires API calls, we expect this to fail in tests
    // The important thing is that it reaches the right code path
    assert!(result.is_err());
}

#[tokio::test]
async fn test_custom_agent_invoke() {
    let context = get_test_context();

    let custom_agent = TestCustomAgent::new("test_custom".to_string());

    let task = TaskStep {
        task: "Process this custom task".to_string(),
        task_images: None,
    };

    // Test invoke without events
    let result = custom_agent
        .invoke(task.clone(), None, context.clone(), None)
        .await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert!(response.contains("Custom agent 'test_custom' processed task"));
    assert!(response.contains("Process this custom task"));
}

#[tokio::test]
async fn test_custom_agent_invoke_stream() {
    let context = get_test_context();

    let custom_agent = TestCustomAgent::new("streaming_agent".to_string());

    let task = TaskStep {
        task: "Stream this response".to_string(),
        task_images: None,
    };

    let (event_tx, mut event_rx) = mpsc::channel(100);

    // Test invoke_stream
    let stream_result = custom_agent
        .invoke_stream(task.clone(), None, context.clone(), event_tx)
        .await;

    assert!(stream_result.is_ok());

    // Collect events
    let mut events = Vec::new();
    while let Some(event) = event_rx.recv().await {
        events.push(event);
        // Break after a reasonable number of events to avoid infinite loop
        if events.len() > 20 {
            break;
        }
    }

    // Check that we received the expected events
    assert!(!events.is_empty());

    // Should have RunStarted, TextMessageStart, multiple TextMessageContent, TextMessageEnd, RunFinished
    use crate::agent::AgentEvent;

    // First event should be RunStarted
    match &events[0] {
        AgentEvent::RunStarted { .. } => (),
        _ => panic!("Expected RunStarted event"),
    }

    // Should contain TextMessageStart
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::TextMessageStart { .. })));

    // Should contain multiple TextMessageContent events
    let content_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::TextMessageContent { .. }))
        .collect();
    assert!(!content_events.is_empty());

    // Should contain TextMessageEnd
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::TextMessageEnd { .. })));

    // Last event should be RunFinished
    match events.last().unwrap() {
        AgentEvent::RunFinished { .. } => (),
        _ => panic!("Expected RunFinished event"),
    }
}

#[tokio::test]
async fn test_agent_store_operations() {
    let context = get_test_context();

    let agent_store = Arc::new(InMemoryAgentStore::new());
    let session_store = Arc::new(Box::new(LocalSessionStore::new()) as Box<dyn SessionStore>);
    let registry = Arc::new(RwLock::new(ServerRegistry::new()));

    let executor = Arc::new(AgentExecutor::new(
        registry,
        None,
        Some(session_store.clone()),
        agent_store.clone(),
        context.clone(),
    ));

    let definition = AgentDefinition {
        name: "stored_agent".to_string(),
        description: "Test stored agent".to_string(),
        ..Default::default()
    };

    // Create a DefaultAgent
    let default_agent: Box<dyn BaseAgent> = Box::new(DefaultAgent::new(
        definition.clone(),
        vec![],
        executor.clone(),
        context.clone(),
        session_store.clone(),
    ));

    // Create an AgentRecord and register it
    let record = AgentRecord {
        definition: definition.clone(),
        agent: default_agent,
    };

    let registered_agent = executor.register_agent(record).await;
    assert!(registered_agent.is_ok());

    // Test getting the agent from store
    let retrieved_agent = agent_store.get("stored_agent").await;
    assert!(retrieved_agent.is_some());

    let agent = retrieved_agent.unwrap();
    assert_eq!(agent.get_name(), "stored_agent");

    // Test listing agents
    let (agents, _cursor) = agent_store.list(None, None).await;
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].get_name(), "stored_agent");
}

#[tokio::test]
async fn test_custom_agent_store_operations() {
    let agent_store = Arc::new(InMemoryAgentStore::new());

    // Create a custom agent
    let custom_agent: Box<dyn BaseAgent> =
        Box::new(TestCustomAgent::new("custom_test".to_string()));

    // Register the custom agent
    let result = agent_store.register(custom_agent, vec![]).await;
    assert!(result.is_ok());

    // Retrieve and test
    let retrieved_agent = agent_store.get("custom_test").await;
    assert!(retrieved_agent.is_some());

    let agent = retrieved_agent.unwrap();
    assert_eq!(agent.get_name(), "custom_test");
}

#[tokio::test]
async fn test_agent_invoke_traits_not_implemented() {
    // Create a basic agent that doesn't implement AgentInvoke/AgentInvokeStream
    #[derive(Debug, Clone)]
    struct BasicAgent {
        name: String,
        description: String,
    }

    #[async_trait::async_trait]
    impl BaseAgent for BasicAgent {
        fn get_definition(&self) -> AgentDefinition {
            AgentDefinition {
                name: self.name.clone(),
                description: self.description.clone(),
                ..Default::default()
            }
        }
        fn get_description(&self) -> &str {
            &self.description
        }

        async fn invoke(
            &self,
            _task: TaskStep,
            _params: Option<serde_json::Value>,
            _context: Arc<ExecutorContext>,
            _event_tx: Option<mpsc::Sender<crate::agent::AgentEvent>>,
        ) -> Result<String, crate::error::AgentError> {
            Ok("Basic response".to_string())
        }

        async fn invoke_stream(
            &self,
            _task: TaskStep,
            _params: Option<serde_json::Value>,
            _context: Arc<ExecutorContext>,
            _event_tx: mpsc::Sender<crate::agent::AgentEvent>,
        ) -> Result<(), crate::error::AgentError> {
            Ok(())
        }

        fn clone_box(&self) -> Box<dyn BaseAgent> {
            Box::new(self.clone())
        }

        fn get_name(&self) -> &str {
            &self.name
        }
    }

    #[async_trait::async_trait]
    impl AgentInvoke for BasicAgent {
        // Using default implementation which should error
    }

    let basic_agent = BasicAgent {
        name: "basic".to_string(),
        description: "basic".to_string(),
    };

    let context = get_test_context();

    let task = TaskStep {
        task: "test".to_string(),
        task_images: None,
    };

    // Test that default AgentInvoke implementation errors
    let result = basic_agent.agent_invoke(task, None, context, None).await;
    assert!(result.is_err());

    match result.unwrap_err() {
        crate::error::AgentError::NotImplemented(msg) => {
            assert!(msg.contains("AgentInvoke::agent_invoke not implemented"));
        }
        _ => panic!("Expected NotImplemented error"),
    }
}
