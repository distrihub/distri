use crate::{
    error::AgentError,
    executor::AgentExecutor,
    servers::registry::ServerRegistry,
    store::{AgentSessionStore, ToolSessionStore},
    tools::{execute_tool, get_tools},
    types::{AgentDefinition, AgentSession, Message, ServerTools},
};
use std::{collections::HashMap, sync::Arc, time::SystemTime};
use tokio::sync::{mpsc, Mutex, RwLock};

use super::{AgentCoordinator, AgentHandle, CoordinatorMessage};
// Message types for coordinator communication

#[derive(Clone)]
pub struct LocalCoordinator {
    agent_definitions: Arc<RwLock<HashMap<String, AgentDefinition>>>,
    agent_tools: Arc<RwLock<HashMap<String, Vec<ServerTools>>>>,
    agent_sessions: Option<Arc<Box<dyn AgentSessionStore>>>,
    tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
    registry: Arc<RwLock<ServerRegistry>>,
    coordinator_rx: Arc<Mutex<mpsc::Receiver<CoordinatorMessage>>>,
    coordinator_tx: mpsc::Sender<CoordinatorMessage>,
}

impl LocalCoordinator {
    pub fn new(
        registry: Arc<RwLock<ServerRegistry>>,
        agent_sessions: Option<Arc<Box<dyn AgentSessionStore>>>,
        tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
    ) -> Self {
        let (coordinator_tx, coordinator_rx) = mpsc::channel(100);
        Self {
            agent_definitions: Arc::new(RwLock::new(HashMap::new())),
            agent_tools: Arc::new(RwLock::new(HashMap::new())),
            agent_sessions,
            tool_sessions,
            registry,
            coordinator_rx: Arc::new(Mutex::new(coordinator_rx)),
            coordinator_tx,
        }
    }

    pub fn get_handle(&self, agent_id: String) -> AgentHandle {
        AgentHandle {
            agent_id,
            coordinator_tx: self.coordinator_tx.clone(),
        }
    }

    pub async fn register_agent(&self, definition: AgentDefinition) -> anyhow::Result<()> {
        let (name, resolved_tools) = {
            let name = definition.name.clone();
            let server_tools = get_tools(&definition.mcp_servers, self.registry.clone()).await?;

            (name, server_tools)
        };
        // Store both the definition and its tools

        tracing::debug!(
            "Registering agent: {name} with {:?}",
            resolved_tools
                .iter()
                .map(
                    |r| serde_json::json!({"name": r.definition.name, "type": r.definition.r#type, "tools": r.tools.len()})
                )
                .collect::<Vec<_>>()
        );

        {
            let mut definitions = self.agent_definitions.write().await;
            definitions.insert(name.clone(), definition);
        }

        // Store the resolved tools
        let mut tools = self.agent_tools.write().await;
        tools.insert(name, resolved_tools);
        Ok(())
    }

    async fn store_session_messages(
        &self,
        agent_id: &str,
        messages: &[Message],
    ) -> anyhow::Result<()> {
        if let Some(store) = &self.agent_sessions {
            let session = store
                .get_session(agent_id)
                .await?
                .unwrap_or_else(|| AgentSession {
                    agent_id: agent_id.to_string(),
                    parent_agent_id: None,
                    messages: Vec::new(),
                    created_at: SystemTime::now(),
                    updated_at: SystemTime::now(),
                });

            let mut updated_session = session.clone();
            updated_session.messages.extend_from_slice(messages);
            updated_session.updated_at = SystemTime::now();

            store.set_session(updated_session).await?;
        }
        Ok(())
    }

    async fn get_session_messages(&self, agent_id: &str) -> Result<Vec<Message>, AgentError> {
        if let Some(store) = &self.agent_sessions {
            if let Some(session) = store.get_session(agent_id).await? {
                return Ok(session.messages);
            }
        }
        Ok(Vec::new())
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        tracing::info!("AgentCoordinator run loop started");

        while let Some(msg) = self.coordinator_rx.lock().await.recv().await {
            tracing::info!("AgentCoordinator received a message: {:?}", msg);
            match msg {
                CoordinatorMessage::ExecuteTool {
                    agent_id,
                    tool_call,
                    response_tx,
                } => {
                    tracing::info!("Handling ExecuteTool for agent: {}", agent_id);
                    let registry = self.registry.clone();
                    let agent_tools = self.agent_tools.clone();
                    let tool_sessions = self.tool_sessions.clone();
                    tokio::spawn(async move {
                        let result = async {
                            // Get the server tools in a separate scope to release the lock quickly
                            let server_tools = {
                                let tools = agent_tools.read().await;
                                tools.get(&agent_id).cloned()
                            };

                            match server_tools {
                                Some(server_tools) => {
                                    // Execute the tool
                                    let tool_def = server_tools.iter().find(|t| {
                                        t.tools.iter().any(|tool| tool.name == tool_call.tool_name)
                                    });

                                    let content = match tool_def {
                                        Some(server_tool) => execute_tool(
                                            &tool_call,
                                            &server_tool.definition,
                                            registry,
                                            tool_sessions,
                                        )
                                        .await
                                        .unwrap_or_else(|err| format!("Error: {}", err)),
                                        None => format!("Tool not found {}", tool_call.tool_name),
                                    };
                                    Ok(content)
                                }
                                None => Err(AgentError::ToolExecution(format!(
                                    "Agent {} not found",
                                    agent_id
                                ))),
                            }
                        }
                        .await;

                        let _ =
                            response_tx.send(result.unwrap_or_else(|e| format!("Error: {}", e)));
                    });
                }
                CoordinatorMessage::Execute {
                    agent_id,
                    messages,
                    params,
                    response_tx,
                } => {
                    tracing::info!(
                        "Handling Execute for agent: {} with messages: {:?}",
                        agent_id,
                        messages
                    );
                    let definitions = self.agent_definitions.clone();
                    let coordinator = Arc::new(self.get_handle(agent_id.clone()));
                    let agent_tools = self.agent_tools.clone();
                    let this = self.clone();

                    tokio::spawn(async move {
                        tracing::info!("Spawned Execute task for agent: {}", agent_id);
                        let result = async {
                            // Get agent definition - MOVE THIS OUTSIDE OTHER ASYNC OPERATIONS
                            let definition = {
                                let definitions = definitions.read().await;
                                definitions.get(&agent_id).cloned().ok_or_else(|| {
                                    AgentError::ToolExecution(format!(
                                        "Agent {} not found",
                                        agent_id
                                    ))
                                })?
                            };

                            // Get the tools for this agent
                            let tools = {
                                agent_tools
                                    .read()
                                    .await
                                    .get(&agent_id)
                                    .cloned()
                                    .unwrap_or_default()
                            };

                            let history_size = definition.history_size;
                            // Get all messages including parent messages
                            let mut all_messages = if history_size.is_some() {
                                this.get_session_messages(&agent_id).await?
                            } else {
                                vec![]
                            };
                            // Append new messages
                            all_messages.extend_from_slice(&messages);

                            // Create executor and execute
                            let executor = AgentExecutor::new(definition, tools, Some(coordinator));
                            let response = executor.execute(&all_messages, params).await?;

                            all_messages.push(Message {
                                name: Some(agent_id.clone()),
                                role: crate::types::Role::Assistant,
                                message: response.clone(),
                            });
                            if let Some(history_size) = history_size {
                                let sliced_messages = all_messages
                                    .into_iter()
                                    .rev()
                                    .take(history_size)
                                    .rev()
                                    .collect::<Vec<_>>();
                                this.store_session_messages(&agent_id, &sliced_messages)
                                    .await
                                    .map_err(|e| AgentError::Session(e.to_string()))?;
                            }

                            Ok(response)
                        }
                        .await;

                        let _ = response_tx.send(result);
                    });
                }
            }
        }
        tracing::info!("AgentCoordinator run loop exiting");
        Ok(())
    }
}

#[async_trait::async_trait]
impl AgentCoordinator for LocalCoordinator {
    async fn list_agents(
        &self,
        _cursor: Option<String>,
    ) -> Result<(Vec<AgentDefinition>, Option<String>), AgentError> {
        let definitions = self.agent_definitions.read().await;
        let agents: Vec<AgentDefinition> = definitions.values().take(30).cloned().collect();
        Ok((agents, None))
    }

    async fn get_agent(&self, agent_name: &str) -> Result<AgentDefinition, AgentError> {
        let definitions = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.agent_definitions.read(),
        )
        .await
        .map_err(|_| AgentError::ToolExecution("get_agent timed out".into()))?;

        definitions
            .get(agent_name)
            .cloned()
            .ok_or_else(|| AgentError::ToolExecution(format!("Agent {} not found", agent_name)))
    }

    async fn get_tools(&self, agent_name: &str) -> Result<Vec<ServerTools>, AgentError> {
        let tools =
            tokio::time::timeout(std::time::Duration::from_secs(5), self.agent_tools.read())
                .await
                .map_err(|_| AgentError::ToolExecution("get_tools timed out".into()))?;

        Ok(tools.get(agent_name).cloned().unwrap_or_default())
    }

    async fn execute(
        &self,
        agent_name: &str,
        messages: Vec<Message>,
        params: Option<serde_json::Value>,
    ) -> Result<String, AgentError> {
        // Start coordinator in background

        let handle = self.get_handle(agent_name.to_string());
        handle.execute(messages, params).await
    }
}
