//! Who called an agent, and a hook that sees every top-level agent request.
//!
//! The caller is established by the server's own auth layer or by the
//! server-side component that started the run (the channel gateway), never
//! by the request body. Cloud inserts a [`RequestCaller`] as an actix request
//! extension; `distri-server`'s A2A route reads it and passes it on.
//!
//! [`AgentRequestObserver`] is the extension point the cloud uses to refuse
//! requests the caller may not make (an embed token calling a server agent)
//! and to record which clients and tools reach each agent. Standalone
//! `distri-server` has no observer.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{RuntimeMode, StandardDefinition, ToolDefinition};

/// The kind of caller behind an agent request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallerKind {
    /// A browser embed authenticated with a public-client token.
    PublicClient,
    /// A channel message delivered through a bot.
    Bot,
    /// A workflow run or schedule.
    Workflow,
    /// The distri CLI.
    Cli,
    /// A signed-in workspace member or a workspace API key (dashboard, SDK).
    Dashboard,
}

impl CallerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PublicClient => "public_client",
            Self::Bot => "bot",
            Self::Workflow => "workflow",
            Self::Cli => "cli",
            Self::Dashboard => "dashboard",
        }
    }
}

/// Who sent an agent request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCaller {
    pub kind: CallerKind,
    /// `public_client_id`, `bot_id` or workflow id, depending on `kind`.
    pub source_id: Option<String>,
    /// Page origin for browser callers; channel provider for bots.
    pub origin: Option<String>,
    /// Client SDK name and version, when the client sends one.
    pub client: Option<String>,
}

/// One top-level agent request, after its runtime has been resolved.
pub struct AgentRequest<'a> {
    pub agent: &'a StandardDefinition,
    pub user_id: &'a str,
    pub workspace_id: Option<&'a str>,
    pub caller: Option<&'a RequestCaller>,
    /// The runtime the client claimed in its request metadata.
    pub claimed_runtime: &'a RuntimeMode,
    /// The runtime the request actually runs in.
    pub runtime: &'a RuntimeMode,
    /// External tools the client sent with this request.
    pub external_tools: &'a [ToolDefinition],
}

/// Sees every top-level agent request before it runs. Returning `Err`
/// refuses the request with that message.
#[async_trait]
pub trait AgentRequestObserver: Send + Sync + 'static {
    async fn on_request(&self, request: AgentRequest<'_>) -> Result<(), String>;
}
