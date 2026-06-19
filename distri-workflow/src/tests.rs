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
}
