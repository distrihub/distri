//! Type-level tests for `distri-workflow`.
//!
//! The old `WorkflowRunner` + `InMemoryStore` + `EventSink` + `StepExecutor`
//! abstractions are gone — the workflow execution loop lives in
//! `distri-core` (`agent/workflow_driver.rs`) and is exercised by the
//! `distri-core` test suite. What remains here is type-level: definition
//! shape, run aggregate helpers (`runnable_steps`, `is_complete`,
//! `is_stuck`, `apply_entry_point`, `with_input`), DAG cycle detection,
//! template resolution.

#[cfg(test)]
mod tests {
    use crate::*;
    use distri_types::TaskStatus;
    use std::collections::HashMap;

    fn wf_with_one_checkpoint() -> WorkflowDefinition {
        WorkflowDefinition::new(vec![WorkflowStep::checkpoint("ok", "Done", "ok")])
    }

    #[test]
    fn workflow_serializes_to_json() {
        let def = wf_with_one_checkpoint();
        let run = WorkflowRun::new(def);
        let json = serde_json::to_string(&run).unwrap();
        assert!(json.contains("\"status\":\"pending\""));
        assert!(json.contains("\"step_id\":\"ok\""));
    }

    #[test]
    fn empty_workflow_is_immediately_complete() {
        let run = WorkflowRun::new(WorkflowDefinition::new(vec![]));
        assert!(run.is_complete());
    }

    #[test]
    fn runnable_steps_respects_dependencies() {
        let def = WorkflowDefinition::new(vec![
            WorkflowStep::checkpoint("a", "A", "ok"),
            WorkflowStep::checkpoint("b", "B", "ok").with_depends_on(vec!["a"]),
        ]);
        let mut run = WorkflowRun::new(def);
        let runnable: Vec<String> = run
            .runnable_steps()
            .iter()
            .map(|(_, s)| s.id.clone())
            .collect();
        assert_eq!(runnable, vec!["a".to_string()]);

        run.step_runs[0].status = TaskStatus::Completed;
        let runnable: Vec<String> = run
            .runnable_steps()
            .iter()
            .map(|(_, s)| s.id.clone())
            .collect();
        assert_eq!(runnable, vec!["b".to_string()]);
    }

    #[test]
    fn is_stuck_when_blocked_dep() {
        let def = WorkflowDefinition::new(vec![
            WorkflowStep::checkpoint("a", "A", "ok"),
            WorkflowStep::checkpoint("b", "B", "ok").with_depends_on(vec!["a"]),
        ]);
        let mut run = WorkflowRun::new(def);
        run.step_runs[0].status = TaskStatus::Failed;
        run.step_runs[0].error = Some("upstream failure".into());
        assert!(run.is_stuck());
    }

    #[test]
    fn step_requirement_native_builder() {
        let req = StepRequirement::native("network");
        assert_eq!(req.skill, "native:network");
    }

    #[test]
    fn step_requirement_connection_builder() {
        let req =
            StepRequirement::connection("google", "drive").with_permissions(vec!["drive.readonly"]);
        assert_eq!(req.skill, "google:drive");
        assert_eq!(req.permissions, vec!["drive.readonly"]);
    }

    #[test]
    fn step_requirement_validation_rejects_unnamespaced() {
        let req = StepRequirement {
            skill: "network".into(),
            permissions: vec![],
            config: Default::default(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn detect_cycles_catches_back_edge() {
        let def = WorkflowDefinition::new(vec![
            WorkflowStep::checkpoint("a", "A", "ok").with_depends_on(vec!["b"]),
            WorkflowStep::checkpoint("b", "B", "ok").with_depends_on(vec!["a"]),
        ]);
        assert!(def.detect_cycles().is_err());
    }

    #[test]
    fn apply_entry_point_marks_unreachable_canceled() {
        let def = WorkflowDefinition::new(vec![
            WorkflowStep::checkpoint("a", "A", "ok"),
            WorkflowStep::checkpoint("b", "B", "ok").with_depends_on(vec!["a"]),
            WorkflowStep::checkpoint("c", "C", "ok"),
        ])
        .with_entry_points(vec![EntryPoint {
            id: "main".into(),
            label: "Main".into(),
            description: None,
            starts_at: "a".into(),
            preset_results: HashMap::new(),
            required_inputs: vec![],
            triggers: vec![],
        }]);

        let run = WorkflowRun::new(def).apply_entry_point("main").unwrap();
        let c_run = run.step_run_by_id("c").unwrap();
        assert_eq!(c_run.status, TaskStatus::Canceled);
        let a_run = run.step_run_by_id("a").unwrap();
        assert_eq!(a_run.status, TaskStatus::Pending);
    }

    #[test]
    fn with_input_validates_schema() {
        let mut def = WorkflowDefinition::new(vec![WorkflowStep::checkpoint("ok", "OK", "ok")]);
        def.input_schema = Some(serde_json::json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"]
        }));
        let run = WorkflowRun::new(def);
        let err = run
            .clone()
            .with_input(serde_json::json!({"other": 1}))
            .unwrap_err();
        assert!(err.contains("Input validation failed"));
        let ok = run.with_input(serde_json::json!({"name": "x"})).unwrap();
        assert_eq!(ok.status, TaskStatus::Running);
    }

    #[test]
    fn resolve_template_substitutes_input_steps_env() {
        let ctx = serde_json::json!({
            "input": {"doc_id": "abc"},
            "steps": {"fetch": {"size": 42}},
            "env": {"api": "http://x"}
        });
        assert_eq!(
            resolve::resolve_template("{env.api}/docs/{input.doc_id}", &ctx),
            "http://x/docs/abc"
        );
        let v = resolve::resolve_value(&serde_json::json!("{steps.fetch.size}"), &ctx);
        assert_eq!(v, serde_json::json!(42));
    }

    #[test]
    fn evaluate_skip_condition_truthy_and_falsy() {
        let ctx = serde_json::json!({"input": {"flag": true, "empty": ""}});
        assert!(resolve::evaluate_skip_condition("{input.flag}", &ctx));
        assert!(!resolve::evaluate_skip_condition("{input.empty}", &ctx));
        assert!(!resolve::evaluate_skip_condition("!{input.flag}", &ctx));
        assert!(resolve::evaluate_skip_condition("!{input.empty}", &ctx));
    }

    // ── phase 3 (#304): node kinds, for_each ────────────────────────────────

    fn step_of(v: serde_json::Value) -> WorkflowStep {
        serde_json::from_value(v).expect("step parses")
    }

    #[test]
    fn llm_node_parses_and_round_trips() {
        let s = step_of(serde_json::json!({
            "id": "judge", "label": "Judge",
            "kind": {"type": "llm", "prompt": "Rate {item.title}", "system": "be terse",
                     "output_schema": {"type": "object", "required": ["score"]},
                     "temperature": 0.2}
        }));
        match &s.kind {
            StepKind::Llm { prompt, system, output_schema, temperature, model, .. } => {
                assert_eq!(prompt, "Rate {item.title}");
                assert_eq!(system.as_deref(), Some("be terse"));
                assert!(output_schema.is_some());
                assert_eq!(*temperature, Some(0.2));
                assert!(model.is_none());
            }
            other => panic!("not llm: {other:?}"),
        }
        assert!(s.is_replay_safe());
        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["kind"]["type"], "llm");
        assert!(back.get("for_each").is_none());
    }

    #[test]
    fn person_node_parses_with_defaults() {
        let s = step_of(serde_json::json!({
            "id": "ok", "label": "Ask",
            "kind": {"type": "person", "ask": "Post {steps.d.title}?", "buttons": ["Yes", "No"],
                     "deadline_secs": 3600}
        }));
        match &s.kind {
            StepKind::Person { ask, to, buttons, input_schema, deadline_secs } => {
                assert_eq!(ask, "Post {steps.d.title}?");
                assert!(to.is_none());
                assert_eq!(buttons, &vec!["Yes".to_string(), "No".to_string()]);
                assert!(input_schema.is_none());
                assert_eq!(*deadline_secs, Some(3600));
            }
            other => panic!("not person: {other:?}"),
        }
        assert!(s.is_replay_safe());
        assert_eq!(s.for_each_refused(), Some("person"));
    }

    #[test]
    fn agent_alias_reads_as_agent_run() {
        let s = step_of(serde_json::json!({
            "id": "w", "label": "Write",
            "kind": {"type": "agent", "agent": "outreach_writer", "prompt": "draft {item.account}"}
        }));
        match &s.kind {
            StepKind::AgentRun { agent_id, prompt, .. } => {
                assert_eq!(agent_id, "outreach_writer");
                assert_eq!(prompt, "draft {item.account}");
            }
            other => panic!("not agent_run: {other:?}"),
        }
        // Written back in the canonical form.
        let back = serde_json::to_value(&s.kind).unwrap();
        assert_eq!(back["type"], "agent_run");
        assert_eq!(back["agent_id"], "outreach_writer");
        // The canonical form still parses.
        let again: StepKind = serde_json::from_value(back).unwrap();
        assert!(matches!(again, StepKind::AgentRun { .. }));
    }

    #[test]
    fn for_each_as_a_string_or_an_object() {
        let s = step_of(serde_json::json!({
            "id": "n", "label": "n", "for_each": "{steps.pick.items}",
            "kind": {"type": "notify", "text": "{index}: {item.title}"}
        }));
        let fe = s.for_each.clone().unwrap();
        assert_eq!(fe, ForEach::Expr("{steps.pick.items}".into()));
        assert_eq!(fe.items_expr(), "{steps.pick.items}");
        assert_eq!(fe.as_name(), "item");
        assert_eq!(fe.max(), FOR_EACH_DEFAULT_MAX);
        assert_eq!(fe.concurrency(), 1);

        let s = step_of(serde_json::json!({
            "id": "n", "label": "n",
            "for_each": {"in": "{input.accounts}", "as": "account", "max": 3, "concurrency": 99},
            "kind": {"type": "notify", "text": "{account.name}"}
        }));
        let fe = s.for_each.clone().unwrap();
        assert_eq!(fe.items_expr(), "{input.accounts}");
        assert_eq!(fe.as_name(), "account");
        assert_eq!(fe.max(), 3);
        let huge: ForEach =
            serde_json::from_value(serde_json::json!({"in": "{input.x}", "max": 100000})).unwrap();
        assert_eq!(huge.max(), FOR_EACH_HARD_MAX);
        assert_eq!(fe.concurrency(), FOR_EACH_MAX_CONCURRENCY);
        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["for_each"]["in"], "{input.accounts}");
        assert_eq!(back["for_each"]["as"], "account");
    }

    #[test]
    fn index_locals_and_array_paths_resolve() {
        let ctx = serde_json::json!({
            "steps": {"x": [{"name": "a"}, {"name": "b"}]},
            "item": {"title": "T"},
            "index": 1,
            resolve::LOCALS_KEY: {"account": {"name": "radish"}}
        });
        assert_eq!(resolve_template("{index}:{item.title}", &ctx), "1:T");
        assert_eq!(resolve_template("{steps.x.1.name}", &ctx), "b");
        assert_eq!(resolve_template("{account.name}", &ctx), "radish");
        assert_eq!(resolve_value(&serde_json::json!("{index}"), &ctx), serde_json::json!(1));
        // Unknown names stay as written.
        assert_eq!(resolve_template("{nope.x} {steps.x.9.name}", &ctx), "{nope.x} {steps.x.9.name}");
    }

    #[test]
    fn code_with_side_effecting_script_calls_is_not_replay_safe() {
        use distri_types::script_sdk::ScriptCallRecord;
        let code = WorkflowStep::code("c", "export default () => 1");
        let call = |op: &str, side_effect: bool| ScriptCallRecord {
            seq: 1,
            op: op.into(),
            at: chrono::Utc::now(),
            ok: true,
            side_effect,
            summary: String::new(),
            error: None,
        };
        assert!(code.is_replay_safe_after(&[]));
        assert!(code.is_replay_safe_after(&[call("state_set", false)]));
        assert!(!code.is_replay_safe_after(&[call("notify", true)]));
        let mut forced = code.clone();
        forced.replay_safe = Some(true);
        assert!(forced.is_replay_safe_after(&[call("notify", true)]));
    }

    #[test]
    fn pause_reason_person_and_step_rows_round_trip() {
        assert_eq!(PauseReason::Person.as_str(), "person");
        let v = serde_json::to_value(PauseReason::Person).unwrap();
        assert_eq!(v, "person");
        let row = WorkflowStepState {
            step_id: "s".into(),
            items: Some(vec![ForEachItem {
                index: 0,
                status: "completed".into(),
                result: Some(serde_json::json!(1)),
                error: None,
                child_task_id: None,
            }]),
            ..Default::default()
        };
        let back: WorkflowStepState =
            serde_json::from_value(serde_json::to_value(&row).unwrap()).unwrap();
        assert_eq!(back.items.unwrap()[0].status, "completed");
        // Old rows (no items, no script calls) still read.
        let old: WorkflowStepState =
            serde_json::from_value(serde_json::json!({"step_id": "s", "status": "completed"})).unwrap();
        assert!(old.items.is_none() && old.script_calls.is_empty());
    }
}
