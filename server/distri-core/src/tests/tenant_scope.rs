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

    // a pending call whose task the caller's store does not return
    let mut rx = orch
        .stores
        .external_tool_calls_store
        .register_external_tool_call("call-1", "task-of-someone-else")
        .await
        .unwrap();
    assert!(orch
        .clone()
        .complete_tool("call-1", tool_response("call-1"))
        .await
        .is_err());
    assert!(rx.try_recv().is_err(), "the run must not receive a result");

    // a pending call of a task the caller can read completes, and the run receives the result
    let thread = orch
        .create_thread(distri_types::CreateThreadRequest {
            agent_id: "agent".to_string(),
            title: None,
            thread_id: None,
            attributes: None,
            user_id: None,
            external_id: None,
            channel_id: None,
        })
        .await
        .unwrap();
    let task = orch
        .stores
        .task_store
        .create_task(distri_types::stores::CreateTaskInput::local(&thread.id))
        .await
        .unwrap();
    let rx_own = orch
        .stores
        .external_tool_calls_store
        .register_external_tool_call("call-own", &task.id)
        .await
        .unwrap();
    orch.clone()
        .complete_tool("call-own", tool_response("call-own"))
        .await
        .unwrap();
    assert_eq!(rx_own.await.unwrap().tool_call_id, "call-own");

    // a call with no pending record is refused on a multi-tenant host
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

#[tokio::test]
async fn artifacts_written_before_the_tenant_scope_move_under_their_tenant() {
    let dir = tempfile::tempdir().unwrap();
    // written by a server with no tenant scope: at the shared root
    let before = orchestrator(dir.path(), None).await;
    let old = before.session_filesystem().unwrap();
    let ns = "threads/0000aaaa/tasks/0000bbbb/content";
    old.write(&format!("{ns}/note.txt"), "alice's note")
        .await
        .unwrap();
    old.write_binary(&format!("{ns}/shot.png"), &[0x89, 0x50, 0x4e, 0x47, 0, 255])
        .await
        .unwrap();
    old.write(
        "threads/0000cccc/tasks/0000dddd/content/orphan.txt",
        "unknown thread",
    )
    .await
    .unwrap();

    let after = orchestrator(dir.path(), Some(Arc::new(Tenant("a")))).await;
    let moved = after
        .migrate_unscoped_artifacts(|hashes| async move {
            assert!(hashes.contains(&"0000aaaa".to_string()));
            Ok(std::collections::HashMap::from([(
                "0000aaaa".to_string(),
                "workspaces/a".to_string(),
            )]))
        })
        .await
        .unwrap();
    assert_eq!(moved, 2);

    let fs = after.session_filesystem().unwrap();
    assert_eq!(
        fs.read_binary(&format!("{ns}/note.txt")).await.unwrap(),
        b"alice's note".to_vec()
    );
    assert_eq!(
        fs.read_binary(&format!("{ns}/shot.png")).await.unwrap(),
        vec![0x89, 0x50, 0x4e, 0x47, 0, 255]
    );
    // gone from the shared root; an unknown thread's stay where they were
    assert!(old
        .read(&format!("{ns}/note.txt"), Default::default())
        .await
        .is_err());
    assert!(old
        .read(
            "threads/0000cccc/tasks/0000dddd/content/orphan.txt",
            Default::default()
        )
        .await
        .is_ok());

    // running it again asks only about the folder that still holds files (the emptied one is
    // not asked about), and the host may move the unknown thread's files elsewhere
    let again = after
        .migrate_unscoped_artifacts(|hashes| async move {
            assert_eq!(hashes, vec!["0000cccc".to_string()]);
            Ok(std::collections::HashMap::from([(
                "0000cccc".to_string(),
                "orphaned".to_string(),
            )]))
        })
        .await
        .unwrap();
    assert_eq!(again, 1);

    // with no files left at the root, the host is not asked at all
    let none = after
        .migrate_unscoped_artifacts(|_| async { panic!("asked with nothing to move") })
        .await
        .unwrap();
    assert_eq!(none, 0);
}

/// A thread folder that cannot be listed fails the move (to be retried on the next start)
/// instead of being taken for empty and skipped with its files left behind.
#[cfg(unix)]
#[tokio::test]
async fn an_unreadable_thread_folder_fails_the_move_instead_of_being_skipped() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let before = orchestrator(dir.path(), None).await;
    let old = before.session_filesystem().unwrap();
    old.write("threads/0000eeee/tasks/0000ffff/content/note.txt", "kept")
        .await
        .unwrap();
    let locked = dir.path().join("threads/0000eeee/tasks");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

    let after = orchestrator(dir.path(), Some(Arc::new(Tenant("a")))).await;
    let result = after
        .migrate_unscoped_artifacts(|_| async { panic!("asked although a folder was unreadable") })
        .await;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        result.is_err(),
        "an unreadable folder must not count as empty"
    );

    // readable again: the next run moves it
    let moved = after
        .migrate_unscoped_artifacts(|_| async {
            Ok(std::collections::HashMap::from([(
                "0000eeee".to_string(),
                "workspaces/a".to_string(),
            )]))
        })
        .await
        .unwrap();
    assert_eq!(moved, 1);
}
