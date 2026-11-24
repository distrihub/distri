//! The memory tools an agent with `[memory]` gets: `memory_read`, `memory_search`, and (unless
//! `read_only`) `memory_write` and `memory_forget`. They act in the agent's space and the
//! run's scope.
//!
//! Refusals carry a code: `forbidden` (`rules/`, pinned items, a read-only agent),
//! `out_of_scope` (another space or scope), `read_first` (editing an item not read in this
//! task), `not_found`.

use std::sync::Arc;

use async_trait::async_trait;
use distri_types::memory::{MemoryOp, MemoryPlan, MemoryStatus, MemoryUri, RULES_FOLDER};
use distri_types::{Part, Tool, ToolCall};
use serde::Deserialize;
use serde_json::json;

use super::{session_of, MemorySession};
use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::AgentError;

pub const MEMORY_READ: &str = "memory_read";
pub const MEMORY_SEARCH: &str = "memory_search";
pub const MEMORY_WRITE: &str = "memory_write";
pub const MEMORY_FORGET: &str = "memory_forget";

/// The tools for an agent's `[memory]`.
pub fn memory_tools(read_only: bool) -> Vec<Arc<dyn Tool>> {
    let mut tools: Vec<Arc<dyn Tool>> = vec![Arc::new(MemoryReadTool), Arc::new(MemorySearchTool)];
    if !read_only {
        tools.push(Arc::new(MemoryWriteTool));
        tools.push(Arc::new(MemoryForgetTool));
    }
    tools
}

fn err(msg: impl Into<String>) -> AgentError {
    AgentError::ToolExecution(msg.into())
}

async fn session(ctx: &ExecutorContext) -> Result<Arc<MemorySession>, AgentError> {
    session_of(ctx)
        .await
        .ok_or_else(|| err("memory is not enabled for this agent (no [memory] in its definition)"))
}

fn store(ctx: &ExecutorContext) -> Result<Arc<dyn distri_types::AgentMemoryStore>, AgentError> {
    ctx.orchestrator
        .as_ref()
        .map(|o| o.agent_memory_store.clone())
        .ok_or_else(|| err("memory: no orchestrator"))
}

fn scoped_uri(s: &MemorySession, raw: &str) -> Result<MemoryUri, AgentError> {
    let uri = s.uri(raw).map_err(|e| err(format!("invalid_uri: {e}")))?;
    if !s.in_scope(&uri) {
        return Err(err(format!(
            "out_of_scope: {uri} is outside this agent's memory (mem://{}/{})",
            s.space,
            s.scope()
        )));
    }
    if !uri.is_item() {
        return Err(err(format!(
            "invalid_uri: {uri} names a folder; add a slug (folder/slug)"
        )));
    }
    Ok(uri)
}

fn data(v: serde_json::Value) -> Vec<Part> {
    vec![Part::Data(v)]
}

fn input<T: for<'de> Deserialize<'de>>(call: &ToolCall) -> Result<T, AgentError> {
    serde_json::from_value(call.input.clone())
        .map_err(|e| err(format!("invalid {} input: {e}", call.tool_name)))
}

macro_rules! executor_only_tool {
    ($ty:ident, $name:expr, $desc:expr, $params:expr) => {
        #[derive(Debug, Clone)]
        pub struct $ty;

        #[async_trait]
        impl Tool for $ty {
            fn get_name(&self) -> String {
                $name.to_string()
            }
            fn get_description(&self) -> String {
                $desc.to_string()
            }
            fn get_parameters(&self) -> serde_json::Value {
                $params
            }
            fn needs_executor_context(&self) -> bool {
                true
            }
            async fn execute(
                &self,
                _tool_call: ToolCall,
                _context: Arc<distri_types::ToolContext>,
            ) -> Result<Vec<Part>, anyhow::Error> {
                anyhow::bail!("{} requires executor context", $name)
            }
        }
    };
}

executor_only_tool!(
    MemoryReadTool,
    MEMORY_READ,
    "Read one memory item in full (its L2 body, L1 overview and evidence). The memory block \
     in your instructions shows only one line per item: read an item before relying on its \
     detail, and before changing it with memory_write.",
    json!({
        "type": "object",
        "properties": {
            "uri": {"type": "string", "description": "mem://space/scope/folder/slug, or folder/slug"}
        },
        "required": ["uri"]
    })
);

executor_only_tool!(
    MemorySearchTool,
    MEMORY_SEARCH,
    "Search your memory (L0, L1 and body) by words. Returns matching URIs with their one-line \
     L0; read one with memory_read.",
    json!({
        "type": "object",
        "properties": {
            "query": {"type": "string"},
            "folder": {"type": "string", "description": "Only this folder (style, preferences, facts, cases, experiences, …)"},
            "limit": {"type": "integer", "minimum": 1, "maximum": 50}
        },
        "required": ["query"]
    })
);

executor_only_tool!(
    MemoryWriteTool,
    MEMORY_WRITE,
    "Create or replace a memory item that should outlive this run: a preference, a style \
     note, a fact, a case or an experience. l0 is one line (at most 200 characters) shown in \
     every future run; body is the full detail. To change an existing item, memory_read it \
     first in this run. rules/ and owner-pinned items cannot be written.",
    json!({
        "type": "object",
        "properties": {
            "uri": {"type": "string", "description": "mem://space/scope/folder/slug, or folder/slug (slug: lowercase-with-dashes)"},
            "l0": {"type": "string", "description": "One line, at most 200 characters"},
            "body": {"type": "string"},
            "l1": {"type": "string", "description": "Short overview (default: the body when short)"},
            "evidence": {"type": "array", "items": {"type": "string"}, "description": "What this rests on: run:<task_id>, sig:<id>, mem://…"}
        },
        "required": ["uri", "l0", "body"]
    })
);

executor_only_tool!(
    MemoryForgetTool,
    MEMORY_FORGET,
    "Forget a memory item that is wrong or outdated. It is archived, not deleted: the owner \
     can restore it. Say why.",
    json!({
        "type": "object",
        "properties": {
            "uri": {"type": "string"},
            "reason": {"type": "string"}
        },
        "required": ["uri", "reason"]
    })
);

#[derive(Deserialize)]
struct ReadInput {
    uri: String,
}

#[async_trait]
impl ExecutorContextTool for MemoryReadTool {
    async fn execute_with_executor_context(
        &self,
        call: ToolCall,
        ctx: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let s = session(&ctx).await?;
        let input: ReadInput = input(&call)?;
        let uri = scoped_uri(&s, &input.uri)?;
        let store = store(&ctx)?;
        let item = store
            .get(&s.workspace_id, &uri)
            .await
            .map_err(|e| err(format!("memory_read: {e}")))?
            .filter(|i| i.status != MemoryStatus::Archived)
            .ok_or_else(|| err(format!("not_found: {uri}")))?;
        s.mark_read(&ctx.task_id, &uri);
        if let Err(e) = store
            .mark_used(&s.workspace_id, std::slice::from_ref(&uri))
            .await
        {
            tracing::warn!(error = %e, "memory_read: mark_used failed");
        }
        Ok(data(json!({
            "uri": uri.to_string(),
            "l0": item.l0,
            "l1": item.l1,
            "body": item.body,
            "evidence": item.evidence,
            "status": item.status,
            "pinned": item.pinned,
            "version": item.version,
            "updated_at": item.updated_at,
        })))
    }
}

#[derive(Deserialize)]
struct SearchInput {
    query: String,
    #[serde(default)]
    folder: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl ExecutorContextTool for MemorySearchTool {
    async fn execute_with_executor_context(
        &self,
        call: ToolCall,
        ctx: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let s = session(&ctx).await?;
        let input: SearchInput = input(&call)?;
        let hits = store(&ctx)?
            .search(
                &s.workspace_id,
                &s.space,
                &s.scope(),
                &input.query,
                input.folder.as_deref(),
                input.limit.unwrap_or(10).clamp(1, 50),
            )
            .await
            .map_err(|e| err(format!("memory_search: {e}")))?;
        let results: Vec<_> = hits
            .iter()
            .map(|i| json!({"uri": i.uri.to_string(), "l0": i.l0}))
            .collect();
        Ok(data(json!({ "results": results })))
    }
}

#[derive(Deserialize)]
struct WriteInput {
    uri: String,
    l0: String,
    body: String,
    #[serde(default)]
    l1: Option<String>,
    #[serde(default)]
    evidence: Vec<String>,
}

#[async_trait]
impl ExecutorContextTool for MemoryWriteTool {
    async fn execute_with_executor_context(
        &self,
        call: ToolCall,
        ctx: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let s = session(&ctx).await?;
        if s.config.read_only {
            return Err(err("forbidden: this agent's memory is read-only"));
        }
        let input: WriteInput = input(&call)?;
        let uri = scoped_uri(&s, &input.uri)?;
        if uri.folder == RULES_FOLDER {
            return Err(err(format!(
                "forbidden: {uri}: rules/ is written by the owner only"
            )));
        }
        let folders = s.config.folders();
        if !folders.contains(&uri.folder) {
            return Err(err(format!(
                "unknown folder `{}`; use one of: {}",
                uri.folder,
                folders
                    .iter()
                    .filter(|f| *f != RULES_FOLDER)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let store = store(&ctx)?;
        let existing = store
            .get(&s.workspace_id, &uri)
            .await
            .map_err(|e| err(format!("memory_write: {e}")))?;
        if existing.as_ref().map(|i| i.pinned).unwrap_or(false) {
            return Err(err(format!("forbidden: {uri} is pinned by the owner")));
        }
        let live = existing.filter(|i| i.status != MemoryStatus::Archived);
        if live.is_some() && !s.was_read(&ctx.task_id, &uri) {
            return Err(err(format!(
                "read_first: {uri} exists; memory_read it in this run before changing it"
            )));
        }
        let op = if live.is_some() {
            MemoryOp::Replace {
                uri: uri.clone(),
                l0: input.l0,
                l1: input.l1,
                body: input.body,
                evidence: input.evidence,
                pinned: None,
            }
        } else {
            MemoryOp::Create {
                uri: uri.clone(),
                l0: input.l0,
                l1: input.l1,
                body: input.body,
                evidence: input.evidence,
                pinned: None,
            }
        };
        let op_name = op.name();
        let changes = store
            .apply(
                &s.workspace_id,
                &MemoryPlan {
                    ops: vec![op],
                    skipped: vec![],
                },
                &s.author(&ctx.task_id),
            )
            .await
            .map_err(|e| err(format!("memory_write: {e}")))?;
        // Written in this task counts as read: a later edit needs no extra read.
        s.mark_read(&ctx.task_id, &uri);
        Ok(data(json!({
            "uri": uri.to_string(),
            "op": op_name,
            "change_id": changes.first().map(|c| c.id.clone()),
        })))
    }
}

#[derive(Deserialize)]
struct ForgetInput {
    uri: String,
    #[serde(default)]
    reason: String,
}

#[async_trait]
impl ExecutorContextTool for MemoryForgetTool {
    async fn execute_with_executor_context(
        &self,
        call: ToolCall,
        ctx: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let s = session(&ctx).await?;
        if s.config.read_only {
            return Err(err("forbidden: this agent's memory is read-only"));
        }
        let input: ForgetInput = input(&call)?;
        let uri = scoped_uri(&s, &input.uri)?;
        if uri.folder == RULES_FOLDER {
            return Err(err(format!(
                "forbidden: {uri}: rules/ is written by the owner only"
            )));
        }
        if input.reason.trim().is_empty() {
            return Err(err("memory_forget needs a reason"));
        }
        let store = store(&ctx)?;
        let changes = store
            .apply(
                &s.workspace_id,
                &MemoryPlan {
                    ops: vec![MemoryOp::Archive {
                        uri: uri.clone(),
                        replacement: None,
                        reason: input.reason,
                    }],
                    skipped: vec![],
                },
                &s.author(&ctx.task_id),
            )
            .await
            .map_err(|e| err(format!("memory_forget: {e}")))?;
        Ok(data(json!({
            "uri": uri.to_string(),
            "archived": true,
            "change_id": changes.first().map(|c| c.id.clone()),
        })))
    }
}
