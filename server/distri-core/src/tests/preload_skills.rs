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

/// A fork-type skill is NOT eagerly injected (forking at startup is handled
/// separately); preload returns it as not-injected and writes no entry.
#[tokio::test]
async fn preload_defers_fork_skill() {
    let ctx = make_test_context().await;
    let orchestrator = ctx.orchestrator.clone().expect("orchestrator");
    let skill_store = orchestrator
        .stores
        .skill_store
        .clone()
        .expect("skill store");

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

    let injected = ctx
        .preload_skills(&[created.id.clone()])
        .await
        .expect("preload_skills");

    assert!(injected.is_empty(), "fork skills are not injected inline");

    let entries = orchestrator
        .stores
        .scratchpad_store
        .get_entries(&ctx.thread_id, &ctx.task_id, None)
        .await
        .expect("get_entries");
    assert!(
        !entries
            .iter()
            .any(|e| matches!(&e.entry_type, distri_types::ScratchpadEntryType::SkillContext(_))),
        "fork skill must not inject a SkillContext entry"
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
