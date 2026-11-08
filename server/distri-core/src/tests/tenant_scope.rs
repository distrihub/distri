//! A multi-tenant host's [`TenantScope`]: the session filesystem is the tenant's own, and a
//! pending external tool call or inline hook is completed only by a caller whose task store
//! returns its task (#216 in distri-cloud).

use std::sync::Arc;

use async_trait::async_trait;
use distri_types::filesystem::FileSystemOps;
use distri_types::{HookMutation, Part, TenantScope, ToolResponse};

use crate::agent::orchestrator::PendingInlineHook;
use crate::tests::helpers::test_store_config;
use crate::{AgentOrchestrator, AgentOrchestratorBuilder};

/// A host whose tenant is fixed, that lets every caller in.
#[derive(Debug)]
struct Tenant(&'static str);

#[async_trait]
impl TenantScope for Tenant {
    fn artifact_root_prefix(&self) -> anyhow::Result<String> {
        Ok(format!("workspaces/{}", self.0))
    }
    async fn allows_artifact_namespace(&self, _namespace: &str) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn allows_session(&self, _session_id: &str) -> anyhow::Result<bool> {
        Ok(true)
    }
}

/// A host with no tenant in context.
#[derive(Debug)]
struct NoTenant;

#[async_trait]
impl TenantScope for NoTenant {
    fn artifact_root_prefix(&self) -> anyhow::Result<String> {
        Err(anyhow::anyhow!("Workspace context required"))
    }
    async fn allows_artifact_namespace(&self, _namespace: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
    async fn allows_session(&self, _session_id: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
}

async fn orchestrator(
    storage: &std::path::Path,
    scope: Option<Arc<dyn TenantScope>>,
) -> Arc<AgentOrchestrator> {
    let mut builder = AgentOrchestratorBuilder::default()
        .with_store_config(test_store_config())
        .with_session_storage_path(storage.to_path_buf());
    if let Some(scope) = scope {
        builder = builder.with_tenant_scope(scope);
    }
    Arc::new(builder.build().await.unwrap())
}

fn tool_response(id: &str) -> ToolResponse {
    ToolResponse {
        tool_call_id: id.to_string(),
        tool_name: "client_tool".to_string(),
        parts: vec![Part::Text("done".to_string())],
        parts_metadata: None,
    }
}

#[tokio::test]
async fn each_tenant_has_its_own_session_filesystem() {
    let dir = tempfile::tempdir().unwrap();
    let a = orchestrator(dir.path(), Some(Arc::new(Tenant("a")))).await;
    let b = orchestrator(dir.path(), Some(Arc::new(Tenant("b")))).await;
    let ns = "threads/0000aaaa/tasks/0000bbbb/content";

    let fs_a = a.session_filesystem().unwrap();
    fs_a.write(&format!("{ns}/secret.txt"), "tenant a")
        .await
        .unwrap();
    assert!(dir
        .path()
        .join("workspaces/a")
        .join(ns)
        .join("secret.txt")
        .exists());

    let fs_b = b.session_filesystem().unwrap();
    assert!(fs_b
        .read(&format!("{ns}/secret.txt"), Default::default())
        .await
        .is_err());
    assert!(fs_b
        .list("threads")
        .await
        .map(|l| l.entries.is_empty())
        .unwrap_or(true));

    // no tenant in context: no filesystem at all, never the shared root
    let none = orchestrator(dir.path(), Some(Arc::new(NoTenant))).await;
    assert!(none.session_filesystem().is_err());
    assert!(!none.artifact_namespace_allowed(ns).await.unwrap());
    assert!(!none.session_allowed("th-1").await.unwrap());

    // a single-tenant server keeps the one root
    let single = orchestrator(dir.path(), None).await;
    assert!(single.session_filesystem().is_ok());
    assert!(single.artifact_namespace_allowed(ns).await.unwrap());
}

#[tokio::test]
async fn a_pending_tool_call_or_hook_is_completed_only_through_a_readable_task() {
    let dir = tempfile::tempdir().unwrap();
    let orch = orchestrator(dir.path(), Some(Arc::new(Tenant("a")))).await;

    // a call this process registered, whose task the caller's store does not return
    let mut rx = orch
        .stores
        .external_tool_calls_store
        .register_external_tool_call("call-1")
        .await
        .unwrap();
    orch.pending_tool_call_tasks
        .insert("call-1".to_string(), "task-of-someone-else".to_string());
    assert!(orch
        .clone()
        .complete_tool("call-1", tool_response("call-1"))
        .await
        .is_err());
    assert!(rx.try_recv().is_err(), "the run must not receive a result");

    // a call this process never registered is refused on a multi-tenant host
    assert!(orch
        .clone()
        .complete_tool("call-unknown", tool_response("call-unknown"))
        .await
        .is_err());

    // an inline hook of a task the caller cannot read: refused, and still pending
    let (tx, mut hook_rx) = tokio::sync::oneshot::channel();
    orch.inline_hooks.insert(
        "hook-1".to_string(),
        PendingInlineHook {
            task_id: "task-of-someone-else".to_string(),
            sender: tx,
        },
    );
    assert!(orch
        .complete_inline_hook("hook-1", HookMutation::none())
        .await
        .is_err());
    assert!(orch.inline_hooks.contains_key("hook-1"));
    assert!(hook_rx.try_recv().is_err());
}
