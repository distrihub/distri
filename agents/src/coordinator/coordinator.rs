use crate::{
    error::AgentError,
    executor::AgentExecutor,
    servers::registry::ServerRegistry,
    store::{AgentSessionStore, SessionStore},
    tools::execute_tool,
    types::{AgentDefinition, AgentSession, AgentStatus, Message, ServerTools, ToolCall},
};
use async_mcp::{
    server::{Server, ServerBuilder},
    transport::Transport,
    types::{CallToolRequest, CallToolResponse, ServerCapabilities, Tool},
};
use serde_json::json;
use std::{collections::HashMap, sync::Arc, time::SystemTime};
use tokio::sync::{mpsc, oneshot, RwLock};

// Message types for coordinator communication
#[derive(Debug)]
pub enum CoordinatorMessage {
    ExecuteTool {
        agent_id: String,
        tool_call: ToolCall,
        response_tx: oneshot::Sender<String>,
    },
    NewAgent {
        agent_id: String,
        definition: AgentDefinition,
        parent_session: Option<String>,
        response_tx: oneshot::Sender<Result<AgentHandle, AgentError>>,
    },
    GetAgentStatus {
        agent_id: String,
        response_tx: oneshot::Sender<Option<AgentStatus>>,
    },
    StopAgent {
        agent_id: String,
        response_tx: oneshot::Sender<Result<(), AgentError>>,
    },
}

#[derive(Debug, Clone)]
pub struct AgentHandle {
    pub agent_id: String,
    coordinator_tx: mpsc::Sender<CoordinatorMessage>,
}

pub struct AgentCoordinator {
    agents: Arc<RwLock<HashMap<String, Arc<AgentExecutor>>>>,
    agent_sessions: Option<Arc<Box<dyn AgentSessionStore>>>,
    tool_sessions: Option<Arc<Box<dyn SessionStore>>>,
    registry: Arc<ServerRegistry>,
    coordinator_rx: mpsc::Receiver<CoordinatorMessage>,
    coordinator_tx: mpsc::Sender<CoordinatorMessage>,
}

impl AgentCoordinator {
    pub fn new(
        registry: Arc<ServerRegistry>,
        agent_sessions: Option<Arc<Box<dyn AgentSessionStore>>>,
        tool_sessions: Option<Arc<Box<dyn SessionStore>>>,
    ) -> Self {
        let (coordinator_tx, coordinator_rx) = mpsc::channel(100);
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
            agent_sessions,
            tool_sessions,
            registry,
            coordinator_rx,
            coordinator_tx,
        }
    }

    pub fn get_handle(&self, agent_id: String) -> AgentHandle {
        AgentHandle {
            agent_id,
            coordinator_tx: self.coordinator_tx.clone(),
        }
    }

    async fn update_session_status(&self, agent_id: &str, status: AgentStatus) {
        if let Some(sessions) = &self.agent_sessions {
            if let Ok(mut session) = sessions.get_agent_session(agent_id).await {
                if let Some(mut session) = session {
                    session.status = status;
                    session.updated_at = SystemTime::now();
                    let _ = sessions.set_agent_session(session).await;
                }
            }
        }
    }

    pub async fn run(&mut self) {
        while let Some(msg) = self.coordinator_rx.recv().await {
            match msg {
                CoordinatorMessage::ExecuteTool {
                    agent_id,
                    tool_call,
                    response_tx,
                } => {
                    let registry = self.registry.clone();
                    let agents = self.agents.clone();
                    let agent_sessions = self.agent_sessions.clone();
                    let tool_sessions = self.tool_sessions.clone();

                    tokio::spawn(async move {
                        let result = if let Some(agent) = agents.read().await.get(&agent_id) {
                            let can_execute = if let Some(sessions) = agent_sessions {
                                if let Ok(Some(session)) =
                                    sessions.get_agent_session(&agent_id).await
                                {
                                    session.status == AgentStatus::Running
                                } else {
                                    true // No session tracking, allow execution
                                }
                            } else {
                                true // No session store, allow execution
                            };

                            if can_execute {
                                let server_tools = agent.get_server_tools();
                                execute_tool_async(&tool_call, registry, server_tools).await
                            } else {
                                Err(AgentError::ToolExecution(format!(
                                    "Agent {} is not running",
                                    agent_id
                                )))
                            }
                        } else {
                            Err(AgentError::ToolExecution(format!(
                                "Agent {} not found",
                                agent_id
                            )))
                        };

                        let _ =
                            response_tx.send(result.unwrap_or_else(|e| format!("Error: {}", e)));
                    });
                }
                CoordinatorMessage::NewAgent {
                    agent_id,
                    definition,
                    parent_session,
                    response_tx,
                } => {
                    // Create session if we have a session store
                    if let Some(sessions) = &self.agent_sessions {
                        let session = AgentSession {
                            agent_id: agent_id.clone(),
                            status: AgentStatus::Idle,
                            state: json!({}),
                            parent_session,
                            created_at: SystemTime::now(),
                            updated_at: SystemTime::now(),
                        };
                        let _ = sessions.set_agent_session(session).await;
                    }

                    // Create new agent executor
                    let executor = AgentExecutor::new(
                        definition.clone(),
                        self.registry.clone(),
                        self.tool_sessions.clone(),
                        vec![],
                        Some(Arc::new(self.get_handle(agent_id.clone()))),
                    );
                    let mut agents = self.agents.write().await;
                    agents.insert(agent_id.clone(), Arc::new(executor));

                    self.update_session_status(&agent_id, AgentStatus::Running)
                        .await;
                    let handle = self.get_handle(agent_id);
                    let _ = response_tx.send(Ok(handle));
                }
                CoordinatorMessage::GetAgentStatus {
                    agent_id,
                    response_tx,
                } => {
                    let status = if let Some(sessions) = &self.agent_sessions {
                        if let Ok(Some(session)) = sessions.get_agent_session(&agent_id).await {
                            Some(session.status)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let _ = response_tx.send(status);
                }
                CoordinatorMessage::StopAgent {
                    agent_id,
                    response_tx,
                } => {
                    self.update_session_status(&agent_id, AgentStatus::Stopped)
                        .await;
                    let _ = response_tx.send(Ok(()));
                }
            }
        }
    }
}

impl AgentHandle {
    pub async fn execute_tool(&self, tool_call: ToolCall) -> Result<String, AgentError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.coordinator_tx
            .send(CoordinatorMessage::ExecuteTool {
                agent_id: self.agent_id.clone(),
                tool_call,
                response_tx,
            })
            .await
            .map_err(|e| {
                AgentError::ToolExecution(format!("Failed to send tool execution request: {}", e))
            })?;

        response_rx.await.map_err(|e| {
            AgentError::ToolExecution(format!("Failed to receive tool response: {}", e))
        })
    }

    pub async fn get_status(&self) -> Result<Option<AgentStatus>, AgentError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.coordinator_tx
            .send(CoordinatorMessage::GetAgentStatus {
                agent_id: self.agent_id.clone(),
                response_tx,
            })
            .await
            .map_err(|e| AgentError::ToolExecution(format!("Failed to get agent status: {}", e)))?;

        response_rx.await.map_err(|e| {
            AgentError::ToolExecution(format!("Failed to receive agent status: {}", e))
        })
    }

    pub async fn stop(&self) -> Result<(), AgentError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.coordinator_tx
            .send(CoordinatorMessage::StopAgent {
                agent_id: self.agent_id.clone(),
                response_tx,
            })
            .await
            .map_err(|e| AgentError::ToolExecution(format!("Failed to stop agent: {}", e)))?;

        response_rx.await.map_err(|e| {
            AgentError::ToolExecution(format!("Failed to receive stop confirmation: {}", e))
        })?
    }

    pub async fn execute(
        &self,
        messages: Vec<Message>,
        params: Option<serde_json::Value>,
    ) -> Result<String, AgentError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.coordinator_tx
            .send(CoordinatorMessage::NewAgent {
                agent_id: self.agent_id.clone(),
                definition: AgentDefinition {
                    name: self.agent_id.clone(),
                    description: String::new(),
                    system_prompt: None,
                    mcp_servers: vec![],
                    model_settings: Default::default(),
                    parameters: Default::default(),
                    sub_agents: vec![],
                },
                parent_session: None,
                response_tx,
            })
            .await
            .map_err(|e| AgentError::ToolExecution(format!("Failed to create agent: {}", e)))?;

        let handle = response_rx.await.map_err(|e| {
            AgentError::ToolExecution(format!("Failed to receive agent handle: {}", e))
        })??;

        handle
            .execute_tool(ToolCall {
                tool_id: "execute".to_string(),
                tool_name: "execute".to_string(),
                input: serde_json::to_string(&json!({
                    "messages": messages,
                    "params": params
                }))
                .unwrap(),
            })
            .await
    }
}

async fn execute_tool_async(
    tool_call: &ToolCall,
    registry: Arc<ServerRegistry>,
    server_tools: Vec<ServerTools>,
) -> Result<String, AgentError> {
    // Find the tool definition
    let tool_def = server_tools
        .iter()
        .find(|t| t.tools.iter().any(|tool| tool.name == tool_call.tool_name))
        .ok_or_else(|| {
            AgentError::ToolExecution(format!("Tool not found: {}", tool_call.tool_name))
        })?;

    // Execute the tool
    execute_tool(tool_call, &tool_def.definition, registry, None)
        .await
        .map_err(|e| AgentError::ToolExecution(e.to_string()))
}
