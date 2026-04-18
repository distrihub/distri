//! Legacy `A2AHandler` adapter.
//!
//! The actual logic lives in `crate::a2a::service::A2AService`. This type is
//! kept so existing callers (distri-cloud gateway, distri-server routes) keep
//! compiling. New code should depend on `A2AService` directly.
//!
//! Every method here forwards 1:1 to the service.

use crate::a2a::service::{A2AService, BoxedSseStream, ServiceRequest};
use crate::a2a::A2AError;
use crate::agent::{AgentOrchestrator, ExecutorContext};
use crate::AgentError;
use distri_a2a::{AgentCard, JsonRpcRequest, JsonRpcResponse, Task};
use futures::future::Either;
use std::sync::Arc;

pub struct A2AHandler {
    executor: Arc<AgentOrchestrator>,
}

impl A2AHandler {
    pub fn new(executor: Arc<AgentOrchestrator>) -> Self {
        Self { executor }
    }

    /// Build an `AgentCard` for the given agent name.
    pub async fn agent_def_to_card(
        &self,
        agent_id: String,
        server_config: Option<distri_types::configuration::ServerConfig>,
    ) -> Result<AgentCard, A2AError> {
        let service = A2AService::new(self.executor.clone());
        service.agent_def_to_card(agent_id, server_config).await
    }

    /// Fetch a `Task` by id. Exists for legacy callers that bypass the JSON-RPC
    /// dispatcher.
    pub async fn get_task(&self, params: serde_json::Value) -> Result<Task, A2AError> {
        let service = A2AService::new(self.executor.clone());
        service.get_task(params).await
    }

    /// Build an `ExecutorContext` from a JSON-RPC message request. Preserved as
    /// a free function to match the previous `A2AHandler::get_executor_context`
    /// call-site that the gateway uses when it wants to hand-build the context
    /// before streaming.
    pub async fn get_executor_context(
        req: &JsonRpcRequest,
        agent_id: String,
        user_id: String,
        workspace_id: Option<String>,
        verbose: bool,
        orchestrator: Arc<AgentOrchestrator>,
    ) -> Result<ExecutorContext, AgentError> {
        let service = A2AService::new(orchestrator);
        // Leverage the service method via `send_message`-shaped construction —
        // but expose it through its own code path: we build a stub
        // `ServiceRequest` and extract the context via the private helper.
        // Since `get_executor_context` is a helper on `A2AService`, reconstruct
        // the same logic here rather than widening the service's public surface.
        service
            .build_executor_context(req, agent_id, user_id, workspace_id, verbose)
            .await
    }

    /// JSON-RPC dispatch. Thin forwarder to `A2AService::handle`.
    pub async fn handle_jsonrpc(
        &self,
        agent_id: String,
        user_id: String,
        workspace_id: Option<String>,
        req: JsonRpcRequest,
        executor_context: Option<ExecutorContext>,
        verbose: bool,
        workspace_model_settings: Option<distri_types::ModelSettings>,
    ) -> Either<BoxedSseStream, JsonRpcResponse> {
        let service = A2AService::new(self.executor.clone());
        service
            .handle(ServiceRequest {
                agent_id,
                user_id,
                workspace_id,
                req,
                executor_context,
                verbose,
                workspace_model_settings,
            })
            .await
    }
}

#[allow(unused_imports)]
pub use crate::a2a::service::map_agent_error;
#[allow(unused_imports)]
pub use crate::a2a::validate_message;
