//! `McpPoolProvider` — orchestrator-attached hook that builds the per-run
//! `McpClientPool` during tool resolution.
//!
//! Every agent run (cloud channel/bot gateway, CLI `POST /v1/agents/{id}`
//! JSON-RPC path, or tests) flows through `AgentOrchestrator::create_agent_from_config`
//! when its tools are resolved. That is the single chokepoint where the
//! orchestrator asks the attached provider for a pool, threads it into the
//! tool resolver, and stores it inside each `McpToolAdapter` so subsequent
//! `tools/call`s reuse the same live connection.
//!
//! The OSS standalone server attaches no provider; agents there fall back to
//! the static `[[tools.mcp]]` registry only.

use std::sync::Arc;

use super::McpClientPool;
use crate::agent::ExecutorContext;

/// Builds a per-run `McpClientPool` for an agent execution.
///
/// Implementations enumerate the host's MCP-kind connections visible to the
/// run's workspace (system-seeded + workspace-owned) and resolve each
/// connection's auth into `McpServerHandle::resolved_headers`. The pool is
/// scoped to a single run — `connect_named` caches one rmcp connection per
/// server for the lifetime of the pool.
///
/// Returning `None` is valid (no workspace, no connections, or the host
/// chooses to opt out for this particular context).
#[async_trait::async_trait]
pub trait McpPoolProvider: Send + Sync {
    async fn build_pool(&self, ctx: &ExecutorContext) -> Option<Arc<McpClientPool>>;
}
