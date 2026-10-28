//! Routing index from declared workflow triggers back to the
//! `(agent_id, entry_point_id)` that should be invoked.
//!
//! Built (or rebuilt) on orchestrator boot from every
//! `WorkflowAgentDefinition` in the agent store, and updated when an
//! agent upserts. The registry is what makes inbound stimuli —
//! `/v1/workflows/webhook/{path}` requests, the scheduler tick,
//! event-bus publishes, A2A `message/send` to a workflow-as-tool —
//! resolve back to a concrete workflow run.
//!
//! Implementations: in-memory (tests + OSS server-cli); Redis-backed
//! follows for the cloud (so multiple cloud processes see the same
//! routing without each having to rebuild on boot).

use crate::types::WorkflowDefinition;
use distri_types::WorkflowTrigger;
use std::collections::HashMap;

/// What a registry hit resolves to.
///
/// `workspace_id` is the tenant the agent belongs to (cloud); `None`
/// for OSS / single-tenant deployments. The webhook / scheduler /
/// event dispatchers use it to set the task-local workspace context
/// before spawning the run.
#[derive(Debug, Clone, PartialEq)]
pub struct TriggerBinding {
    pub agent_id: String,
    pub workspace_id: Option<String>,
    pub entry_point_id: String,
    pub trigger: WorkflowTrigger,
}

/// Persist and query the trigger -> (agent_id, entry_point_id) routing.
///
/// Bindings belong to an agent **within a workspace**: agent names are
/// only unique per workspace, so two workspaces can each register an agent
/// called `daily_report` without touching the other's triggers.
/// `workspace_id` is `None` for OSS / single-tenant deployments.
#[async_trait::async_trait]
pub trait WorkflowTriggerRegistry: Send + Sync {
    /// Register all triggers from an agent's workflow definition.
    /// Overwrites any previous bindings for this agent in this workspace
    /// (call after upsert).
    async fn register(
        &self,
        agent_id: &str,
        workspace_id: Option<&str>,
        def: &WorkflowDefinition,
    ) -> anyhow::Result<()>;

    /// Remove all bindings for an agent in a workspace.
    async fn unregister(&self, agent_id: &str, workspace_id: Option<&str>) -> anyhow::Result<()>;

    /// Resolve the binding for a `Webhook { path }` trigger. The
    /// webhook HTTP route maps `/v1/workflows/webhook/{path}` here; the
    /// path is the whole address, so it is global. The last agent to
    /// register a path owns it.
    async fn find_webhook(&self, path: &str) -> anyhow::Result<Option<TriggerBinding>>;

    /// Resolve the binding for a `Tool { name }` trigger (workflow
    /// exposed as an A2A skill) in a workspace.
    async fn find_tool(
        &self,
        tool_name: &str,
        workspace_id: Option<&str>,
    ) -> anyhow::Result<Option<TriggerBinding>>;

    /// All bindings for an `Event { topic }` trigger in a workspace.
    /// Returns every subscriber so the event bus can fan-out.
    async fn find_event(
        &self,
        topic: &str,
        workspace_id: Option<&str>,
    ) -> anyhow::Result<Vec<TriggerBinding>>;

    /// All bindings that are `Schedule { … }` triggers. The
    /// scheduler tick walks this list each interval and fires due
    /// runs.
    async fn list_schedules(&self) -> anyhow::Result<Vec<TriggerBinding>>;
}

/// The agent a set of bindings belongs to: `(workspace_id, agent_id)`.
type Owner = (Option<String>, String);

fn owner(agent_id: &str, workspace_id: Option<&str>) -> Owner {
    (workspace_id.map(str::to_string), agent_id.to_string())
}

/// In-memory registry: bindings per `(workspace_id, agent_id)`, scanned
/// for lookups. The Redis impl keeps the same ownership with per-kind
/// secondary indices.
#[derive(Default)]
pub struct InMemoryWorkflowTriggerRegistry {
    bindings: std::sync::Mutex<HashMap<Owner, Vec<TriggerBinding>>>,
}

impl InMemoryWorkflowTriggerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn collect_bindings(
        agent_id: &str,
        workspace_id: Option<&str>,
        def: &WorkflowDefinition,
    ) -> Vec<TriggerBinding> {
        let mut out = Vec::new();
        for ep in &def.entry_points {
            for trigger in &ep.triggers {
                out.push(TriggerBinding {
                    agent_id: agent_id.to_string(),
                    workspace_id: workspace_id.map(|s| s.to_string()),
                    entry_point_id: ep.id.clone(),
                    trigger: trigger.clone(),
                });
            }
        }
        out
    }
}

#[async_trait::async_trait]
impl WorkflowTriggerRegistry for InMemoryWorkflowTriggerRegistry {
    async fn register(
        &self,
        agent_id: &str,
        workspace_id: Option<&str>,
        def: &WorkflowDefinition,
    ) -> anyhow::Result<()> {
        let mut guard = self
            .bindings
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        guard.insert(
            owner(agent_id, workspace_id),
            Self::collect_bindings(agent_id, workspace_id, def),
        );
        Ok(())
    }

    async fn unregister(&self, agent_id: &str, workspace_id: Option<&str>) -> anyhow::Result<()> {
        let mut guard = self
            .bindings
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        guard.remove(&owner(agent_id, workspace_id));
        Ok(())
    }

    async fn find_webhook(&self, path: &str) -> anyhow::Result<Option<TriggerBinding>> {
        let guard = self
            .bindings
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        for entries in guard.values() {
            for binding in entries {
                if let WorkflowTrigger::Webhook { path: p, .. } = &binding.trigger {
                    if p == path {
                        return Ok(Some(binding.clone()));
                    }
                }
            }
        }
        Ok(None)
    }

    async fn find_tool(
        &self,
        tool_name: &str,
        workspace_id: Option<&str>,
    ) -> anyhow::Result<Option<TriggerBinding>> {
        let guard = self
            .bindings
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let in_workspace = guard
            .iter()
            .filter(|((ws, _), _)| ws.as_deref() == workspace_id)
            .map(|(_, entries)| entries);
        for entries in in_workspace {
            for binding in entries {
                if let WorkflowTrigger::Tool { name, .. } = &binding.trigger {
                    if name == tool_name {
                        return Ok(Some(binding.clone()));
                    }
                }
            }
        }
        Ok(None)
    }

    async fn find_event(
        &self,
        topic: &str,
        workspace_id: Option<&str>,
    ) -> anyhow::Result<Vec<TriggerBinding>> {
        let guard = self
            .bindings
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let mut out = Vec::new();
        let in_workspace = guard
            .iter()
            .filter(|((ws, _), _)| ws.as_deref() == workspace_id)
            .map(|(_, entries)| entries);
        for entries in in_workspace {
            for binding in entries {
                if let WorkflowTrigger::Event { topic: t, .. } = &binding.trigger {
                    if t == topic {
                        out.push(binding.clone());
                    }
                }
            }
        }
        Ok(out)
    }

    async fn list_schedules(&self) -> anyhow::Result<Vec<TriggerBinding>> {
        let guard = self
            .bindings
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let mut out = Vec::new();
        for entries in guard.values() {
            for binding in entries {
                if matches!(&binding.trigger, WorkflowTrigger::Schedule { .. }) {
                    out.push(binding.clone());
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EntryPoint, WorkflowDefinition, WorkflowStep};
    use distri_types::workflow_triggers::WebhookAuth;

    fn def_with(triggers: Vec<WorkflowTrigger>) -> WorkflowDefinition {
        WorkflowDefinition::new(vec![WorkflowStep::checkpoint("s", "S", "ok")]).with_entry_points(
            vec![EntryPoint {
                id: "main".into(),
                label: "Main".into(),
                description: None,
                starts_at: "s".into(),
                preset_results: Default::default(),
                required_inputs: vec![],
                triggers,
            }],
        )
    }

    #[tokio::test]
    async fn register_then_find_webhook() {
        let reg = InMemoryWorkflowTriggerRegistry::new();
        let def = def_with(vec![WorkflowTrigger::Webhook {
            path: "github".into(),
            methods: vec!["POST".into()],
            auth: WebhookAuth::None,
            response: Default::default(),
        }]);
        reg.register("agent-1", None, &def).await.unwrap();

        let hit = reg.find_webhook("github").await.unwrap().unwrap();
        assert_eq!(hit.agent_id, "agent-1");
        assert_eq!(hit.entry_point_id, "main");

        assert!(reg.find_webhook("missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn register_then_find_tool() {
        let reg = InMemoryWorkflowTriggerRegistry::new();
        let def = def_with(vec![WorkflowTrigger::Tool {
            name: "summarize".into(),
            description: "summarize a document".into(),
            input_schema: None,
        }]);
        reg.register("wf-summarize", None, &def).await.unwrap();

        let hit = reg.find_tool("summarize", None).await.unwrap().unwrap();
        assert_eq!(hit.agent_id, "wf-summarize");

        assert!(reg.find_tool("nope", None).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn find_event_fans_out() {
        let reg = InMemoryWorkflowTriggerRegistry::new();
        let def_a = def_with(vec![WorkflowTrigger::Event {
            topic: "user.signup".into(),
            filter: None,
        }]);
        let def_b = def_with(vec![WorkflowTrigger::Event {
            topic: "user.signup".into(),
            filter: None,
        }]);
        reg.register("agent-a", None, &def_a).await.unwrap();
        reg.register("agent-b", None, &def_b).await.unwrap();

        let hits = reg.find_event("user.signup", None).await.unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[tokio::test]
    async fn list_schedules_returns_only_schedule_triggers() {
        let reg = InMemoryWorkflowTriggerRegistry::new();
        let def = def_with(vec![
            WorkflowTrigger::Schedule {
                cron: "0 * * * *".into(),
                timezone: None,
                enabled: true,
                input: None,
                deliver_to: None,
            },
            WorkflowTrigger::Manual,
        ]);
        reg.register("nightly", None, &def).await.unwrap();

        let sched = reg.list_schedules().await.unwrap();
        assert_eq!(sched.len(), 1);
        assert!(matches!(sched[0].trigger, WorkflowTrigger::Schedule { .. }));
    }

    #[tokio::test]
    async fn unregister_clears_bindings() {
        let reg = InMemoryWorkflowTriggerRegistry::new();
        let def = def_with(vec![WorkflowTrigger::Webhook {
            path: "stripe".into(),
            methods: vec![],
            auth: WebhookAuth::None,
            response: Default::default(),
        }]);
        reg.register("billing", None, &def).await.unwrap();
        assert!(reg.find_webhook("stripe").await.unwrap().is_some());

        reg.unregister("billing", None).await.unwrap();
        assert!(reg.find_webhook("stripe").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn register_overwrites_previous_bindings_for_agent() {
        let reg = InMemoryWorkflowTriggerRegistry::new();
        let def_v1 = def_with(vec![WorkflowTrigger::Webhook {
            path: "v1".into(),
            methods: vec![],
            auth: WebhookAuth::None,
            response: Default::default(),
        }]);
        reg.register("api", None, &def_v1).await.unwrap();
        assert!(reg.find_webhook("v1").await.unwrap().is_some());

        // Re-register with a different path — v1 should disappear.
        let def_v2 = def_with(vec![WorkflowTrigger::Webhook {
            path: "v2".into(),
            methods: vec![],
            auth: WebhookAuth::None,
            response: Default::default(),
        }]);
        reg.register("api", None, &def_v2).await.unwrap();
        assert!(reg.find_webhook("v1").await.unwrap().is_none());
        assert!(reg.find_webhook("v2").await.unwrap().is_some());
    }

    fn nightly() -> WorkflowDefinition {
        def_with(vec![
            WorkflowTrigger::Schedule {
                cron: "0 0 8 * * *".into(),
                timezone: None,
                enabled: true,
                input: None,
                deliver_to: None,
            },
            WorkflowTrigger::Tool {
                name: "brief".into(),
                description: "morning brief".into(),
                input_schema: None,
            },
        ])
    }

    #[tokio::test]
    async fn same_agent_name_in_two_workspaces_stays_apart() {
        let reg = InMemoryWorkflowTriggerRegistry::new();
        reg.register("daily", Some("ws-a"), &nightly())
            .await
            .unwrap();
        reg.register("daily", Some("ws-b"), &nightly())
            .await
            .unwrap();

        let mut owners: Vec<_> = reg
            .list_schedules()
            .await
            .unwrap()
            .into_iter()
            .map(|b| b.workspace_id)
            .collect();
        owners.sort();
        assert_eq!(
            owners,
            vec![Some("ws-a".to_string()), Some("ws-b".to_string())]
        );

        let hit = reg.find_tool("brief", Some("ws-b")).await.unwrap().unwrap();
        assert_eq!(hit.workspace_id.as_deref(), Some("ws-b"));
        assert!(reg
            .find_tool("brief", Some("ws-c"))
            .await
            .unwrap()
            .is_none());

        // Removing one workspace's agent leaves the other's schedule.
        reg.unregister("daily", Some("ws-a")).await.unwrap();
        let left = reg.list_schedules().await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].workspace_id.as_deref(), Some("ws-b"));
    }
}
