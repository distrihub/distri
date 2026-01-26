use crate::agent::{BaseAgent, ExecutorContext, InvokeResult};
use crate::tools::Tool;
use crate::types::{AgentError, Message, ToolCall};
use anyhow::Result;
use distri_plugin_executor::PluginExecutor;
use distri_types::configuration::AgentConfig;
use distri_types::configuration::{
    AgentRef, CustomAgentDefinition, DagWorkflowDefinition, DagWorkflowNode,
    SequentialWorkflowDefinition, WorkflowStep,
};
use distri_types::{AgentEvent, AgentEventType};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info};
use uuid::Uuid;

/// Workflow agent that can execute both sequential and DAG workflows
#[derive(Debug, Clone)]
pub struct WorkflowAgent {
    name: String,
    definition: AgentConfig,
    tools: Vec<Arc<dyn Tool>>,
}

#[derive(Debug, Clone)]
pub enum WorkflowType {
    Sequential(SequentialWorkflowDefinition),
    Dag(DagWorkflowDefinition),
    Custom(CustomAgentDefinition),
}
impl WorkflowAgent {
    /// Create a new workflow agent from any AgentConfig
    pub fn new(
        config: distri_types::configuration::AgentConfig,
        tools: Vec<Arc<dyn Tool>>,
    ) -> Self {
        Self {
            name: config.get_name().to_string(),
            definition: config,
            tools,
        }
    }

    /// Create a new workflow agent from a sequential workflow definition  
    pub fn new_sequential(
        definition: SequentialWorkflowDefinition,
        tools: Vec<Arc<dyn Tool>>,
    ) -> Self {
        let config = distri_types::configuration::AgentConfig::SequentialWorkflowAgent(definition);
        Self::new(config, tools)
    }

    /// Create a new workflow agent from a DAG workflow definition
    pub fn new_dag(definition: DagWorkflowDefinition, tools: Vec<Arc<dyn Tool>>) -> Self {
        let config = distri_types::configuration::AgentConfig::DagWorkflowAgent(definition);
        Self::new(config, tools)
    }

    /// Create a new workflow agent from a custom TypeScript agent definition
    pub fn new_custom(definition: CustomAgentDefinition, tools: Vec<Arc<dyn Tool>>) -> Self {
        let config = distri_types::configuration::AgentConfig::CustomAgent(definition);
        Self::new(config, tools)
    }

    /// Execute a sequential workflow
    async fn execute_sequential(
        &self,
        definition: &SequentialWorkflowDefinition,
        context: Arc<ExecutorContext>,
    ) -> Result<InvokeResult, AgentError> {
        let mut results = HashMap::new();
        let _all_tool_calls: Vec<ToolCall> = Vec::new(); // Reserved for future use
        let mut output_messages = Vec::new();

        info!("Starting sequential workflow: {}", definition.name);

        // Send workflow started event
        {
            let event_tx = context.event_tx.as_ref();
            if let Some(tx) = event_tx {
                let _ = tx
                    .send(AgentEvent::with_context(
                        AgentEventType::WorkflowStarted {
                            workflow_name: definition.name.clone(),
                            total_steps: definition.steps.len(),
                        },
                        "default".to_string(), // thread_id
                        context.run_id.clone(),
                        context.task_id.clone(),
                        self.name.clone(),
                    ))
                    .await;
            }
        }

        for (step_index, step) in definition.steps.iter().enumerate() {
            let step_name = match step {
                WorkflowStep::Tool {
                    name, tool_name, ..
                } => name.as_ref().unwrap_or(tool_name),
                WorkflowStep::Agent { name, agent, .. } => {
                    let agent_name = match agent {
                        AgentRef::Name(name) => name,
                        AgentRef::Definition(def) => &def.name,
                    };
                    name.as_ref().unwrap_or(agent_name)
                }
            };
            debug!("Executing step {}: {}", step_index, step_name);

            // Send step started event
            {
                let event_tx = context.event_tx.as_ref();
                if let Some(tx) = event_tx.as_ref() {
                    let _ = tx
                        .send(AgentEvent::with_context(
                            AgentEventType::NodeStarted {
                                node_id: step_name.clone(),
                                node_name: step_name.clone(),
                                step_type: match step {
                                    WorkflowStep::Tool { .. } => "Tool".to_string(),
                                    WorkflowStep::Agent { .. } => "Agent".to_string(),
                                },
                            },
                            "default".to_string(),
                            context.run_id.clone(),
                            context.task_id.clone(),
                            self.name.clone(),
                        ))
                        .await;
                }
            }

            let step_result = self
                .execute_workflow_step(step, &results, context.clone())
                .await;

            match step_result {
                Ok(result) => {
                    results.insert(step_name.clone(), result.clone());
                    output_messages.push(format!("Step '{}': SUCCESS", step_name));

                    // Send step completed event
                    {
                        let event_tx = context.event_tx.as_ref();
                        if let Some(tx) = event_tx.as_ref() {
                            let _ = tx
                                .send(AgentEvent::with_context(
                                    AgentEventType::NodeCompleted {
                                        node_id: step_name.clone(),
                                        node_name: step_name.clone(),
                                        success: true,
                                        error: None,
                                    },
                                    "default".to_string(),
                                    context.run_id.clone(),
                                    context.task_id.clone(),
                                    self.name.clone(),
                                ))
                                .await;
                        }
                    }
                }
                Err(error) => {
                    error!("Step '{}' failed: {}", step_name, error);
                    output_messages.push(format!("Step '{}': FAILED - {}", step_name, error));

                    // Send step failed event
                    {
                        let event_tx = context.event_tx.as_ref();
                        if let Some(tx) = event_tx.as_ref() {
                            let _ = tx
                                .send(AgentEvent::with_context(
                                    AgentEventType::NodeCompleted {
                                        node_id: step_name.clone(),
                                        node_name: step_name.clone(),
                                        success: false,
                                        error: Some(error.to_string()),
                                    },
                                    "default".to_string(),
                                    context.run_id.clone(),
                                    context.task_id.clone(),
                                    self.name.clone(),
                                ))
                                .await;
                        }
                    }

                    // For now, fail the entire workflow if any step fails
                    return Err(AgentError::WorkflowExecutionFailed(error.to_string()));
                }
            }
        }

        // Send workflow completed event
        {
            let event_tx = context.event_tx.as_ref();
            if let Some(tx) = event_tx.as_ref() {
                let _ = tx
                    .send(AgentEvent::with_context(
                        AgentEventType::RunCompleted {
                            workflow_name: definition.name.clone(),
                            success: true,
                            total_steps: definition.steps.len(),
                        },
                        "default".to_string(),
                        context.run_id.clone(),
                        context.task_id.clone(),
                        self.name.clone(),
                    ))
                    .await;
            }
        }

        info!("Sequential workflow completed: {}", definition.name);

        Ok(InvokeResult {
            content: Some(output_messages.join("\n")),
            tool_calls: vec![], // TODO: Collect actual tool calls if needed
        })
    }

    /// Execute a single workflow step
    async fn execute_workflow_step(
        &self,
        step: &WorkflowStep,
        previous_results: &HashMap<String, Value>,
        context: Arc<ExecutorContext>,
    ) -> Result<Value, AgentError> {
        match step {
            WorkflowStep::Tool {
                name: _,
                tool_name,
                input,
            } => {
                // Find the tool
                debug!(
                    "Looking for tool '{}' in {} available tools",
                    tool_name,
                    self.tools.len()
                );
                for tool in &self.tools {
                    debug!("Available tool: {}", tool.get_name());
                }
                let tool = self
                    .tools
                    .iter()
                    .find(|t| t.get_name() == *tool_name)
                    .ok_or_else(|| AgentError::ToolNotFound(tool_name.clone()))?;

                // Create tool call
                let tool_call = ToolCall {
                    tool_call_id: Uuid::new_v4().to_string(),
                    tool_name: tool_name.clone(),
                    input: input.clone().unwrap_or_else(|| serde_json::json!({})),
                };

                // Execute tool using the centralized execution logic
                let result = crate::tools::execute_tool_with_executor_context(
                    tool.as_ref(),
                    tool_call,
                    context.clone(),
                )
                .await
                .map_err(|e| {
                    AgentError::ToolExecutionFailed(format!("Tool '{}' failed: {}", tool_name, e))
                })?;

                // Convert Vec<Part> back to Value for workflow compatibility
                let value = if result.len() == 1 {
                    match &result[0] {
                        distri_types::Part::Data(data) => data.clone(),
                        _ => serde_json::json!({"result": result}),
                    }
                } else {
                    serde_json::json!({"parts": result})
                };

                Ok(value)
            }
            WorkflowStep::Agent {
                name: _,
                agent,
                task,
            } => {
                // Process template substitutions in the task
                let processed_task = self.substitute_templates(task, previous_results)?;

                // Get orchestrator
                let orchestrator = context.orchestrator.as_ref().ok_or_else(|| {
                    AgentError::InvalidConfiguration(
                        "Orchestrator required for agent execution".to_string(),
                    )
                })?;

                // Handle agent reference (name or inline definition)
                let agent = match agent {
                    AgentRef::Name(agent_name) => {
                        // Get existing agent by name
                        let agent_config =
                            orchestrator.get_agent(agent_name).await.ok_or_else(|| {
                                AgentError::AgentNotFound(format!(
                                    "Agent '{}' not found",
                                    agent_name
                                ))
                            })?;
                        orchestrator
                            .create_agent_from_config(agent_config, context.clone())
                            .await?
                    }
                    AgentRef::Definition(agent_def) => {
                        // Create agent from inline definition
                        let agent_config = distri_types::configuration::AgentConfig::StandardAgent(
                            agent_def.clone(),
                        );
                        orchestrator
                            .create_agent_from_config(agent_config, context.clone())
                            .await?
                    }
                };

                // Create message for agent
                let message = distri_types::Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: None,
                    parts: vec![distri_types::Part::Text(processed_task)],
                    role: distri_types::MessageRole::User,
                    created_at: chrono::Utc::now().timestamp_millis(),
                    agent_id: None,
                };

                // Execute agent
                let result = agent.invoke_stream(message, context.clone()).await?;

                // Return the agent's content response
                Ok(serde_json::json!({
                    "agent": match agent.get_definition() {
                        distri_types::configuration::AgentConfig::StandardAgent(def) => def.name,
                        config => config.get_name().to_string(),
                    },
                    "response": result.content.unwrap_or_default(),
                    "tool_calls": result.tool_calls.len()
                }))
            }
        }
    }

    /// Substitute template variables in strings like {{step[0].result}}
    fn substitute_templates(
        &self,
        template: &str,
        results: &HashMap<String, Value>,
    ) -> Result<String, AgentError> {
        let mut output = template.to_string();

        // Simple template substitution for now
        // TODO: Implement proper template parsing for {{step[0].result}} syntax

        // For now, just replace step names directly
        for (step_name, result) in results {
            let placeholder = format!("{{{{ step.{}.result }}}}", step_name);
            if let Ok(result_str) = serde_json::to_string(result) {
                output = output.replace(&placeholder, &result_str);
            }
        }

        Ok(output)
    }

    /// Execute a DAG workflow (placeholder for now)
    async fn execute_dag(
        &self,
        definition: &DagWorkflowDefinition,
        _context: Arc<ExecutorContext>,
    ) -> Result<InvokeResult, AgentError> {
        // TODO: Implement proper DAG execution with dependency resolution
        info!(
            "DAG workflow execution not yet fully implemented: {}",
            definition.name
        );

        Ok(InvokeResult {
            content: Some(format!(
                "DAG workflow '{}' execution placeholder",
                definition.name
            )),
            tool_calls: vec![],
        })
    }

    /// Extract input data from message parts (handles both text and JSON data)
    fn extract_input_data(&self, message: &Message) -> Result<serde_json::Value, AgentError> {
        let text = message.as_text().unwrap_or_default();
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(value) => Ok(value),
            Err(e) => {
                tracing::warn!("Error parsing message as JSON: {}", e);
                Ok(serde_json::json!({"message": text}))
            }
        }
    }

    /// Execute a custom TypeScript agent using the plugin system
    async fn execute_custom(
        &self,
        definition: &CustomAgentDefinition,
        input_data: serde_json::Value,
        context: Arc<ExecutorContext>,
    ) -> Result<InvokeResult, AgentError> {
        info!("Executing custom TypeScript agent: {}", definition.name);

        // Get the orchestrator and plugin registry
        let orchestrator = context.orchestrator.as_ref().ok_or_else(|| {
            AgentError::InvalidConfiguration(
                "Orchestrator required for custom agent execution".to_string(),
            )
        })?;

        let dap_registry = orchestrator.plugin_registry.clone();

        // Determine package and workflow name
        let script_name = std::path::Path::new(&definition.script_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&definition.name);

        let (name_package, name_component) = definition
            .name
            .split_once('/')
            .map(|(pkg, workflow)| (Some(pkg.to_string()), workflow.to_string()))
            .unwrap_or((None, script_name.to_string()));

        let package_name = definition
            .package
            .clone()
            .or(name_package)
            .unwrap_or_else(|| "distri_local".to_string());

        let workflow_name = name_component;

        // Execute through plugin system directly with input data
        let registry = dap_registry.clone();
        let workflow_result = registry
            .plugin_system
            .execute_workflow(
                &package_name,
                &workflow_name,
                input_data.clone(),
                distri_plugin_executor::PluginContext {
                    call_id: context.run_id.clone(),
                    agent_id: Some(context.agent_id.clone()),
                    session_id: Some(context.session_id.clone()),
                    task_id: Some(context.task_id.clone()),
                    run_id: Some(context.run_id.clone()),
                    user_id: Some(context.user_id.clone()),
                    params: input_data,
                    secrets: std::collections::HashMap::new(), // TODO: Load secrets if needed for agent workflows
                    auth_session: None, // No auth session for workflow context
                },
            )
            .await
            .map_err(|e| AgentError::Execution(format!("Custom agent execution failed: {}", e)))?;

        // Convert plugin result to InvokeResult
        let result_str = serde_json::to_string_pretty(&workflow_result)
            .unwrap_or_else(|_| format!("Custom agent '{}' completed", definition.name));

        Ok(InvokeResult {
            content: Some(result_str),
            tool_calls: vec![],
        })
    }
}

#[async_trait::async_trait]
impl BaseAgent for WorkflowAgent {
    async fn validate(&self) -> Result<(), AgentError> {
        match &self.definition {
            distri_types::configuration::AgentConfig::SequentialWorkflowAgent(def) => {
                if def.steps.is_empty() {
                    return Err(AgentError::Validation(
                        "Sequential workflow must have at least one step".to_string(),
                    ));
                }
            }
            distri_types::configuration::AgentConfig::DagWorkflowAgent(def) => {
                if def.nodes.is_empty() {
                    return Err(AgentError::Validation(
                        "DAG workflow must have at least one node".to_string(),
                    ));
                }
                // TODO: Add cycle detection
            }
            distri_types::configuration::AgentConfig::CustomAgent(def) => {
                if def.script_path.is_empty() {
                    return Err(AgentError::Validation(
                        "Custom agent must have a script_path".to_string(),
                    ));
                }
            }
            distri_types::configuration::AgentConfig::StandardAgent(_) => {
                return Err(AgentError::Validation(
                    "StandardAgent should not use WorkflowAgent".to_string(),
                ));
            }
        }
        Ok(())
    }

    async fn invoke_stream(
        &self,
        message: Message,
        context: Arc<ExecutorContext>,
    ) -> Result<InvokeResult, AgentError> {
        debug!(
            "WorkflowAgent::invoke_stream called for {}",
            self.definition.get_name()
        );

        // Extract JSON data from message if present (for workflow agents that need structured input)
        let input_data = self.extract_input_data(&message)?;

        match &self.definition {
            distri_types::configuration::AgentConfig::SequentialWorkflowAgent(def) => {
                self.execute_sequential(def, context).await
            }
            distri_types::configuration::AgentConfig::DagWorkflowAgent(def) => {
                self.execute_dag(def, context).await
            }
            distri_types::configuration::AgentConfig::CustomAgent(def) => {
                self.execute_custom(def, input_data, context).await
            }
            distri_types::configuration::AgentConfig::StandardAgent(_) => {
                Err(AgentError::InvalidConfiguration(
                    "StandardAgent should not use WorkflowAgent".to_string(),
                ))
            }
        }
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(self.clone())
    }

    fn get_name(&self) -> &str {
        self.definition.get_name()
    }

    fn get_description(&self) -> &str {
        self.definition.get_description()
    }

    fn get_definition(&self) -> distri_types::configuration::AgentConfig {
        self.definition.clone()
    }

    fn get_tools(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.clone()
    }

    fn get_dag(&self) -> crate::agent::types::AgentDag {
        match &self.definition {
            distri_types::configuration::AgentConfig::SequentialWorkflowAgent(def) => {
                // Convert sequential steps to DAG nodes with linear dependencies
                let mut nodes = Vec::new();
                for (i, step) in def.steps.iter().enumerate() {
                    let node_id = format!("step_{}", i);
                    let dependencies = if i == 0 {
                        vec![] // First step has no dependencies
                    } else {
                        vec![format!("step_{}", i - 1)] // Depends on previous step
                    };

                    let (name, node_type, metadata) = match step {
                        WorkflowStep::Tool {
                            name,
                            tool_name,
                            input,
                        } => (
                            name.as_ref().unwrap_or(tool_name).clone(),
                            "tool".to_string(),
                            serde_json::json!({
                                "step_type": "tool",
                                "tool_name": tool_name,
                                "input": input
                            }),
                        ),
                        WorkflowStep::Agent { name, agent, task } => {
                            let agent_name = match agent {
                                AgentRef::Name(name) => name.clone(),
                                AgentRef::Definition(def) => def.name.clone(),
                            };
                            (
                                name.as_ref().unwrap_or(&agent_name).clone(),
                                "agent".to_string(),
                                serde_json::json!({
                                    "step_type": "agent",
                                    "agent": agent,
                                    "task": task
                                }),
                            )
                        }
                    };

                    let node = crate::agent::types::DagNode {
                        id: node_id,
                        name,
                        node_type,
                        dependencies,
                        metadata,
                    };
                    nodes.push(node);
                }

                crate::agent::types::AgentDag {
                    nodes,
                    agent_name: self.definition.get_name().to_string(),
                    description: self.definition.get_description().to_string(),
                }
            }
            distri_types::configuration::AgentConfig::DagWorkflowAgent(def) => {
                // Convert DAG workflow nodes directly to DAG representation
                let nodes = def
                    .nodes
                    .iter()
                    .map(|wf_node| {
                        let (id, name, node_type, dependencies, metadata) = match wf_node {
                            DagWorkflowNode::Tool {
                                id,
                                name,
                                tool_name,
                                input,
                                depends_on,
                            } => (
                                id.clone(),
                                name.clone(),
                                "tool".to_string(),
                                depends_on.clone(),
                                serde_json::json!({
                                    "step_type": "tool",
                                    "tool_name": tool_name,
                                    "input": input
                                }),
                            ),
                            DagWorkflowNode::Agent {
                                id,
                                name,
                                agent_name,
                                task,
                                depends_on,
                            } => (
                                id.clone(),
                                name.clone(),
                                "agent".to_string(),
                                depends_on.clone(),
                                serde_json::json!({
                                    "step_type": "agent",
                                    "agent_name": agent_name,
                                    "task": task
                                }),
                            ),
                        };

                        crate::agent::types::DagNode {
                            id,
                            name,
                            node_type,
                            dependencies,
                            metadata,
                        }
                    })
                    .collect();

                crate::agent::types::AgentDag {
                    nodes,
                    agent_name: self.definition.get_name().to_string(),
                    description: self.definition.get_description().to_string(),
                }
            }
            distri_types::configuration::AgentConfig::CustomAgent(def) => {
                // Custom agents are single TypeScript execution blocks
                let node = crate::agent::types::DagNode {
                    id: "custom_agent_execution".to_string(),
                    name: def.name.clone(),
                    node_type: "custom_agent".to_string(),
                    dependencies: vec![],
                    metadata: serde_json::json!({
                        "agent_type": "custom",
                        "script_path": def.script_path,
                        "parameters": def.parameters
                    }),
                };

                crate::agent::types::AgentDag {
                    nodes: vec![node],
                    agent_name: self.definition.get_name().to_string(),
                    description: self.definition.get_description().to_string(),
                }
            }
            distri_types::configuration::AgentConfig::StandardAgent(_) => {
                // StandardAgent should not use WorkflowAgent, but provide fallback
                let node = crate::agent::types::DagNode {
                    id: "standard_agent_execution".to_string(),
                    name: self.definition.get_name().to_string(),
                    node_type: "standard_agent".to_string(),
                    dependencies: vec![],
                    metadata: serde_json::json!({
                        "agent_type": "standard"
                    }),
                };

                crate::agent::types::AgentDag {
                    nodes: vec![node],
                    agent_name: self.definition.get_name().to_string(),
                    description: self.definition.get_description().to_string(),
                }
            }
        }
    }
}
