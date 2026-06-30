//! Integration tests for `ExecutorContext::preload_skills` — the
//! metadata-driven skill auto-load that runs at task start so a run reaches the
//! actual task without a `load_skill` round-trip.

use super::helpers::make_test_context;
use distri_types::stores::{ContextExecutionType, NewSkill};

/// An inline skill named in `load_skills` is rendered and injected up-front as
/// a SkillContext scratchpad entry, and tracked for post-compaction reinjection.
#[tokio::test]
async fn preload_injects_inline_skill_at_startup() {
    let ctx = make_test_context().await;
    let orchestrator = ctx.orchestrator.clone().expect("orchestrator");
    let skill_store = orchestrator
        .stores
        .skill_store
        .clone()
        .expect("skill store configured in test harness");

    let created = skill_store
        .create(NewSkill {
            name: "preload_inline".to_string(),
            description: Some("test inline skill".to_string()),
            content: "INLINE SKILL BODY: do the thing".to_string(),
            tags: vec![],
            model: None,
            context: ContextExecutionType::Inline,
        })
        .await
        .expect("create skill");

    let injected = ctx
        .preload_skills(&[created.id.clone()])
        .await
        .expect("preload_skills");

    assert_eq!(injected, vec![created.id.clone()], "inline skill injected");

    // The skill body landed in the scratchpad as a SkillContext entry.
    let entries = orchestrator
        .stores
        .scratchpad_store
        .get_entries(&ctx.thread_id, &ctx.task_id, None)
        .await
        .expect("get_entries");
    let has_skill_ctx = entries.iter().any(|e| {
        matches!(
            &e.entry_type,
            distri_types::ScratchpadEntryType::SkillContext(s)
                if s.content.contains("INLINE SKILL BODY")
        )
    });
    assert!(has_skill_ctx, "expected a SkillContext scratchpad entry");

    // And it's tracked for reinjection after compaction.
    let tracker = ctx.skill_tracker.read().await;
    assert!(
        !tracker.get_reinjection_candidates().is_empty(),
        "preloaded skill should be tracked for reinjection"
    );
}

/// A fork-type skill named in `load_skills` spawns an isolated child task at
/// startup (same thread, parent_task_id = current) — the metadata-driven
/// fork-as-subtask. The child body is NOT injected inline into the parent
/// (only the child's gist would be, on a successful run). In this no-LLM test
/// harness the child run can't reach an LLM, so we assert the dispatch side
/// effect that *is* deterministic: a child task row created under the parent.
#[tokio::test]
async fn preload_forks_fork_skill_as_child_task() {
    let ctx = make_test_context().await;
    let orchestrator = ctx.orchestrator.clone().expect("orchestrator");
    let skill_store = orchestrator
        .stores
        .skill_store
        .clone()
        .expect("skill store");

    // The fork now dispatches through invoke(), which resolves the agent
    // definition BEFORE persisting the child — so the running agent ("default",
    // from ExecutorContext::default) must be registered for the child row to be
    // created. (The child's loop then fails on the missing LLM, which preload
    // swallows — we only assert the deterministic persistence side effect.)
    orchestrator
        .register_agent_definition(distri_types::StandardDefinition {
            name: "default".to_string(),
            description: "preload fork test agent".to_string(),
            ..Default::default()
        })
        .await
        .expect("register default agent");

    // The parent thread + task must exist for the child's parent linkage.
    orchestrator
        .stores
        .thread_store
        .create_thread(distri_types::CreateThreadRequest {
            agent_id: "default".to_string(),
            title: Some("preload-fork".to_string()),
            thread_id: Some(ctx.thread_id.clone()),
            attributes: None,
            user_id: None,
            external_id: None,
            channel_id: None,
        })
        .await
        .expect("create thread");
    orchestrator
        .stores
        .task_store
        .get_or_create_task(&ctx.thread_id, &ctx.task_id)
        .await
        .expect("create parent task");

    let created = skill_store
        .create(NewSkill {
            name: "preload_fork".to_string(),
            description: None,
            content: "fork body".to_string(),
            tags: vec![],
            model: None,
            context: ContextExecutionType::Fork,
        })
        .await
        .expect("create fork skill");

    // Best-effort: with no LLM configured the child run errors out, which
    // preload swallows (logged, not fatal) and returns no injected ids.
    let injected = ctx
        .preload_skills(&[created.id.clone()])
        .await
        .expect("preload_skills");
    assert!(
        injected.is_empty(),
        "fork skill body is never injected inline into the parent"
    );

    // The fork DID dispatch: a child task exists in the same thread with
    // parent_task_id == the parent task (persisted before the LLM is called).
    let tasks = orchestrator
        .stores
        .task_store
        .list_tasks(Some(&ctx.thread_id))
        .await
        .expect("list_tasks");
    let child = tasks
        .iter()
        .find(|t| t.id != ctx.task_id && t.parent_task_id.as_deref() == Some(ctx.task_id.as_str()));
    assert!(
        child.is_some(),
        "fork-type skill should have spawned a child task under the parent; tasks: {:?}",
        tasks.iter().map(|t| (&t.id, &t.parent_task_id)).collect::<Vec<_>>()
    );

    // And no inline skill body leaked into the parent's scratchpad.
    let entries = orchestrator
        .stores
        .scratchpad_store
        .get_entries(&ctx.thread_id, &ctx.task_id, None)
        .await
        .expect("get_entries");
    assert!(
        !entries.iter().any(|e| matches!(
            &e.entry_type,
            distri_types::ScratchpadEntryType::SkillContext(s) if s.content.contains("fork body")
        )),
        "fork skill body must not be injected inline into the parent"
    );
}

/// Unknown skill ids and an empty list are no-ops, not errors.
#[tokio::test]
async fn preload_skips_unknown_and_empty() {
    let ctx = make_test_context().await;

    let empty = ctx.preload_skills(&[]).await.expect("empty ok");
    assert!(empty.is_empty());

    let unknown = ctx
        .preload_skills(&["does-not-exist".to_string()])
        .await
        .expect("unknown id is skipped, not fatal");
    assert!(unknown.is_empty());
}
