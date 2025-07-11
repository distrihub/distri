use std::{collections::HashMap, sync::Arc};
use tokio::sync::mpsc;
use tracing::info;

use crate::{
    agent::{
        AgentEvent, AgentEventType, AgentExecutor, AgentExecutorBuilder, BaseAgent, ExecutorContext,
        StandardAgent,
    },
    memory::TaskStep,
    tests::{
        mock_llm::{create_mock_tool_call, MockLLM, MockResponse},
        utils::init_executor,
    },
    tools::LlmToolsRegistry,
    types::{AgentDefinition, ModelSettings, StoreConfig},
    a2a::{A2AHandler, SseMessage},
};
use async_openai::types::FinishReason;
use distri_a2a::{JsonRpcRequest, MessageSendParams, Message as A2aMessage, Part, Role, TextPart};

/// Test that all events are properly returned during agent execution
#[tokio::test]
async fn test_all_events_returned() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    
    let tool_call = create_mock_tool_call("test_tool", r#"{"query": "test"}"#);
    let mock_llm = MockLLM::new().with_responses(vec![
        MockResponse::text("I need to use a tool").with_tool_calls(vec![tool_call.clone()]),
        MockResponse::text("Here's the final result")
    ]);

    let agent_def = AgentDefinition {
        name: "test_agent".to_string(),
        description: "Test agent for event system".to_string(),
        system_prompt: Some("You are a helpful assistant".to_string()),
        model_settings: ModelSettings::default(),
        ..Default::default()
    };

    let executor = init_executor().await;
    let session_store = executor.session_store.clone();
    let tools_registry = Arc::new(LlmToolsRegistry::new(HashMap::new()));
    let context = Arc::new(ExecutorContext::default());

    // Create a mock agent that uses our mock LLM
    let agent = StandardAgent::new(
        agent_def.clone(),
        tools_registry.clone(),
        executor.clone(),
        context.clone(),
        session_store,
    );

    executor.register_agent(Box::new(agent)).await?;

    // Start coordinator
    let executor_clone = executor.clone();
    let coordinator_handle = tokio::spawn(async move {
        executor_clone.run().await.unwrap();
    });

    // Create event channel
    let (event_tx, mut event_rx) = mpsc::channel(100);

    // Execute streaming task
    let task = TaskStep {
        task: "Test task that uses tools".to_string(),
        task_images: None,
    };

    let executor_clone = executor.clone();
    let handle = tokio::spawn(async move {
        executor_clone
            .execute_stream("test_agent", task, None, event_tx, context)
            .await
    });

    // Collect all events
    let mut events = Vec::new();
    let mut run_finished = false;
    
    while let Some(event) = event_rx.recv().await {
        match &event.event {
            AgentEventType::RunFinished { .. } => {
                run_finished = true;
                events.push(event);
                break;
            }
            AgentEventType::RunError { .. } => {
                events.push(event);
                break;
            }
            _ => {
                events.push(event);
            }
        }
    }

    // Wait for execution to complete
    let _ = handle.await;

    // Verify all expected events were received
    let run_started = events.iter().any(|e| matches!(e.event, AgentEventType::RunStarted { .. }));
    let run_finished = events.iter().any(|e| matches!(e.event, AgentEventType::RunFinished { .. }));
    let text_message_start = events.iter().any(|e| matches!(e.event, AgentEventType::TextMessageStart { .. }));
    let text_message_content = events.iter().any(|e| matches!(e.event, AgentEventType::TextMessageContent { .. }));
    let text_message_end = events.iter().any(|e| matches!(e.event, AgentEventType::TextMessageEnd { .. }));
    let tool_call_start = events.iter().any(|e| matches!(e.event, AgentEventType::ToolCallStart { .. }));
    let tool_call_args = events.iter().any(|e| matches!(e.event, AgentEventType::ToolCallArgs { .. }));
    let tool_call_end = events.iter().any(|e| matches!(e.event, AgentEventType::ToolCallEnd { .. }));
    let tool_call_result = events.iter().any(|e| matches!(e.event, AgentEventType::ToolCallResult { .. }));

    info!("Received {} events", events.len());
    for event in &events {
        info!("Event: {:?}", event.event);
    }

    assert!(run_started, "RunStarted event should be present");
    assert!(run_finished, "RunFinished event should be present");
    assert!(text_message_start, "TextMessageStart event should be present");
    assert!(text_message_content, "TextMessageContent event should be present");
    assert!(text_message_end, "TextMessageEnd event should be present");
    assert!(tool_call_start, "ToolCallStart event should be present");
    assert!(tool_call_args, "ToolCallArgs event should be present");
    assert!(tool_call_end, "ToolCallEnd event should be present");
    assert!(tool_call_result, "ToolCallResult event should be present");

    // Verify event order makes sense
    let run_start_pos = events.iter().position(|e| matches!(e.event, AgentEventType::RunStarted { .. })).unwrap();
    let run_end_pos = events.iter().position(|e| matches!(e.event, AgentEventType::RunFinished { .. })).unwrap();
    assert!(run_start_pos < run_end_pos, "RunStarted should come before RunFinished");

    coordinator_handle.abort();
    Ok(())
}

/// Test that error events are properly returned
#[tokio::test]
async fn test_error_events_returned() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    
    let mock_llm = MockLLM::new().with_failure();

    let agent_def = AgentDefinition {
        name: "failing_agent".to_string(),
        description: "Test agent that fails".to_string(),
        system_prompt: Some("You are a helpful assistant".to_string()),
        model_settings: ModelSettings::default(),
        ..Default::default()
    };

    let executor = init_executor().await;
    let session_store = executor.session_store.clone();
    let tools_registry = Arc::new(LlmToolsRegistry::new(HashMap::new()));
    let context = Arc::new(ExecutorContext::default());

    let agent = StandardAgent::new(
        agent_def.clone(),
        tools_registry.clone(),
        executor.clone(),
        context.clone(),
        session_store,
    );

    executor.register_agent(Box::new(agent)).await?;

    // Start coordinator
    let executor_clone = executor.clone();
    let coordinator_handle = tokio::spawn(async move {
        executor_clone.run().await.unwrap();
    });

    // Create event channel
    let (event_tx, mut event_rx) = mpsc::channel(100);

    // Execute streaming task
    let task = TaskStep {
        task: "Test task that fails".to_string(),
        task_images: None,
    };

    let executor_clone = executor.clone();
    let handle = tokio::spawn(async move {
        executor_clone
            .execute_stream("failing_agent", task, None, event_tx, context)
            .await
    });

    // Collect all events
    let mut events = Vec::new();
    
    while let Some(event) = event_rx.recv().await {
        match &event.event {
            AgentEventType::RunError { .. } => {
                events.push(event);
                break;
            }
            AgentEventType::RunFinished { .. } => {
                events.push(event);
                break;
            }
            _ => {
                events.push(event);
            }
        }
    }

    // Wait for execution to complete
    let _ = handle.await;

    // Verify error event was received
    let run_error = events.iter().any(|e| matches!(e.event, AgentEventType::RunError { .. }));
    assert!(run_error, "RunError event should be present");

    coordinator_handle.abort();
    Ok(())
}

/// Test that background mode works properly
#[tokio::test]
async fn test_background_mode() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    
    let mock_llm = MockLLM::new().with_responses(vec![
        MockResponse::text("Background task response").with_chunks(vec![
            "Back".to_string(),
            "ground".to_string(),
            " task".to_string(),
            " response".to_string(),
        ])
    ]);

    let agent_def = AgentDefinition {
        name: "background_agent".to_string(),
        description: "Test agent for background mode".to_string(),
        system_prompt: Some("You are a helpful assistant".to_string()),
        model_settings: ModelSettings::default(),
        ..Default::default()
    };

    let executor = init_executor().await;
    let session_store = executor.session_store.clone();
    let tools_registry = Arc::new(LlmToolsRegistry::new(HashMap::new()));
    let context = Arc::new(ExecutorContext::default());

    let agent = StandardAgent::new(
        agent_def.clone(),
        tools_registry.clone(),
        executor.clone(),
        context.clone(),
        session_store,
    );

    executor.register_agent(Box::new(agent)).await?;

    // Start coordinator
    let executor_clone = executor.clone();
    let coordinator_handle = tokio::spawn(async move {
        executor_clone.run().await.unwrap();
    });

    // Create event channel
    let (event_tx, mut event_rx) = mpsc::channel(100);

    // Execute task in background (similar to distri-cli background mode)
    let task = TaskStep {
        task: "Background task".to_string(),
        task_images: None,
    };

    let executor_clone = executor.clone();
    let background_handle = tokio::spawn(async move {
        executor_clone
            .execute_stream("background_agent", task, None, event_tx, context)
            .await
    });

    // Collect events from the background task
    let mut events = Vec::new();
    let mut content_chunks = Vec::new();
    
    while let Some(event) = event_rx.recv().await {
        match &event.event {
            AgentEventType::TextMessageContent { delta, .. } => {
                content_chunks.push(delta.clone());
                events.push(event);
            }
            AgentEventType::RunFinished { .. } => {
                events.push(event);
                break;
            }
            AgentEventType::RunError { .. } => {
                events.push(event);
                break;
            }
            _ => {
                events.push(event);
            }
        }
    }

    // Wait for background task to complete
    let _ = background_handle.await;

    // Verify background mode works
    let run_started = events.iter().any(|e| matches!(e.event, AgentEventType::RunStarted { .. }));
    let run_finished = events.iter().any(|e| matches!(e.event, AgentEventType::RunFinished { .. }));
    
    assert!(run_started, "Background task should have RunStarted event");
    assert!(run_finished, "Background task should have RunFinished event");
    assert_eq!(content_chunks.len(), 4, "Should have received 4 content chunks");
    
    let full_content = content_chunks.join("");
    assert_eq!(full_content, "Background task response");

    coordinator_handle.abort();
    Ok(())
}

/// Test that A2A handler properly streams events
#[tokio::test]
async fn test_a2a_handler_streaming() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    
    let mock_llm = MockLLM::new().with_responses(vec![
        MockResponse::text("A2A response").with_chunks(vec![
            "A2A".to_string(),
            " response".to_string(),
        ])
    ]);

    let agent_def = AgentDefinition {
        name: "a2a_agent".to_string(),
        description: "Test agent for A2A handler".to_string(),
        system_prompt: Some("You are a helpful assistant".to_string()),
        model_settings: ModelSettings::default(),
        ..Default::default()
    };

    let executor = init_executor().await;
    let session_store = executor.session_store.clone();
    let tools_registry = Arc::new(LlmToolsRegistry::new(HashMap::new()));
    let context = Arc::new(ExecutorContext::default());

    let agent = StandardAgent::new(
        agent_def.clone(),
        tools_registry.clone(),
        executor.clone(),
        context.clone(),
        session_store,
    );

    executor.register_agent(Box::new(agent)).await?;

    // Start coordinator
    let executor_clone = executor.clone();
    let coordinator_handle = tokio::spawn(async move {
        executor_clone.run().await.unwrap();
    });

    // Create A2A handler
    let a2a_handler = A2AHandler::new(executor.clone());

    // Create a mock A2A message
    let a2a_message = A2aMessage {
        message_id: uuid::Uuid::new_v4().to_string(),
        role: Role::User,
        parts: vec![Part::Text(TextPart {
            text: "Hello from A2A".to_string(),
        })],
        context_id: Some("test_context".to_string()),
        task_id: Some(uuid::Uuid::new_v4().to_string()),
        ..Default::default()
    };

    // Create JSON-RPC request
    let params = MessageSendParams {
        message: a2a_message,
        metadata: None,
    };

    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "message/stream".to_string(),
        params: serde_json::to_value(params)?,
        id: Some(serde_json::Value::String("test_request".to_string())),
    };

    // Handle the request and get the stream
    let response = a2a_handler.handle_jsonrpc("a2a_agent".to_string(), request, None).await;

    // Process the stream
    let mut sse_messages = Vec::new();
    
    match response {
        futures::future::Either::Left(mut stream) => {
            use futures::StreamExt;
            
            while let Some(result) = stream.next().await {
                match result {
                    Ok(sse_message) => {
                        sse_messages.push(sse_message);
                        
                        // Parse the message to check if it's a completion event
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&sse_message.data) {
                            if let Some(result) = parsed.get("result") {
                                if let Some(metadata) = result.get("metadata") {
                                    if let Some(event_type) = metadata.get("type") {
                                        if event_type == "run_finished" {
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        panic!("Stream error: {:?}", e);
                    }
                }
            }
        }
        futures::future::Either::Right(json_response) => {
            panic!("Expected stream response, got JSON response: {:?}", json_response);
        }
    }

    // Verify A2A streaming worked
    assert!(!sse_messages.is_empty(), "Should have received SSE messages");
    
    // Check for various event types in the metadata
    let mut has_run_started = false;
    let mut has_text_content = false;
    let mut has_run_finished = false;
    
    for sse_message in &sse_messages {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&sse_message.data) {
            if let Some(result) = parsed.get("result") {
                if let Some(metadata) = result.get("metadata") {
                    if let Some(event_type) = metadata.get("type") {
                        match event_type.as_str() {
                            Some("run_started") => has_run_started = true,
                            Some("text_message_content") => has_text_content = true,
                            Some("run_finished") => has_run_finished = true,
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    assert!(has_run_started, "A2A stream should have run_started event");
    assert!(has_text_content, "A2A stream should have text_message_content event");
    assert!(has_run_finished, "A2A stream should have run_finished event");

    coordinator_handle.abort();
    Ok(())
}

/// Test that tool call events are properly streamed
#[tokio::test]
async fn test_tool_call_events_streaming() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    
    let tool_call = create_mock_tool_call("search_tool", r#"{"query": "test search"}"#);
    let mock_llm = MockLLM::new().with_responses(vec![
        MockResponse::text("I'll search for that").with_tool_calls(vec![tool_call.clone()]),
        MockResponse::text("Here are the search results")
    ]);

    let agent_def = AgentDefinition {
        name: "tool_agent".to_string(),
        description: "Test agent with tools".to_string(),
        system_prompt: Some("You are a helpful assistant with tools".to_string()),
        model_settings: ModelSettings::default(),
        ..Default::default()
    };

    let executor = init_executor().await;
    let session_store = executor.session_store.clone();
    let tools_registry = Arc::new(LlmToolsRegistry::new(HashMap::new()));
    let context = Arc::new(ExecutorContext::default());

    let agent = StandardAgent::new(
        agent_def.clone(),
        tools_registry.clone(),
        executor.clone(),
        context.clone(),
        session_store,
    );

    executor.register_agent(Box::new(agent)).await?;

    // Start coordinator
    let executor_clone = executor.clone();
    let coordinator_handle = tokio::spawn(async move {
        executor_clone.run().await.unwrap();
    });

    // Create event channel
    let (event_tx, mut event_rx) = mpsc::channel(100);

    // Execute task with tool calls
    let task = TaskStep {
        task: "Search for information".to_string(),
        task_images: None,
    };

    let executor_clone = executor.clone();
    let handle = tokio::spawn(async move {
        executor_clone
            .execute_stream("tool_agent", task, None, event_tx, context)
            .await
    });

    // Collect tool call events
    let mut tool_events = Vec::new();
    let mut all_events = Vec::new();
    
    while let Some(event) = event_rx.recv().await {
        match &event.event {
            AgentEventType::ToolCallStart { .. } |
            AgentEventType::ToolCallArgs { .. } |
            AgentEventType::ToolCallEnd { .. } |
            AgentEventType::ToolCallResult { .. } => {
                tool_events.push(event.clone());
            }
            AgentEventType::RunFinished { .. } => {
                all_events.push(event);
                break;
            }
            AgentEventType::RunError { .. } => {
                all_events.push(event);
                break;
            }
            _ => {}
        }
        all_events.push(event);
    }

    // Wait for execution to complete
    let _ = handle.await;

    // Verify tool call events
    let tool_call_start = tool_events.iter().any(|e| matches!(e.event, AgentEventType::ToolCallStart { .. }));
    let tool_call_args = tool_events.iter().any(|e| matches!(e.event, AgentEventType::ToolCallArgs { .. }));
    let tool_call_end = tool_events.iter().any(|e| matches!(e.event, AgentEventType::ToolCallEnd { .. }));
    let tool_call_result = tool_events.iter().any(|e| matches!(e.event, AgentEventType::ToolCallResult { .. }));

    assert!(tool_call_start, "ToolCallStart event should be present");
    assert!(tool_call_args, "ToolCallArgs event should be present"); 
    assert!(tool_call_end, "ToolCallEnd event should be present");
    assert!(tool_call_result, "ToolCallResult event should be present");

    // Verify tool call event order
    let start_pos = tool_events.iter().position(|e| matches!(e.event, AgentEventType::ToolCallStart { .. })).unwrap();
    let args_pos = tool_events.iter().position(|e| matches!(e.event, AgentEventType::ToolCallArgs { .. })).unwrap();
    let end_pos = tool_events.iter().position(|e| matches!(e.event, AgentEventType::ToolCallEnd { .. })).unwrap();
    let result_pos = tool_events.iter().position(|e| matches!(e.event, AgentEventType::ToolCallResult { .. })).unwrap();

    assert!(start_pos < args_pos, "ToolCallStart should come before ToolCallArgs");
    assert!(args_pos < end_pos, "ToolCallArgs should come before ToolCallEnd");
    assert!(end_pos < result_pos, "ToolCallEnd should come before ToolCallResult");

    coordinator_handle.abort();
    Ok(())
}

/// Test that events have correct metadata (thread_id, run_id)
#[tokio::test]
async fn test_event_metadata() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    
    let mock_llm = MockLLM::new().with_responses(vec![
        MockResponse::text("Test response")
    ]);

    let agent_def = AgentDefinition {
        name: "metadata_agent".to_string(),
        description: "Test agent for metadata verification".to_string(),
        system_prompt: Some("You are a helpful assistant".to_string()),
        model_settings: ModelSettings::default(),
        ..Default::default()
    };

    let executor = init_executor().await;
    let session_store = executor.session_store.clone();
    let tools_registry = Arc::new(LlmToolsRegistry::new(HashMap::new()));
    
    // Create context with specific IDs
    let thread_id = "test_thread_123".to_string();
    let run_id = "test_run_456".to_string();
    let context = Arc::new(ExecutorContext {
        thread_id: thread_id.clone(),
        run_id: Arc::new(tokio::sync::Mutex::new(run_id.clone())),
        verbose: false,
        user_id: Some("test_user".to_string()),
        metadata: None,
        req_id: None,
    });

    let agent = StandardAgent::new(
        agent_def.clone(),
        tools_registry.clone(),
        executor.clone(),
        context.clone(),
        session_store,
    );

    executor.register_agent(Box::new(agent)).await?;

    // Start coordinator
    let executor_clone = executor.clone();
    let coordinator_handle = tokio::spawn(async move {
        executor_clone.run().await.unwrap();
    });

    // Create event channel
    let (event_tx, mut event_rx) = mpsc::channel(100);

    // Execute task
    let task = TaskStep {
        task: "Test metadata".to_string(),
        task_images: None,
    };

    let executor_clone = executor.clone();
    let handle = tokio::spawn(async move {
        executor_clone
            .execute_stream("metadata_agent", task, None, event_tx, context)
            .await
    });

    // Collect events and verify metadata
    let mut events = Vec::new();
    
    while let Some(event) = event_rx.recv().await {
        match &event.event {
            AgentEventType::RunFinished { .. } => {
                events.push(event);
                break;
            }
            AgentEventType::RunError { .. } => {
                events.push(event);
                break;
            }
            _ => {
                events.push(event);
            }
        }
    }

    // Wait for execution to complete
    let _ = handle.await;

    // Verify all events have correct metadata
    for event in &events {
        assert_eq!(event.thread_id, thread_id, "Event should have correct thread_id");
        assert_eq!(event.run_id, run_id, "Event should have correct run_id");
    }

    coordinator_handle.abort();
    Ok(())
}

/// Test concurrent event streaming
#[tokio::test]
async fn test_concurrent_event_streaming() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    
    let mock_llm = MockLLM::new().with_responses(vec![
        MockResponse::text("Concurrent response 1").with_delay(),
        MockResponse::text("Concurrent response 2").with_delay(),
        MockResponse::text("Concurrent response 3").with_delay(),
    ]);

    let agent_def = AgentDefinition {
        name: "concurrent_agent".to_string(),
        description: "Test agent for concurrent streaming".to_string(),
        system_prompt: Some("You are a helpful assistant".to_string()),
        model_settings: ModelSettings::default(),
        ..Default::default()
    };

    let executor = init_executor().await;
    let session_store = executor.session_store.clone();
    let tools_registry = Arc::new(LlmToolsRegistry::new(HashMap::new()));

    let agent = StandardAgent::new(
        agent_def.clone(),
        tools_registry.clone(),
        executor.clone(),
        Arc::new(ExecutorContext::default()),
        session_store,
    );

    executor.register_agent(Box::new(agent)).await?;

    // Start coordinator
    let executor_clone = executor.clone();
    let coordinator_handle = tokio::spawn(async move {
        executor_clone.run().await.unwrap();
    });

    // Create multiple concurrent streams
    let mut handles = Vec::new();
    let mut event_receivers = Vec::new();

    for i in 0..3 {
        let (event_tx, event_rx) = mpsc::channel(100);
        event_receivers.push(event_rx);

        let task = TaskStep {
            task: format!("Concurrent task {}", i + 1),
            task_images: None,
        };

        let executor_clone = executor.clone();
        let context = Arc::new(ExecutorContext::default());
        
        let handle = tokio::spawn(async move {
            executor_clone
                .execute_stream("concurrent_agent", task, None, event_tx, context)
                .await
        });
        
        handles.push(handle);
    }

    // Collect events from all streams
    let mut all_events = Vec::new();
    let mut tasks = Vec::new();

    for (i, mut event_rx) in event_receivers.into_iter().enumerate() {
        let task = tokio::spawn(async move {
            let mut events = Vec::new();
            while let Some(event) = event_rx.recv().await {
                match &event.event {
                    AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. } => {
                        events.push(event);
                        break;
                    }
                    _ => {
                        events.push(event);
                    }
                }
            }
            (i, events)
        });
        tasks.push(task);
    }

    // Wait for all event collection tasks
    for task in tasks {
        let (stream_id, events) = task.await?;
        info!("Stream {} collected {} events", stream_id, events.len());
        all_events.extend(events);
    }

    // Wait for all execution handles
    for handle in handles {
        let _ = handle.await;
    }

    // Verify concurrent streaming worked
    assert!(!all_events.is_empty(), "Should have received events from concurrent streams");
    
    // Count run events (should have 3 start and 3 finish events)
    let run_started_count = all_events.iter().filter(|e| matches!(e.event, AgentEventType::RunStarted { .. })).count();
    let run_finished_count = all_events.iter().filter(|e| matches!(e.event, AgentEventType::RunFinished { .. })).count();
    
    assert_eq!(run_started_count, 3, "Should have 3 RunStarted events");
    assert_eq!(run_finished_count, 3, "Should have 3 RunFinished events");

    coordinator_handle.abort();
    Ok(())
}