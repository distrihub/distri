#[cfg(test)]
mod tests {
    use crate::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // ========================================================================
    // Mock Executors
    // ========================================================================

    /// Mock executor that records which steps were executed.
    struct MockExecutor {
        call_count: Arc<AtomicUsize>,
        fail_steps: Vec<String>,
    }

    impl MockExecutor {
        fn new() -> Self {
            Self {
                call_count: Arc::new(AtomicUsize::new(0)),
                fail_steps: vec![],
            }
        }

        fn with_failures(fail_steps: Vec<&str>) -> Self {
            Self {
                call_count: Arc::new(AtomicUsize::new(0)),
                fail_steps: fail_steps.into_iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    #[async_trait::async_trait]
    impl StepExecutor for MockExecutor {
        async fn execute(
            &self,
            step: &WorkflowStep,
            _context: &serde_json::Value,
        ) -> Result<StepResult, String> {
            self.call_count.fetch_add(1, Ordering::SeqCst);

            if self.fail_steps.contains(&step.id) {
                return Ok(StepResult::failed(&format!("Step {} failed", step.id)));
            }

            Ok(StepResult::done_with_context(
                serde_json::json!({ "step_id": step.id, "executed": true }),
                serde_json::json!({ format!("{}_done", step.id): true }),
            ))
        }
    }

    /// Mock executor that only supports specific skills.
    struct SkillAwareExecutor {
        supported_skills: Vec<String>,
    }

    impl SkillAwareExecutor {
        fn with_skills(skills: Vec<&str>) -> Self {
            Self {
                supported_skills: skills.into_iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    #[async_trait::async_trait]
    impl StepExecutor for SkillAwareExecutor {
        async fn execute(
            &self,
            step: &WorkflowStep,
            _context: &serde_json::Value,
        ) -> Result<StepResult, String> {
            Ok(StepResult::done(serde_json::json!({ "step_id": step.id })))
        }

        fn supports(&self, requirement: &StepRequirement) -> bool {
            self.supported_skills.contains(&requirement.skill)
        }

        fn available_skills(&self) -> Vec<StepRequirement> {
            self.supported_skills
                .iter()
                .map(|s| StepRequirement {
                    skill: s.clone(),
                    permissions: vec![],
                    config: None,
                })
                .collect()
        }
    }

    // ========================================================================
    // Original Tests (preserved)
    // ========================================================================

    #[tokio::test]
    async fn sequential_workflow_runs_all_steps() {
        let steps = vec![
            WorkflowStep::api_call("step1", "First", "GET", "/api/1"),
            WorkflowStep::api_call("step2", "Second", "POST", "/api/2"),
            WorkflowStep::api_call("step3", "Third", "PUT", "/api/3"),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);

        let status = runner.run_all(&workflow.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Completed);

        let final_state = runner.get_state(&workflow.id()).await.unwrap().unwrap();
        assert_eq!(final_state.status, WorkflowStatus::Completed);
        assert!(final_state
            .step_runs
            .iter()
            .all(|s| s.status == StepStatus::Done));
    }

    #[tokio::test]
    async fn parallel_steps_all_execute() {
        let steps = vec![
            WorkflowStep::api_call("a", "Step A", "GET", "/a").parallel(),
            WorkflowStep::api_call("b", "Step B", "GET", "/b").parallel(),
            WorkflowStep::api_call("c", "Step C", "GET", "/c").parallel(),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let call_count = executor.call_count.clone();
        let runner = WorkflowRunner::new(store, executor);

        let results = runner.run_next(&workflow.id()).await.unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn dependencies_block_execution() {
        let steps = vec![
            WorkflowStep::api_call("fetch", "Fetch data", "GET", "/data"),
            WorkflowStep::api_call("process", "Process data", "POST", "/process")
                .with_depends_on(vec!["fetch"]),
            WorkflowStep::api_call("save", "Save results", "PUT", "/save")
                .with_depends_on(vec!["process"]),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);

        let r1 = runner.run_next(&workflow.id()).await.unwrap();
        assert_eq!(r1.len(), 1);
        assert_eq!(r1[0].0, "fetch");

        let r2 = runner.run_next(&workflow.id()).await.unwrap();
        assert_eq!(r2.len(), 1);
        assert_eq!(r2[0].0, "process");

        let r3 = runner.run_next(&workflow.id()).await.unwrap();
        assert_eq!(r3.len(), 1);
        assert_eq!(r3[0].0, "save");
    }

    #[tokio::test]
    async fn parallel_with_join_dependency() {
        let steps = vec![
            WorkflowStep::api_call("a", "Step A", "GET", "/a").parallel(),
            WorkflowStep::api_call("b", "Step B", "GET", "/b").parallel(),
            WorkflowStep::api_call("c", "Join step", "POST", "/c").with_depends_on(vec!["a", "b"]),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);

        let r1 = runner.run_next(&workflow.id()).await.unwrap();
        assert_eq!(r1.len(), 2);

        let r2 = runner.run_next(&workflow.id()).await.unwrap();
        assert_eq!(r2.len(), 1);
        assert_eq!(r2[0].0, "c");
    }

    #[tokio::test]
    async fn failure_stops_workflow() {
        let steps = vec![
            WorkflowStep::api_call("ok", "OK step", "GET", "/ok"),
            WorkflowStep::api_call("fail", "Failing step", "POST", "/fail"),
            WorkflowStep::api_call("after", "After fail", "GET", "/after"),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::with_failures(vec!["fail"]);
        let runner = WorkflowRunner::new(store, executor);

        let status = runner.run_all(&workflow.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Failed);

        let state = runner.get_state(&workflow.id()).await.unwrap().unwrap();
        assert_eq!(state.step_runs[0].status, StepStatus::Done);
        assert_eq!(state.step_runs[1].status, StepStatus::Failed);
        assert_eq!(state.step_runs[2].status, StepStatus::Pending);
    }

    #[tokio::test]
    async fn context_propagates_between_steps() {
        let steps = vec![
            WorkflowStep::api_call("step1", "First", "GET", "/1"),
            WorkflowStep::api_call("step2", "Second", "GET", "/2"),
        ];
        let workflow =
            WorkflowRun::from_steps(steps).with_context(serde_json::json!({ "initial": true }));
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);

        runner.run_all(&workflow.id()).await.unwrap();

        let state = runner.get_state(&workflow.id()).await.unwrap().unwrap();
        assert_eq!(state.context["initial"], true);
        assert_eq!(state.context["step1_done"], true);
        assert_eq!(state.context["step2_done"], true);
    }

    #[tokio::test]
    async fn run_next_on_completed_returns_empty() {
        let steps = vec![WorkflowStep::api_call("only", "Only step", "GET", "/")];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);

        runner.run_next(&workflow.id()).await.unwrap();
        let results = runner.run_next(&workflow.id()).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn workflow_serializes_to_json() {
        let steps = vec![
            WorkflowStep::api_call("read", "Read doc", "GET", "/doc/{id}")
                .with_body(serde_json::json!({"format": "text"})),
            WorkflowStep::agent_run("detect", "Detect", "importer", "Analyze this")
                .with_depends_on(vec!["read"])
                .parallel(),
        ];
        let workflow =
            WorkflowRun::from_steps(steps).with_context(serde_json::json!({"doc_id": "abc123"}));

        let json = serde_json::to_string_pretty(&workflow).unwrap();
        let parsed: WorkflowRun = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.steps().len(), 2);
        assert_eq!(parsed.steps()[1].depends_on, vec!["read"]);
        assert_eq!(parsed.steps()[1].execution, StepExecution::Parallel);
    }

    #[tokio::test]
    async fn notes_are_recorded() {
        let mut workflow = WorkflowRun::from_steps(vec![]);
        workflow.add_note("step1", "Detected 10 essays");
        workflow.add_note("step2", "Created 10 submissions");

        assert_eq!(workflow.notes.len(), 2);
        assert_eq!(workflow.notes[0].message, "Detected 10 essays");
    }

    #[tokio::test]
    async fn empty_workflow_is_immediately_complete() {
        let workflow = WorkflowRun::from_steps(vec![]);
        assert!(workflow.is_complete());
    }

    // ========================================================================
    // New Tests: Step Requirements
    // ========================================================================

    #[tokio::test]
    async fn step_requirement_native_builder() {
        let req = StepRequirement::native("shell");
        assert_eq!(req.skill, "native:shell");
        assert!(req.is_native());
        assert_eq!(req.namespace(), Some("native"));
        assert_eq!(req.skill_name(), Some("shell"));
        assert!(req.validate().is_ok());
    }

    #[tokio::test]
    async fn step_requirement_connection_builder() {
        let req = StepRequirement::connection("google", "drive")
            .with_permissions(vec!["drive.readonly", "drive.file"]);
        assert_eq!(req.skill, "google:drive");
        assert!(!req.is_native());
        assert_eq!(req.namespace(), Some("google"));
        assert_eq!(req.permissions, vec!["drive.readonly", "drive.file"]);
        assert!(req.validate().is_ok());
    }

    #[tokio::test]
    async fn step_requirement_validation_rejects_unnamespaced() {
        let req = StepRequirement {
            skill: "shell".to_string(),
            permissions: vec![],
            config: None,
        };
        assert!(req.validate().is_err());
    }

    #[tokio::test]
    async fn step_requirement_validation_rejects_unknown_native() {
        let req = StepRequirement::native("teleporter");
        assert!(req.validate().is_err());
    }

    #[tokio::test]
    async fn requirements_block_step_execution() {
        let steps = vec![WorkflowStep::api_call("step1", "Needs shell", "GET", "/1")
            .with_requires(vec![StepRequirement::native("shell")])];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        // Executor only supports network, not shell
        let executor = SkillAwareExecutor::with_skills(vec!["native:network"]);
        let runner = WorkflowRunner::new(store, executor);

        let results = runner.run_next(&workflow.id()).await.unwrap();
        assert!(results.is_empty());

        let state = runner.get_state(&workflow.id()).await.unwrap().unwrap();
        assert_eq!(state.step_runs[0].status, StepStatus::Blocked);
        assert!(state.step_runs[0]
            .error
            .as_ref()
            .unwrap()
            .contains("native:shell"));
    }

    #[tokio::test]
    async fn requirements_met_allows_execution() {
        let steps = vec![
            WorkflowStep::api_call("step1", "Needs network", "GET", "/1")
                .with_requires(vec![StepRequirement::native("network")]),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = SkillAwareExecutor::with_skills(vec!["native:network"]);
        let runner = WorkflowRunner::new(store, executor);

        let results = runner.run_next(&workflow.id()).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "step1");
    }

    #[tokio::test]
    async fn mixed_blocked_and_executable_steps() {
        let steps = vec![
            WorkflowStep::api_call("net_step", "Network only", "GET", "/api")
                .with_requires(vec![StepRequirement::native("network")])
                .parallel(),
            WorkflowStep::api_call("shell_step", "Needs shell", "GET", "/cmd")
                .with_requires(vec![StepRequirement::native("shell")])
                .parallel(),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = SkillAwareExecutor::with_skills(vec!["native:network"]);
        let runner = WorkflowRunner::new(store, executor);

        let results = runner.run_next(&workflow.id()).await.unwrap();
        // Only net_step should execute
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "net_step");

        let state = runner.get_state(&workflow.id()).await.unwrap().unwrap();
        assert_eq!(state.step_runs[1].status, StepStatus::Blocked);
    }

    #[tokio::test]
    async fn blocked_workflow_status() {
        let steps = vec![
            WorkflowStep::api_call("step1", "Needs browser", "GET", "/1")
                .with_requires(vec![StepRequirement::native("browser")]),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = SkillAwareExecutor::with_skills(vec!["native:network"]);
        let runner = WorkflowRunner::new(store, executor);

        let status = runner.run_all(&workflow.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Blocked);
    }

    #[tokio::test]
    async fn is_stuck_with_blocked_deps() {
        let steps = vec![
            WorkflowStep::api_call("blocked", "Blocked step", "GET", "/1")
                .with_requires(vec![StepRequirement::native("browser")]),
            WorkflowStep::api_call("waits", "Waits on blocked", "POST", "/2")
                .with_depends_on(vec!["blocked"]),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = SkillAwareExecutor::with_skills(vec!["native:network"]);
        let runner = WorkflowRunner::new(store, executor);

        let status = runner.run_all(&workflow.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Blocked);

        let state = runner.get_state(&workflow.id()).await.unwrap().unwrap();
        assert_eq!(state.step_runs[0].status, StepStatus::Blocked);
        // Step waiting on blocked is still pending but workflow is stuck
        assert_eq!(state.step_runs[1].status, StepStatus::Pending);
        assert!(state.is_stuck());
    }

    #[tokio::test]
    async fn no_requirements_uses_default_executor() {
        // Steps without requires should work with any executor (backward compat)
        let steps = vec![WorkflowStep::api_call("step1", "No reqs", "GET", "/1")];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);

        let status = runner.run_all(&workflow.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Completed);
    }

    // ========================================================================
    // New Tests: ToolCall StepKind
    // ========================================================================

    #[tokio::test]
    async fn tool_call_step_executes() {
        let steps = vec![WorkflowStep::tool_call(
            "call_api",
            "Call API request tool",
            "api_request",
            serde_json::json!({"method": "GET", "path": "/v1/skills"}),
        )];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);

        let status = runner.run_all(&workflow.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Completed);
    }

    #[tokio::test]
    async fn tool_call_serializes_correctly() {
        let step = WorkflowStep::tool_call(
            "tc",
            "Tool call",
            "read_doc",
            serde_json::json!({"doc_id": "123"}),
        );
        let json = serde_json::to_value(&step).unwrap();
        assert_eq!(json["kind"]["type"], "tool_call");
        assert_eq!(json["kind"]["tool_name"], "read_doc");
        assert_eq!(json["kind"]["input"]["doc_id"], "123");

        // Round-trip
        let parsed: WorkflowStep = serde_json::from_value(json).unwrap();
        if let StepKind::ToolCall {
            tool_name, input, ..
        } = &parsed.kind
        {
            assert_eq!(tool_name, "read_doc");
            assert_eq!(input["doc_id"], "123");
        } else {
            panic!("Expected ToolCall kind");
        }
    }

    // ========================================================================
    // New Tests: Richer Script
    // ========================================================================

    #[tokio::test]
    async fn script_builder_with_options() {
        let step = WorkflowStep::script("test", "Run tests", "cargo test")
            .with_cwd("/project")
            .with_timeout(300)
            .with_env(
                [("RUST_LOG".to_string(), "debug".to_string())]
                    .into_iter()
                    .collect(),
            );

        if let StepKind::Script {
            command,
            cwd,
            timeout_secs,
            env,
            ..
        } = &step.kind
        {
            assert_eq!(command, "cargo test");
            assert_eq!(cwd.as_deref(), Some("/project"));
            assert_eq!(*timeout_secs, Some(300));
            assert_eq!(env.as_ref().unwrap()["RUST_LOG"], "debug");
        } else {
            panic!("Expected Script kind");
        }
    }

    #[tokio::test]
    async fn script_serializes_with_all_fields() {
        let step = WorkflowStep::script("s", "Script", "echo hello")
            .with_cwd("/tmp")
            .with_timeout(60);

        let json = serde_json::to_value(&step).unwrap();
        assert_eq!(json["kind"]["type"], "script");
        assert_eq!(json["kind"]["command"], "echo hello");
        assert_eq!(json["kind"]["cwd"], "/tmp");
        assert_eq!(json["kind"]["timeout_secs"], 60);

        // Round-trip
        let parsed: WorkflowStep = serde_json::from_value(json).unwrap();
        if let StepKind::Script {
            cwd, timeout_secs, ..
        } = &parsed.kind
        {
            assert_eq!(cwd.as_deref(), Some("/tmp"));
            assert_eq!(*timeout_secs, Some(60));
        } else {
            panic!("Expected Script kind");
        }
    }

    #[tokio::test]
    async fn script_output_format_serializes() {
        let json = serde_json::json!({
            "id": "s",
            "label": "Script",
            "kind": {
                "type": "script",
                "command": "cat data.json",
                "args": [],
                "output_format": "json",
                "shell": "bash"
            },
            "status": "pending",
            "depends_on": [],
            "execution": "sequential",
            "requires": []
        });

        let step: WorkflowStep = serde_json::from_value(json).unwrap();
        if let StepKind::Script {
            output_format,
            shell,
            ..
        } = &step.kind
        {
            assert_eq!(*output_format, Some(ScriptOutputFormat::Json));
            assert_eq!(*shell, Some(ShellType::Bash));
        } else {
            panic!("Expected Script kind");
        }
    }

    // ========================================================================
    // New Tests: Checkpoint Strategy
    // ========================================================================

    #[tokio::test]
    async fn checkpoint_strategy_defaults_to_internal() {
        let workflow = WorkflowRun::from_steps(vec![]);
        match workflow.definition.checkpoint {
            CheckpointStrategy::Internal { ttl_secs } => {
                assert_eq!(ttl_secs, None);
            }
            _ => panic!("Expected Internal checkpoint strategy"),
        }
    }

    #[tokio::test]
    async fn checkpoint_strategy_serializes() {
        let workflow =
            WorkflowRun::from_steps(vec![]).with_checkpoint(CheckpointStrategy::External {
                tool_name: "my_checkpoint_tool".to_string(),
            });

        let json = serde_json::to_value(&workflow).unwrap();
        assert_eq!(json["checkpoint"]["type"], "external");
        assert_eq!(json["checkpoint"]["tool_name"], "my_checkpoint_tool");

        let parsed: WorkflowDefinition = serde_json::from_value(json).unwrap();
        match parsed.checkpoint {
            CheckpointStrategy::External { tool_name } => {
                assert_eq!(tool_name, "my_checkpoint_tool");
            }
            _ => panic!("Expected External checkpoint"),
        }
    }

    #[tokio::test]
    async fn internal_checkpoint_serializes_with_ttl() {
        let workflow =
            WorkflowRun::from_steps(vec![]).with_checkpoint(CheckpointStrategy::Internal {
                ttl_secs: Some(3600),
            });

        let json = serde_json::to_value(&workflow).unwrap();
        assert_eq!(json["checkpoint"]["type"], "internal");
        assert_eq!(json["checkpoint"]["ttl_secs"], 3600);
    }

    // ========================================================================
    // New Tests: AgentRun with skills
    // ========================================================================

    #[tokio::test]
    async fn agent_run_with_skills_serializes() {
        let json = serde_json::json!({
            "id": "analyze",
            "label": "Analyze doc",
            "kind": {
                "type": "agent_run",
                "agent_id": "inline",
                "prompt": "Analyze this document",
                "tools": ["read_file"],
                "skills": ["document_analysis", "grading_rubric"],
                "model": "claude-sonnet-4",
                "max_iterations": 10
            },
            "status": "pending",
            "depends_on": [],
            "execution": "sequential",
            "requires": []
        });

        let step: WorkflowStep = serde_json::from_value(json).unwrap();
        if let StepKind::AgentRun {
            agent_id,
            skills,
            model,
            max_iterations,
            ..
        } = &step.kind
        {
            assert_eq!(agent_id, "inline");
            assert_eq!(skills, &vec!["document_analysis", "grading_rubric"]);
            assert_eq!(model.as_deref(), Some("claude-sonnet-4"));
            assert_eq!(*max_iterations, Some(10));
        } else {
            panic!("Expected AgentRun kind");
        }
    }

    // ========================================================================
    // New Tests: is_stuck logic
    // ========================================================================

    #[tokio::test]
    async fn not_stuck_when_steps_are_pending() {
        let workflow =
            WorkflowRun::from_steps(vec![WorkflowStep::api_call("s", "Step", "GET", "/")]);
        assert!(!workflow.is_stuck());
    }

    #[tokio::test]
    async fn not_stuck_when_all_done() {
        let mut workflow =
            WorkflowRun::from_steps(vec![WorkflowStep::api_call("s", "Step", "GET", "/")]);
        workflow.step_runs[0].status = StepStatus::Done;
        assert!(!workflow.is_stuck());
    }

    #[tokio::test]
    async fn stuck_when_only_blocked_steps_remain() {
        let mut workflow = WorkflowRun::from_steps(vec![
            WorkflowStep::api_call("s1", "Step 1", "GET", "/1"),
            WorkflowStep::api_call("s2", "Step 2", "GET", "/2"),
        ]);
        workflow.step_runs[0].status = StepStatus::Done;
        workflow.step_runs[1].status = StepStatus::Blocked;
        assert!(workflow.is_stuck());
    }

    // ========================================================================
    // New Tests: Full workflow with requirements and new step kinds
    // ========================================================================

    #[tokio::test]
    async fn full_workflow_with_mixed_step_kinds() {
        let steps = vec![
            WorkflowStep::api_call("fetch", "Fetch data", "GET", "/api/data"),
            WorkflowStep::tool_call(
                "process",
                "Process with tool",
                "transform",
                serde_json::json!({"format": "csv"}),
            )
            .with_depends_on(vec!["fetch"]),
            WorkflowStep::script("validate", "Validate output", "python validate.py")
                .with_cwd("/scripts")
                .with_depends_on(vec!["process"]),
        ];

        let workflow =
            WorkflowRun::from_steps(steps).with_context(serde_json::json!({"source": "api"}));

        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);

        let status = runner.run_all(&workflow.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Completed);

        let state = runner.get_state(&workflow.id()).await.unwrap().unwrap();
        assert!(state.step_runs.iter().all(|s| s.status == StepStatus::Done));
    }

    #[tokio::test]
    async fn requirement_serialization_roundtrip() {
        let req =
            StepRequirement::connection("google", "drive").with_permissions(vec!["drive.readonly"]);

        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["skill"], "google:drive");
        assert_eq!(json["permissions"], serde_json::json!(["drive.readonly"]));

        let parsed: StepRequirement = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.skill, "google:drive");
        assert_eq!(parsed.permissions, vec!["drive.readonly"]);
    }

    #[tokio::test]
    async fn existing_test_workflow_json_compatible() {
        // Verify the existing test_workflow.json format still deserializes
        // (backward compatibility — no requires field in old JSON)
        let json = serde_json::json!({
            "id": "test-001",
            "status": "pending",
            "current_step": 0,
            "context": {"file_id": "abc"},
            "steps": [{
                "id": "read_doc",
                "label": "Read Google Doc",
                "kind": {
                    "type": "api_call",
                    "method": "GET",
                    "url": "/files/{id}"
                },
                "status": "pending",
                "depends_on": [],
                "execution": "sequential"
            }],
            "notes": [],
            "created_at": "2026-03-25T00:00:00Z",
            "updated_at": "2026-03-25T00:00:00Z"
        });

        let workflow: WorkflowRun = serde_json::from_value(json).unwrap();
        assert_eq!(workflow.id(), "test-001");
        assert!(workflow.steps()[0].requires.is_empty());
        // Checkpoint defaults to Internal
        match workflow.definition.checkpoint {
            CheckpointStrategy::Internal { ttl_secs } => assert_eq!(ttl_secs, None),
            _ => panic!("Expected Internal default"),
        }
    }

    #[tokio::test]
    async fn checkpoint_builder() {
        let step = WorkflowStep::checkpoint("verify", "Verify import", "Check results");
        assert_eq!(step.id, "verify");
        if let StepKind::Checkpoint { message } = &step.kind {
            assert_eq!(message, "Check results");
        } else {
            panic!("Expected Checkpoint kind");
        }
    }

    // ========================================================================
    // New Tests: input_schema + with_input
    // ========================================================================

    #[tokio::test]
    async fn workflow_with_input_merges_context() {
        let workflow = WorkflowRun::from_steps(vec![])
            .with_context(serde_json::json!({"existing": true}))
            .with_input(serde_json::json!({"file_id": "abc", "class_id": "xyz"}))
            .unwrap();

        assert_eq!(workflow.context["existing"], true);
        assert_eq!(workflow.context["file_id"], "abc");
        assert_eq!(workflow.context["class_id"], "xyz");
    }

    #[tokio::test]
    async fn workflow_serializes_with_input_schema() {
        let mut workflow = WorkflowRun::from_steps(vec![]);
        workflow.definition.input_schema = Some(serde_json::json!({
            "type": "object",
            "required": ["file_id"],
            "properties": { "file_id": { "type": "string" } }
        }));

        let json = serde_json::to_string(&workflow).unwrap();
        let parsed: WorkflowDefinition = serde_json::from_str(&json).unwrap();
        assert!(parsed.input_schema.is_some());
    }

    #[tokio::test]
    async fn executor_available_skills() {
        let executor = SkillAwareExecutor::with_skills(vec!["native:shell", "native:network"]);
        let skills = executor.available_skills();
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].skill, "native:shell");
    }

    // ========================================================================
    // Input validation tests
    // ========================================================================

    #[tokio::test]
    async fn input_validation_rejects_missing_required_field() {
        let mut workflow = WorkflowRun::from_steps(vec![]);
        workflow.definition.input_schema = Some(serde_json::json!({
            "type": "object",
            "required": ["file_id", "class_id"],
            "properties": {
                "file_id": { "type": "string" },
                "class_id": { "type": "string" }
            }
        }));

        // Missing class_id
        let result = workflow.with_input(serde_json::json!({ "file_id": "abc" }));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("validation failed"));
    }

    #[tokio::test]
    async fn input_validation_rejects_wrong_type() {
        let mut workflow = WorkflowRun::from_steps(vec![]);
        workflow.definition.input_schema = Some(serde_json::json!({
            "type": "object",
            "properties": { "count": { "type": "integer" } }
        }));

        let result = workflow.with_input(serde_json::json!({ "count": "not a number" }));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn input_validation_accepts_valid_input() {
        let mut workflow = WorkflowRun::from_steps(vec![]);
        workflow.definition.input_schema = Some(serde_json::json!({
            "type": "object",
            "required": ["file_id"],
            "properties": { "file_id": { "type": "string" } }
        }));

        let result = workflow.with_input(serde_json::json!({ "file_id": "abc123" }));
        assert!(result.is_ok());
        let w = result.unwrap();
        assert_eq!(w.context["file_id"], "abc123");
        assert_eq!(w.status, WorkflowStatus::Running);
    }

    #[tokio::test]
    async fn input_without_schema_accepts_anything() {
        let workflow = WorkflowRun::from_steps(vec![]);
        // No input_schema set
        let result = workflow.with_input(serde_json::json!({ "anything": "goes" }));
        assert!(result.is_ok());
    }

    // ========================================================================
    // Event emission tests
    // ========================================================================

    #[tokio::test]
    async fn run_all_emits_events() {
        use std::sync::Mutex;

        struct CollectingSink {
            events: Mutex<Vec<WorkflowEvent>>,
        }

        #[async_trait::async_trait]
        impl EventSink for CollectingSink {
            async fn emit(&self, event: WorkflowEvent) {
                self.events.lock().unwrap().push(event);
            }
        }

        let steps = vec![
            WorkflowStep::api_call("s1", "Step 1", "GET", "/1"),
            WorkflowStep::api_call("s2", "Step 2", "POST", "/2"),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let sink = CollectingSink {
            events: Mutex::new(vec![]),
        };
        let executor = MockExecutor::new();
        let runner = WorkflowRunner::with_events(store, executor, sink);

        runner.run_all(&workflow.id()).await.unwrap();

        let events = runner.events.events.lock().unwrap();
        // Should have: started, step1_started, step1_completed, step2_started, step2_completed, workflow_completed
        assert!(
            events.len() >= 5,
            "Expected at least 5 events, got {}",
            events.len()
        );

        // First event should be WorkflowStarted
        matches!(&events[0], WorkflowEvent::WorkflowStarted { .. });
        // Last should be WorkflowCompleted
        matches!(
            events.last().unwrap(),
            WorkflowEvent::WorkflowCompleted { .. }
        );
    }

    // ========================================================================
    // Data Flow: Namespace Resolution & Interdependent Steps
    // ========================================================================

    /// Executor that captures the context each step receives and returns configurable results.
    struct ContextCapturingExecutor {
        captured: Arc<tokio::sync::Mutex<Vec<(String, serde_json::Value)>>>,
        results: std::collections::HashMap<String, serde_json::Value>,
    }

    impl ContextCapturingExecutor {
        fn new(results: Vec<(&str, serde_json::Value)>) -> Self {
            Self {
                captured: Arc::new(tokio::sync::Mutex::new(vec![])),
                results: results
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
            }
        }

        fn captured(&self) -> Arc<tokio::sync::Mutex<Vec<(String, serde_json::Value)>>> {
            self.captured.clone()
        }
    }

    #[async_trait::async_trait]
    impl StepExecutor for ContextCapturingExecutor {
        async fn execute(
            &self,
            step: &WorkflowStep,
            context: &serde_json::Value,
        ) -> Result<StepResult, String> {
            self.captured
                .lock()
                .await
                .push((step.id.clone(), context.clone()));

            let result = self
                .results
                .get(&step.id)
                .cloned()
                .unwrap_or(serde_json::json!({"step_id": step.id}));

            Ok(StepResult::done(result))
        }
    }

    #[tokio::test]
    async fn interdependent_steps_receive_resolved_input() {
        // Step 1: fetch_doc — receives input.doc_id, produces {content, title}
        // Step 2: process — depends on fetch_doc, receives {steps.fetch_doc.content, input.class_id}
        // Step 3: save — depends on process, receives {steps.process.summary, steps.fetch_doc.title}

        let steps = vec![
            WorkflowStep::tool_call("fetch_doc", "Fetch", "read_doc", serde_json::json!({}))
                .with_input_mapping(serde_json::json!({
                    "doc_id": "{input.doc_id}"
                })),
            WorkflowStep::tool_call("process", "Process", "analyze", serde_json::json!({}))
                .with_depends_on(vec!["fetch_doc"])
                .with_input_mapping(serde_json::json!({
                    "text": "{steps.fetch_doc.content}",
                    "class": "{input.class_id}"
                })),
            WorkflowStep::tool_call("save", "Save", "persist", serde_json::json!({}))
                .with_depends_on(vec!["process"])
                .with_input_mapping(serde_json::json!({
                    "summary": "{steps.process.summary}",
                    "title": "{steps.fetch_doc.title}"
                })),
        ];

        let mut workflow = WorkflowRun::from_steps(steps);
        // Initialize structured context with input
        workflow.context = serde_json::json!({
            "input": {"doc_id": "doc-123", "class_id": "class-abc"},
            "steps": {},
            "env": {}
        });

        let executor = ContextCapturingExecutor::new(vec![
            (
                "fetch_doc",
                serde_json::json!({"content": "Hello world", "title": "My Essay"}),
            ),
            (
                "process",
                serde_json::json!({"summary": "Essay about greeting"}),
            ),
            ("save", serde_json::json!({"saved": true})),
        ]);
        let captured = executor.captured();

        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();
        let runner = WorkflowRunner::new(store, executor);
        let status = runner.run_all(&workflow.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Completed);

        let caps = captured.lock().await;
        assert_eq!(caps.len(), 3);

        // Step 1 receives resolved {input.doc_id}
        assert_eq!(caps[0].0, "fetch_doc");
        assert_eq!(caps[0].1["doc_id"], "doc-123");

        // Step 2 receives resolved {steps.fetch_doc.content} and {input.class_id}
        assert_eq!(caps[1].0, "process");
        assert_eq!(caps[1].1["text"], "Hello world");
        assert_eq!(caps[1].1["class"], "class-abc");

        // Step 3 receives resolved {steps.process.summary} and {steps.fetch_doc.title}
        assert_eq!(caps[2].0, "save");
        assert_eq!(caps[2].1["summary"], "Essay about greeting");
        assert_eq!(caps[2].1["title"], "My Essay");
    }

    #[tokio::test]
    async fn step_without_input_mapping_receives_full_context() {
        let steps = vec![WorkflowStep::tool_call(
            "s1",
            "Step 1",
            "tool_a",
            serde_json::json!({}),
        )];

        let mut workflow = WorkflowRun::from_steps(steps);
        workflow.context = serde_json::json!({
            "input": {"key": "value"},
            "steps": {},
            "env": {"base": "http://localhost"}
        });

        let executor =
            ContextCapturingExecutor::new(vec![("s1", serde_json::json!({"done": true}))]);
        let captured = executor.captured();

        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();
        let runner = WorkflowRunner::new(store, executor);
        runner.run_all(&workflow.id()).await.unwrap();

        let caps = captured.lock().await;
        // No input mapping → receives full context with all namespaces
        assert!(caps[0].1.get("input").is_some());
        assert!(caps[0].1.get("steps").is_some());
        assert!(caps[0].1.get("env").is_some());
    }

    #[tokio::test]
    async fn full_value_reference_preserves_array_type() {
        let steps = vec![
            WorkflowStep::tool_call("fetch", "Fetch", "tool_a", serde_json::json!({})),
            WorkflowStep::tool_call("use_array", "Use array", "tool_b", serde_json::json!({}))
                .with_depends_on(vec!["fetch"])
                .with_input_mapping(serde_json::json!({
                    "items": "{steps.fetch.items}",
                    "count": "{steps.fetch.count}"
                })),
        ];

        let mut workflow = WorkflowRun::from_steps(steps);
        workflow.context = serde_json::json!({"input": {}, "steps": {}, "env": {}});

        let executor = ContextCapturingExecutor::new(vec![
            ("fetch", serde_json::json!({"items": [1, 2, 3], "count": 3})),
            ("use_array", serde_json::json!({"processed": true})),
        ]);
        let captured = executor.captured();

        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();
        let runner = WorkflowRunner::new(store, executor);
        runner.run_all(&workflow.id()).await.unwrap();

        let caps = captured.lock().await;
        assert_eq!(caps[1].0, "use_array");
        // Array should be preserved, not stringified
        assert!(caps[1].1["items"].is_array());
        assert_eq!(caps[1].1["items"].as_array().unwrap().len(), 3);
        // Number should be preserved
        assert_eq!(caps[1].1["count"], 3);
    }

    // ========================================================================
    // Cycle Detection
    // ========================================================================

    #[tokio::test]
    async fn detect_cycle_simple() {
        let steps = vec![
            WorkflowStep::tool_call("a", "A", "t", serde_json::json!({}))
                .with_depends_on(vec!["c"]),
            WorkflowStep::tool_call("b", "B", "t", serde_json::json!({}))
                .with_depends_on(vec!["a"]),
            WorkflowStep::tool_call("c", "C", "t", serde_json::json!({}))
                .with_depends_on(vec!["b"]),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let result = workflow.detect_cycles();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Circular dependency"));
    }

    #[tokio::test]
    async fn detect_cycle_self_reference() {
        let steps = vec![
            WorkflowStep::tool_call("a", "A", "t", serde_json::json!({}))
                .with_depends_on(vec!["a"]),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let result = workflow.detect_cycles();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn detect_nonexistent_dependency() {
        let steps = vec![
            WorkflowStep::tool_call("a", "A", "t", serde_json::json!({}))
                .with_depends_on(vec!["missing_step"]),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let result = workflow.detect_cycles();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[tokio::test]
    async fn no_cycle_in_valid_dag() {
        let steps = vec![
            WorkflowStep::tool_call("a", "A", "t", serde_json::json!({})),
            WorkflowStep::tool_call("b", "B", "t", serde_json::json!({}))
                .with_depends_on(vec!["a"]),
            WorkflowStep::tool_call("c", "C", "t", serde_json::json!({}))
                .with_depends_on(vec!["a", "b"]),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        assert!(workflow.detect_cycles().is_ok());
    }

    #[tokio::test]
    async fn run_all_rejects_cyclic_workflow() {
        let steps = vec![
            WorkflowStep::tool_call("a", "A", "t", serde_json::json!({}))
                .with_depends_on(vec!["b"]),
            WorkflowStep::tool_call("b", "B", "t", serde_json::json!({}))
                .with_depends_on(vec!["a"]),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);
        let result = runner.run_all(&workflow.id()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Circular dependency"));
    }

    // ========================================================================
    // Parallel fan-out + join with data flow
    // ========================================================================

    #[tokio::test]
    async fn parallel_fan_out_join_with_data_flow() {
        // Three parallel fetches → join step uses all three results
        let steps = vec![
            WorkflowStep::tool_call("fetch_users", "Users", "api", serde_json::json!({}))
                .parallel(),
            WorkflowStep::tool_call("fetch_orders", "Orders", "api", serde_json::json!({}))
                .parallel(),
            WorkflowStep::tool_call("fetch_products", "Products", "api", serde_json::json!({}))
                .parallel(),
            WorkflowStep::tool_call("merge", "Merge", "merge_tool", serde_json::json!({}))
                .with_depends_on(vec!["fetch_users", "fetch_orders", "fetch_products"])
                .with_input_mapping(serde_json::json!({
                    "users": "{steps.fetch_users.data}",
                    "orders": "{steps.fetch_orders.data}",
                    "products": "{steps.fetch_products.data}"
                })),
        ];

        let mut workflow = WorkflowRun::from_steps(steps);
        workflow.context = serde_json::json!({"input": {}, "steps": {}, "env": {}});

        let executor = ContextCapturingExecutor::new(vec![
            (
                "fetch_users",
                serde_json::json!({"data": [{"id": 1, "name": "Alice"}]}),
            ),
            (
                "fetch_orders",
                serde_json::json!({"data": [{"id": 100, "total": 50}]}),
            ),
            (
                "fetch_products",
                serde_json::json!({"data": [{"id": "p1", "name": "Widget"}]}),
            ),
            (
                "merge",
                serde_json::json!({"merged": true, "total_records": 3}),
            ),
        ]);
        let captured = executor.captured();

        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();
        let runner = WorkflowRunner::new(store, executor);
        let status = runner.run_all(&workflow.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Completed);

        let caps = captured.lock().await;
        assert_eq!(caps.len(), 4);

        // Merge step should receive all three data arrays
        let merge_ctx = &caps[3].1;
        assert!(merge_ctx["users"].is_array());
        assert!(merge_ctx["orders"].is_array());
        assert!(merge_ctx["products"].is_array());
        assert_eq!(merge_ctx["users"][0]["name"], "Alice");
        assert_eq!(merge_ctx["orders"][0]["total"], 50);
    }

    // ========================================================================
    // Definition serialization: no runtime state in template
    // ========================================================================

    #[tokio::test]
    async fn definition_without_runtime_fields_deserializes() {
        // A minimal definition — no status, context, current_step, notes, timestamps.
        // The clean shape parses as `WorkflowDefinition`; constructing a run
        // out of it gives us the canonical initial state.
        let json = serde_json::json!({
            "id": "minimal",
            "input_schema": {
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"]
            },
            "steps": [
                {
                    "id": "greet",
                    "label": "Greet user",
                    "kind": { "type": "tool_call", "tool_name": "greeter", "input": {} },
                    "input": { "name": "{input.name}" }
                }
            ]
        });

        let definition: WorkflowDefinition = serde_json::from_value(json).unwrap();
        assert_eq!(definition.id, "minimal");
        assert!(definition.input_schema.is_some());
        assert!(definition.steps[0].input.is_some());

        let run = WorkflowRun::new(definition);
        assert_eq!(run.status, WorkflowStatus::Pending);
        assert_eq!(run.current_step, 0);
        assert!(run.notes.is_empty());
        assert_eq!(run.step_runs[0].status, StepStatus::Pending);
        assert!(run.step_runs[0].result.is_none());
    }

    // ========================================================================
    // Multi-Entry Point Tests
    // ========================================================================

    #[test]
    fn entry_point_skips_unreachable_steps() {
        let steps = vec![
            WorkflowStep::api_call("detect", "Detect docs", "POST", "/detect"),
            WorkflowStep::api_call("create_content", "Create content", "POST", "/content")
                .with_depends_on(vec!["detect"]),
            WorkflowStep::api_call("configure_eval", "Configure eval", "POST", "/eval")
                .with_depends_on(vec!["create_content"]),
            WorkflowStep::api_call("grade", "Grade", "POST", "/grade")
                .with_depends_on(vec!["configure_eval"]),
        ];

        let workflow = WorkflowRun::from_steps(steps).with_entry_points(vec![EntryPoint {
            id: "grade_only".to_string(),
            label: "Grade Only".to_string(),
            description: Some("Start at grading".to_string()),
            starts_at: "grade".to_string(),
            preset_results: HashMap::from([(
                "configure_eval".to_string(),
                serde_json::json!({"rubric_id": "r1"}),
            )]),
            required_inputs: vec!["activity_id".to_string()],
            trigger: None,
        }]);

        let applied = workflow.apply_entry_point("grade_only").unwrap();

        // detect, create_content, configure_eval should be skipped
        assert_eq!(applied.step_runs[0].status, StepStatus::Skipped); // detect
        assert_eq!(applied.step_runs[1].status, StepStatus::Skipped); // create_content
        assert_eq!(applied.step_runs[2].status, StepStatus::Skipped); // configure_eval
        assert_eq!(applied.step_runs[3].status, StepStatus::Pending); // grade — should run

        // configure_eval should have preset result
        assert_eq!(
            applied.step_runs[2].result,
            Some(serde_json::json!({"rubric_id": "r1"}))
        );
    }

    #[test]
    fn entry_point_mid_workflow() {
        let steps = vec![
            WorkflowStep::api_call("detect", "Detect", "POST", "/detect"),
            WorkflowStep::api_call("content", "Content", "POST", "/content")
                .with_depends_on(vec!["detect"]),
            WorkflowStep::api_call("eval", "Eval", "POST", "/eval")
                .with_depends_on(vec!["content"]),
            WorkflowStep::api_call("grade", "Grade", "POST", "/grade")
                .with_depends_on(vec!["eval"]),
        ];

        let workflow = WorkflowRun::from_steps(steps).with_entry_points(vec![EntryPoint {
            id: "existing_activity".to_string(),
            label: "Existing Activity".to_string(),
            description: None,
            starts_at: "eval".to_string(),
            preset_results: HashMap::new(),
            required_inputs: vec![],
            trigger: None,
        }]);

        let applied = workflow.apply_entry_point("existing_activity").unwrap();
        assert_eq!(applied.step_runs[0].status, StepStatus::Skipped); // detect
        assert_eq!(applied.step_runs[1].status, StepStatus::Skipped); // content
        assert_eq!(applied.step_runs[2].status, StepStatus::Pending); // eval
        assert_eq!(applied.step_runs[3].status, StepStatus::Pending); // grade
    }

    #[test]
    fn entry_point_not_found_returns_error() {
        let workflow =
            WorkflowRun::from_steps(vec![WorkflowStep::api_call("s1", "S1", "GET", "/")]);
        assert!(workflow.apply_entry_point("nonexistent").is_err());
    }

    #[test]
    fn entry_point_populates_context() {
        let steps = vec![
            WorkflowStep::api_call("detect", "Detect", "POST", "/detect"),
            WorkflowStep::api_call("grade", "Grade", "POST", "/grade")
                .with_depends_on(vec!["detect"]),
        ];

        let workflow = WorkflowRun::from_steps(steps).with_entry_points(vec![EntryPoint {
            id: "grade_only".to_string(),
            label: "Grade Only".to_string(),
            description: None,
            starts_at: "grade".to_string(),
            preset_results: HashMap::from([(
                "detect".to_string(),
                serde_json::json!({"questions": [1, 2, 3]}),
            )]),
            required_inputs: vec![],
            trigger: None,
        }]);

        let applied = workflow.apply_entry_point("grade_only").unwrap();

        // Context should have preset results under steps namespace
        let ctx_steps = applied.context.get("steps").unwrap();
        assert_eq!(
            ctx_steps["detect"]["questions"],
            serde_json::json!([1, 2, 3])
        );
    }

    #[tokio::test]
    async fn entry_point_runs_from_mid_workflow() {
        let steps = vec![
            WorkflowStep::api_call("detect", "Detect", "POST", "/detect"),
            WorkflowStep::api_call("content", "Content", "POST", "/content")
                .with_depends_on(vec!["detect"]),
            WorkflowStep::api_call("eval", "Eval", "POST", "/eval")
                .with_depends_on(vec!["content"]),
        ];

        let workflow = WorkflowRun::from_steps(steps)
            .with_id("ep-test")
            .with_entry_points(vec![EntryPoint {
                id: "from_eval".to_string(),
                label: "From Eval".to_string(),
                description: None,
                starts_at: "eval".to_string(),
                preset_results: HashMap::new(),
                required_inputs: vec![],
                trigger: None,
            }]);

        let applied = workflow.apply_entry_point("from_eval").unwrap();
        let store = InMemoryStore::new();
        store.save(&applied).await.unwrap();

        let runner = WorkflowRunner::new(store, MockExecutor::new());
        let status = runner.run_all("ep-test").await.unwrap();

        assert_eq!(status, WorkflowStatus::Completed);
    }

    // ========================================================================
    // Skip-If Tests
    // ========================================================================

    #[tokio::test]
    async fn skip_if_skips_step_when_condition_true() {
        let steps = vec![
            WorkflowStep::api_call("detect", "Detect", "POST", "/detect")
                .with_skip_if("{input.activity_id}"),
            WorkflowStep::api_call("grade", "Grade", "POST", "/grade"),
        ];

        let workflow = WorkflowRun::from_steps(steps)
            .with_id("skip-test")
            .with_input(serde_json::json!({"activity_id": "existing-123"}))
            .unwrap();

        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let runner = WorkflowRunner::new(store, MockExecutor::new());
        let status = runner.run_all("skip-test").await.unwrap();

        assert_eq!(status, WorkflowStatus::Completed);
        let final_wf = runner.get_state("skip-test").await.unwrap().unwrap();
        assert_eq!(final_wf.step_runs[0].status, StepStatus::Skipped);
        assert_eq!(final_wf.step_runs[1].status, StepStatus::Done);
    }

    #[tokio::test]
    async fn skip_if_runs_step_when_condition_false() {
        let steps = vec![
            WorkflowStep::api_call("detect", "Detect", "POST", "/detect")
                .with_skip_if("{input.activity_id}"),
            WorkflowStep::api_call("grade", "Grade", "POST", "/grade"),
        ];

        let workflow = WorkflowRun::from_steps(steps)
            .with_id("no-skip-test")
            .with_input(serde_json::json!({}))
            .unwrap();

        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let runner = WorkflowRunner::new(store, MockExecutor::new());
        let status = runner.run_all("no-skip-test").await.unwrap();

        assert_eq!(status, WorkflowStatus::Completed);
        let final_wf = runner.get_state("no-skip-test").await.unwrap().unwrap();
        assert_eq!(final_wf.step_runs[0].status, StepStatus::Done);
        assert_eq!(final_wf.step_runs[1].status, StepStatus::Done);
    }

    #[test]
    fn skip_if_negation_works() {
        let ctx = serde_json::json!({
            "input": { "mode": "generate" },
            "steps": {},
            "env": {}
        });

        // {input.mode} is truthy
        assert!(crate::resolve::evaluate_skip_condition(
            "{input.mode}",
            &ctx
        ));
        // !{input.mode} is falsy
        assert!(!crate::resolve::evaluate_skip_condition(
            "!{input.mode}",
            &ctx
        ));
        // {input.nonexistent} is falsy
        assert!(!crate::resolve::evaluate_skip_condition(
            "{input.nonexistent}",
            &ctx
        ));
        // !{input.nonexistent} is truthy
        assert!(crate::resolve::evaluate_skip_condition(
            "!{input.nonexistent}",
            &ctx
        ));
    }

    #[test]
    fn skip_if_equality_check() {
        let ctx = serde_json::json!({
            "input": { "mode": "pick" },
            "steps": {},
            "env": {}
        });

        assert!(crate::resolve::evaluate_skip_condition(
            "{input.mode} == \"pick\"",
            &ctx
        ));
        assert!(!crate::resolve::evaluate_skip_condition(
            "{input.mode} == \"generate\"",
            &ctx
        ));
        assert!(crate::resolve::evaluate_skip_condition(
            "{input.mode} != \"generate\"",
            &ctx
        ));
    }

    #[test]
    fn entry_point_serializes_roundtrip() {
        let steps = vec![
            WorkflowStep::api_call("s1", "S1", "GET", "/"),
            WorkflowStep::api_call("s2", "S2", "GET", "/").with_depends_on(vec!["s1"]),
        ];

        let workflow = WorkflowRun::from_steps(steps).with_entry_points(vec![EntryPoint {
            id: "from_s2".to_string(),
            label: "From S2".to_string(),
            description: Some("Skip S1".to_string()),
            starts_at: "s2".to_string(),
            preset_results: HashMap::from([("s1".to_string(), serde_json::json!({"done": true}))]),
            required_inputs: vec!["data".to_string()],
            trigger: None,
        }]);

        let json = serde_json::to_value(&workflow).unwrap();
        let deserialized: WorkflowDefinition = serde_json::from_value(json).unwrap();

        assert_eq!(deserialized.entry_points.len(), 1);
        assert_eq!(deserialized.entry_points[0].id, "from_s2");
        assert_eq!(deserialized.entry_points[0].starts_at, "s2");
        assert_eq!(deserialized.entry_points[0].required_inputs, vec!["data"]);
    }

    #[test]
    fn skip_if_serializes_roundtrip() {
        let step =
            WorkflowStep::api_call("s1", "S1", "GET", "/").with_skip_if("{input.existing_id}");

        let json = serde_json::to_value(&step).unwrap();
        assert_eq!(json["skip_if"], "{input.existing_id}");

        let deserialized: WorkflowStep = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.skip_if.as_deref(), Some("{input.existing_id}"));
    }

    // ========================================================================
    // WaitForInput / Human-in-the-loop tests
    // ========================================================================

    #[tokio::test]
    async fn wait_for_input_pauses_workflow() {
        let steps = vec![
            WorkflowStep::api_call("step1", "Step 1", "GET", "/api/data"),
            WorkflowStep::wait_for_input("human_review", "Human Review", "Please review the data")
                .with_depends_on(vec!["step1"]),
            WorkflowStep::api_call("step3", "Step 3", "POST", "/api/submit")
                .with_depends_on(vec!["human_review"]),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();
        let runner = WorkflowRunner::new(store, MockExecutor::new());

        // Run all — should execute step1 then pause at human_review
        let status = runner.run_all(&workflow.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Paused);

        let state = runner.get_state(&workflow.id()).await.unwrap().unwrap();
        assert_eq!(state.step_runs[0].status, StepStatus::Done);
        assert_eq!(state.step_runs[1].status, StepStatus::WaitingForInput);
        assert_eq!(state.step_runs[2].status, StepStatus::Pending);
    }

    #[tokio::test]
    async fn resume_after_wait_completes_workflow() {
        let steps = vec![
            WorkflowStep::api_call("step1", "Step 1", "GET", "/api/data"),
            WorkflowStep::wait_for_input("human_review", "Human Review", "Please review the data")
                .with_depends_on(vec!["step1"]),
            WorkflowStep::api_call("step3", "Step 3", "POST", "/api/submit")
                .with_depends_on(vec!["human_review"]),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();
        let runner = WorkflowRunner::new(store, MockExecutor::new());

        // Run until paused
        let status = runner.run_all(&workflow.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Paused);

        // Resume with human input
        let status = runner
            .resume(
                &workflow.id(),
                "human_review",
                serde_json::json!({"approved": true, "notes": "Looks good"}),
            )
            .await
            .unwrap();
        assert_eq!(status, WorkflowStatus::Completed);

        let state = runner.get_state(&workflow.id()).await.unwrap().unwrap();
        assert_eq!(state.step_runs[0].status, StepStatus::Done);
        assert_eq!(state.step_runs[1].status, StepStatus::Done);
        assert_eq!(state.step_runs[2].status, StepStatus::Done);

        // Verify human input was stored in context for downstream steps
        let ctx = state.context.as_object().unwrap();
        let steps_ctx = ctx.get("steps").unwrap().as_object().unwrap();
        let review_result = steps_ctx.get("human_review").unwrap();
        assert_eq!(review_result["approved"], true);
    }

    #[tokio::test]
    async fn resume_wrong_step_id_fails() {
        let steps = vec![WorkflowStep::wait_for_input(
            "review",
            "Review",
            "Review this",
        )];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();
        let runner = WorkflowRunner::new(store, MockExecutor::new());

        let status = runner.run_all(&workflow.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Paused);

        let err = runner
            .resume(&workflow.id(), "wrong_id", serde_json::json!({}))
            .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn resume_non_paused_workflow_fails() {
        let steps = vec![WorkflowStep::api_call(
            "step1",
            "Step 1",
            "GET",
            "/api/data",
        )];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();
        let runner = WorkflowRunner::new(store, MockExecutor::new());

        let err = runner
            .resume(&workflow.id(), "step1", serde_json::json!({}))
            .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn wait_for_input_with_entry_point() {
        // Workflow with entry point that skips to a wait step
        let steps = vec![
            WorkflowStep::api_call("detect", "Detect", "GET", "/detect"),
            WorkflowStep::wait_for_input(
                "confirm_import",
                "Confirm Import",
                "Review detected items before importing",
            )
            .with_depends_on(vec!["detect"]),
            WorkflowStep::api_call("import", "Import", "POST", "/import")
                .with_depends_on(vec!["confirm_import"]),
        ];

        let workflow = WorkflowRun::from_steps(steps)
            .with_entry_points(vec![EntryPoint {
                id: "review_only".to_string(),
                label: "Review & Import".to_string(),
                description: None,
                starts_at: "confirm_import".to_string(),
                preset_results: {
                    let mut m = HashMap::new();
                    m.insert(
                        "detect".to_string(),
                        serde_json::json!({"items": ["q1", "q2"]}),
                    );
                    m
                },
                required_inputs: vec![],
                trigger: None,
            }])
            .apply_entry_point("review_only")
            .unwrap();

        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();
        let runner = WorkflowRunner::new(store, MockExecutor::new());

        // detect should be skipped, should pause at confirm_import
        let status = runner.run_all(&workflow.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Paused);

        let state = runner.get_state(&workflow.id()).await.unwrap().unwrap();
        assert_eq!(state.step_runs[0].status, StepStatus::Skipped); // detect
        assert_eq!(state.step_runs[1].status, StepStatus::WaitingForInput); // confirm_import

        // Resume
        let status = runner
            .resume(
                &workflow.id(),
                "confirm_import",
                serde_json::json!({"confirmed": true}),
            )
            .await
            .unwrap();
        assert_eq!(status, WorkflowStatus::Completed);
    }

    #[tokio::test]
    async fn multiple_wait_for_input_steps() {
        let steps = vec![
            WorkflowStep::api_call("step1", "Step 1", "GET", "/api/data"),
            WorkflowStep::wait_for_input("review1", "First Review", "Review data")
                .with_depends_on(vec!["step1"]),
            WorkflowStep::api_call("step2", "Process", "POST", "/process")
                .with_depends_on(vec!["review1"]),
            WorkflowStep::wait_for_input("review2", "Final Review", "Final approval")
                .with_depends_on(vec!["step2"]),
            WorkflowStep::api_call("step3", "Submit", "POST", "/submit")
                .with_depends_on(vec!["review2"]),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();
        let runner = WorkflowRunner::new(store, MockExecutor::new());

        // Pause at first review
        let status = runner.run_all(&workflow.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Paused);

        // Resume first review — should run step2 then pause at second review
        let status = runner
            .resume(&workflow.id(), "review1", serde_json::json!({"ok": true}))
            .await
            .unwrap();
        assert_eq!(status, WorkflowStatus::Paused);

        let state = runner.get_state(&workflow.id()).await.unwrap().unwrap();
        assert_eq!(state.step_runs[2].status, StepStatus::Done); // step2 ran
        assert_eq!(state.step_runs[3].status, StepStatus::WaitingForInput); // review2

        // Resume second review — should complete
        let status = runner
            .resume(
                &workflow.id(),
                "review2",
                serde_json::json!({"approved": true}),
            )
            .await
            .unwrap();
        assert_eq!(status, WorkflowStatus::Completed);
    }

    #[tokio::test]
    async fn wait_for_input_serialization() {
        let step = WorkflowStep::wait_for_input("review", "Review", "Please review");
        let json = serde_json::to_value(&step).unwrap();
        assert_eq!(json["kind"]["type"], "wait_for_input");
        assert_eq!(json["kind"]["message"], "Please review");

        let deserialized: WorkflowStep = serde_json::from_value(json).unwrap();
        match deserialized.kind {
            StepKind::WaitForInput { message, schema } => {
                assert_eq!(message, "Please review");
                assert!(schema.is_none());
            }
            _ => panic!("Expected WaitForInput"),
        }
    }

    #[tokio::test]
    async fn wait_for_input_with_schema() {
        let mut step = WorkflowStep::wait_for_input("review", "Review", "Approve?");
        if let StepKind::WaitForInput { ref mut schema, .. } = step.kind {
            *schema = Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "approved": { "type": "boolean" }
                },
                "required": ["approved"]
            }));
        }
        let json = serde_json::to_value(&step).unwrap();
        assert!(json["kind"]["schema"].is_object());
    }

    // ========================================================================
    // Multi-Entry + Checkpoint Integration Scenarios
    // ========================================================================

    /// Full grading pipeline with three entry points:
    /// detect → create_content → configure_eval → checkpoint(review) → grade → report
    /// Entry points: "full" (default), "grade_only" (skip to grade), "review_and_grade" (skip to checkpoint)
    fn grading_workflow() -> WorkflowRun {
        let steps = vec![
            WorkflowStep::api_call("detect", "Detect documents", "POST", "/detect"),
            WorkflowStep::api_call("create_content", "Create content", "POST", "/content")
                .with_depends_on(vec!["detect"]),
            WorkflowStep::api_call("configure_eval", "Configure evaluation", "POST", "/eval")
                .with_depends_on(vec!["create_content"]),
            WorkflowStep::wait_for_input(
                "review",
                "Review Configuration",
                "Please review the evaluation configuration before grading",
            )
            .with_depends_on(vec!["configure_eval"]),
            WorkflowStep::api_call("grade", "Grade submissions", "POST", "/grade")
                .with_depends_on(vec!["review"]),
            WorkflowStep::api_call("report", "Generate report", "POST", "/report")
                .with_depends_on(vec!["grade"]),
        ];

        WorkflowRun::from_steps(steps)
            .with_id("grading-pipeline")
            .with_entry_points(vec![
                EntryPoint {
                    id: "grade_only".to_string(),
                    label: "Grade Only".to_string(),
                    description: Some("Skip detection and content creation, go straight to grading".to_string()),
                    starts_at: "grade".to_string(),
                    preset_results: HashMap::from([
                        ("detect".to_string(), serde_json::json!({"documents": ["doc1", "doc2"]})),
                        ("create_content".to_string(), serde_json::json!({"content_id": "c1"})),
                        ("configure_eval".to_string(), serde_json::json!({"rubric_id": "r1", "criteria": ["accuracy", "clarity"]})),
                        ("review".to_string(), serde_json::json!({"approved": true})),
                    ]),
                    required_inputs: vec!["activity_id".to_string()],
                    trigger: None,
                },
                EntryPoint {
                    id: "review_and_grade".to_string(),
                    label: "Review & Grade".to_string(),
                    description: Some("Start at the review checkpoint".to_string()),
                    starts_at: "review".to_string(),
                    preset_results: HashMap::from([
                        ("detect".to_string(), serde_json::json!({"documents": ["doc1"]})),
                        ("create_content".to_string(), serde_json::json!({"content_id": "c1"})),
                        ("configure_eval".to_string(), serde_json::json!({"rubric_id": "r1"})),
                    ]),
                    required_inputs: vec![],
                    trigger: None,
                },
            ])
    }

    #[tokio::test]
    async fn multi_entry_full_run_hits_checkpoint() {
        let workflow = grading_workflow();
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let runner = WorkflowRunner::new(store, MockExecutor::new());
        let status = runner.run_all("grading-pipeline").await.unwrap();

        // Should pause at the review checkpoint
        assert_eq!(status, WorkflowStatus::Paused);

        let state = runner.get_state("grading-pipeline").await.unwrap().unwrap();
        assert_eq!(state.step_runs[0].status, StepStatus::Done); // detect
        assert_eq!(state.step_runs[1].status, StepStatus::Done); // create_content
        assert_eq!(state.step_runs[2].status, StepStatus::Done); // configure_eval
        assert_eq!(state.step_runs[3].status, StepStatus::WaitingForInput); // review
        assert_eq!(state.step_runs[4].status, StepStatus::Pending); // grade
        assert_eq!(state.step_runs[5].status, StepStatus::Pending); // report
    }

    #[tokio::test]
    async fn multi_entry_full_run_resume_completes() {
        let workflow = grading_workflow();
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let runner = WorkflowRunner::new(store, MockExecutor::new());

        // Run to checkpoint
        let status = runner.run_all("grading-pipeline").await.unwrap();
        assert_eq!(status, WorkflowStatus::Paused);

        // Resume past checkpoint
        let status = runner
            .resume(
                "grading-pipeline",
                "review",
                serde_json::json!({"approved": true, "reviewer": "teacher1"}),
            )
            .await
            .unwrap();
        assert_eq!(status, WorkflowStatus::Completed);

        let state = runner.get_state("grading-pipeline").await.unwrap().unwrap();
        assert!(state.step_runs.iter().all(|s| s.status == StepStatus::Done));

        // Verify review input is in context
        let steps_ctx = state.context.get("steps").unwrap();
        assert_eq!(steps_ctx["review"]["approved"], true);
        assert_eq!(steps_ctx["review"]["reviewer"], "teacher1");
    }

    #[tokio::test]
    async fn multi_entry_grade_only_skips_to_grade() {
        let workflow = grading_workflow().apply_entry_point("grade_only").unwrap();

        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let runner = WorkflowRunner::new(store, MockExecutor::new());
        let status = runner.run_all(&workflow.id()).await.unwrap();

        // Should complete — no checkpoint in the way (review is pre-filled)
        assert_eq!(status, WorkflowStatus::Completed);

        let state = runner.get_state(&workflow.id()).await.unwrap().unwrap();
        assert_eq!(state.step_runs[0].status, StepStatus::Skipped); // detect
        assert_eq!(state.step_runs[1].status, StepStatus::Skipped); // create_content
        assert_eq!(state.step_runs[2].status, StepStatus::Skipped); // configure_eval
        assert_eq!(state.step_runs[3].status, StepStatus::Skipped); // review (preset)
        assert_eq!(state.step_runs[4].status, StepStatus::Done); // grade
        assert_eq!(state.step_runs[5].status, StepStatus::Done); // report

        // Preset results should be in context
        let steps_ctx = state.context.get("steps").unwrap();
        assert_eq!(steps_ctx["configure_eval"]["rubric_id"], "r1");
        assert!(steps_ctx["detect"]["documents"].is_array());
    }

    #[tokio::test]
    async fn multi_entry_review_and_grade_pauses_at_checkpoint() {
        let workflow = grading_workflow()
            .apply_entry_point("review_and_grade")
            .unwrap();

        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let runner = WorkflowRunner::new(store, MockExecutor::new());
        let status = runner.run_all(&workflow.id()).await.unwrap();

        // Should pause at review checkpoint
        assert_eq!(status, WorkflowStatus::Paused);

        let state = runner.get_state(&workflow.id()).await.unwrap().unwrap();
        assert_eq!(state.step_runs[0].status, StepStatus::Skipped); // detect
        assert_eq!(state.step_runs[1].status, StepStatus::Skipped); // create_content
        assert_eq!(state.step_runs[2].status, StepStatus::Skipped); // configure_eval
        assert_eq!(state.step_runs[3].status, StepStatus::WaitingForInput); // review

        // Resume and complete
        let status = runner
            .resume(
                &workflow.id(),
                "review",
                serde_json::json!({"approved": true}),
            )
            .await
            .unwrap();
        assert_eq!(status, WorkflowStatus::Completed);
    }

    #[tokio::test]
    async fn multi_entry_grade_only_with_input() {
        let workflow = grading_workflow()
            .apply_entry_point("grade_only")
            .unwrap()
            .with_input(serde_json::json!({"activity_id": "act-123"}))
            .unwrap();

        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = ContextCapturingExecutor::new(vec![
            ("grade", serde_json::json!({"scores": [85, 92, 78]})),
            ("report", serde_json::json!({"report_url": "/reports/123"})),
        ]);
        let captured = executor.captured();
        let runner = WorkflowRunner::new(store, executor);
        let status = runner.run_all(&workflow.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Completed);

        // Verify input was available to executed steps
        let caps = captured.lock().await;
        assert_eq!(caps.len(), 2); // Only grade and report executed
        assert_eq!(caps[0].0, "grade");
        assert_eq!(caps[1].0, "report");
    }

    #[tokio::test]
    async fn multi_entry_with_data_flow_through_preset_results() {
        // Grade step uses {steps.configure_eval.rubric_id} via input mapping
        let steps = vec![
            WorkflowStep::api_call("detect", "Detect", "POST", "/detect"),
            WorkflowStep::api_call("eval", "Eval", "POST", "/eval").with_depends_on(vec!["detect"]),
            WorkflowStep::api_call("grade", "Grade", "POST", "/grade")
                .with_depends_on(vec!["eval"])
                .with_input_mapping(serde_json::json!({
                    "rubric": "{steps.eval.rubric_id}",
                    "activity": "{input.activity_id}"
                })),
        ];

        let mut workflow = WorkflowRun::from_steps(steps)
            .with_id("data-flow-ep")
            .with_entry_points(vec![EntryPoint {
                id: "grade_only".to_string(),
                label: "Grade Only".to_string(),
                description: None,
                starts_at: "grade".to_string(),
                preset_results: HashMap::from([
                    ("detect".to_string(), serde_json::json!({"docs": ["d1"]})),
                    (
                        "eval".to_string(),
                        serde_json::json!({"rubric_id": "rubric-abc"}),
                    ),
                ]),
                required_inputs: vec!["activity_id".to_string()],
                trigger: None,
            }]);

        workflow = workflow
            .apply_entry_point("grade_only")
            .unwrap()
            .with_input(serde_json::json!({"activity_id": "act-456"}))
            .unwrap();

        let executor =
            ContextCapturingExecutor::new(vec![("grade", serde_json::json!({"score": 95}))]);
        let captured = executor.captured();

        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();
        let runner = WorkflowRunner::new(store, executor);
        let status = runner.run_all("data-flow-ep").await.unwrap();
        assert_eq!(status, WorkflowStatus::Completed);

        // The grade step should have received resolved preset data
        let caps = captured.lock().await;
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].0, "grade");
        assert_eq!(caps[0].1["rubric"], "rubric-abc");
        assert_eq!(caps[0].1["activity"], "act-456");
    }

    #[tokio::test]
    async fn checkpoint_step_pauses_and_resumes() {
        // Checkpoint kind (not WaitForInput) — should execute as a regular step via MockExecutor
        let steps = vec![
            WorkflowStep::api_call("fetch", "Fetch", "GET", "/data"),
            WorkflowStep::checkpoint("verify", "Verify Data", "Check the fetched data is correct")
                .with_depends_on(vec!["fetch"]),
            WorkflowStep::api_call("save", "Save", "POST", "/save").with_depends_on(vec!["verify"]),
        ];
        let workflow = WorkflowRun::from_steps(steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let runner = WorkflowRunner::new(store, MockExecutor::new());
        let status = runner.run_all(&workflow.id()).await.unwrap();

        // Checkpoint kind goes through executor, should complete
        assert_eq!(status, WorkflowStatus::Completed);
    }

    #[tokio::test]
    async fn multiple_entry_points_each_work_independently() {
        let workflow = grading_workflow();

        // Test that both entry points produce valid workflows
        let ep1 = workflow.clone().apply_entry_point("grade_only").unwrap();
        let ep2 = workflow
            .clone()
            .apply_entry_point("review_and_grade")
            .unwrap();

        // grade_only: 4 skipped (detect, create_content, configure_eval, review), 2 pending
        let skipped1: Vec<_> = ep1
            .step_runs
            .iter()
            .filter(|s| s.status == StepStatus::Skipped)
            .collect();
        let pending1: Vec<_> = ep1
            .step_runs
            .iter()
            .filter(|s| s.status == StepStatus::Pending)
            .collect();
        assert_eq!(skipped1.len(), 4);
        assert_eq!(pending1.len(), 2);

        // review_and_grade: 3 skipped (detect, create_content, configure_eval), 3 pending (review, grade, report)
        let skipped2: Vec<_> = ep2
            .step_runs
            .iter()
            .filter(|s| s.status == StepStatus::Skipped)
            .collect();
        let pending2: Vec<_> = ep2
            .step_runs
            .iter()
            .filter(|s| s.status == StepStatus::Pending)
            .collect();
        assert_eq!(skipped2.len(), 3);
        assert_eq!(pending2.len(), 3);
    }

    #[tokio::test]
    async fn entry_point_with_parallel_fan_out() {
        // Workflow: detect → [analyze_a, analyze_b] (parallel) → merge → report
        // Entry point starts at merge — which depends on both parallel steps,
        // so detect + analyze_a + analyze_b are all skipped with preset results
        let steps = vec![
            WorkflowStep::api_call("detect", "Detect", "POST", "/detect"),
            WorkflowStep::api_call("analyze_a", "Analyze A", "POST", "/a")
                .with_depends_on(vec!["detect"])
                .parallel(),
            WorkflowStep::api_call("analyze_b", "Analyze B", "POST", "/b")
                .with_depends_on(vec!["detect"])
                .parallel(),
            WorkflowStep::api_call("merge", "Merge", "POST", "/merge")
                .with_depends_on(vec!["analyze_a", "analyze_b"]),
            WorkflowStep::api_call("report", "Report", "POST", "/report")
                .with_depends_on(vec!["merge"]),
        ];

        let workflow = WorkflowRun::from_steps(steps)
            .with_id("parallel-entry")
            .with_entry_points(vec![EntryPoint {
                id: "from_merge".to_string(),
                label: "From Merge".to_string(),
                description: None,
                starts_at: "merge".to_string(),
                preset_results: HashMap::from([
                    (
                        "detect".to_string(),
                        serde_json::json!({"items": ["i1", "i2"]}),
                    ),
                    (
                        "analyze_a".to_string(),
                        serde_json::json!({"result_a": "done"}),
                    ),
                    (
                        "analyze_b".to_string(),
                        serde_json::json!({"result_b": "done"}),
                    ),
                ]),
                required_inputs: vec![],
                trigger: None,
            }]);

        let applied = workflow.apply_entry_point("from_merge").unwrap();
        let store = InMemoryStore::new();
        store.save(&applied).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);
        let status = runner.run_all("parallel-entry").await.unwrap();
        assert_eq!(status, WorkflowStatus::Completed);

        let state = runner.get_state("parallel-entry").await.unwrap().unwrap();
        assert_eq!(state.step_runs[0].status, StepStatus::Skipped); // detect
        assert_eq!(state.step_runs[1].status, StepStatus::Skipped); // analyze_a
        assert_eq!(state.step_runs[2].status, StepStatus::Skipped); // analyze_b
        assert_eq!(state.step_runs[3].status, StepStatus::Done); // merge
        assert_eq!(state.step_runs[4].status, StepStatus::Done); // report
    }

    #[tokio::test]
    async fn entry_point_checkpoint_events_emitted() {
        use std::sync::Mutex;

        struct CollectingSink {
            events: Mutex<Vec<WorkflowEvent>>,
        }

        #[async_trait::async_trait]
        impl EventSink for CollectingSink {
            async fn emit(&self, event: WorkflowEvent) {
                self.events.lock().unwrap().push(event);
            }
        }

        let workflow = grading_workflow()
            .apply_entry_point("review_and_grade")
            .unwrap();

        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let sink = CollectingSink {
            events: Mutex::new(vec![]),
        };
        let runner = WorkflowRunner::with_events(store, MockExecutor::new(), sink);

        // Run to checkpoint
        let status = runner.run_all(&workflow.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Paused);

        let events = runner.events.events.lock().unwrap();
        // Should have: workflow_started, step_waiting (review)
        assert!(events
            .iter()
            .any(|e| matches!(e, WorkflowEvent::WorkflowStarted { .. })));
        assert!(events.iter().any(
            |e| matches!(e, WorkflowEvent::StepWaiting { step_id, .. } if step_id == "review")
        ));

        // No step_started for skipped steps
        let started_ids: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                WorkflowEvent::StepStarted { step_id, .. } => Some(step_id.clone()),
                _ => None,
            })
            .collect();
        assert!(!started_ids.contains(&"detect".to_string()));
        assert!(!started_ids.contains(&"create_content".to_string()));
        assert!(!started_ids.contains(&"configure_eval".to_string()));
    }

    #[tokio::test]
    async fn entry_point_with_skip_if_interaction() {
        // Workflow where skip_if and entry point interact:
        // detect (skip_if activity_id) → eval → grade
        // Entry from eval with activity_id input
        let steps = vec![
            WorkflowStep::api_call("detect", "Detect", "POST", "/detect")
                .with_skip_if("{input.activity_id}"),
            WorkflowStep::api_call("eval", "Eval", "POST", "/eval").with_depends_on(vec!["detect"]),
            WorkflowStep::api_call("grade", "Grade", "POST", "/grade")
                .with_depends_on(vec!["eval"]),
        ];

        // Using entry point to skip to eval
        let workflow = WorkflowRun::from_steps(steps)
            .with_id("skip-entry-combo")
            .with_entry_points(vec![EntryPoint {
                id: "from_eval".to_string(),
                label: "From Eval".to_string(),
                description: None,
                starts_at: "eval".to_string(),
                preset_results: HashMap::new(),
                required_inputs: vec![],
                trigger: None,
            }])
            .apply_entry_point("from_eval")
            .unwrap()
            .with_input(serde_json::json!({"activity_id": "act-789"}))
            .unwrap();

        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let runner = WorkflowRunner::new(store, MockExecutor::new());
        let status = runner.run_all("skip-entry-combo").await.unwrap();
        assert_eq!(status, WorkflowStatus::Completed);

        let state = runner.get_state("skip-entry-combo").await.unwrap().unwrap();
        assert_eq!(state.step_runs[0].status, StepStatus::Skipped); // detect (by entry point)
        assert_eq!(state.step_runs[1].status, StepStatus::Done); // eval
        assert_eq!(state.step_runs[2].status, StepStatus::Done); // grade
    }

    #[tokio::test]
    async fn checkpoint_with_external_strategy_serializes() {
        let workflow = grading_workflow().with_checkpoint(CheckpointStrategy::External {
            tool_name: "redis_checkpoint".to_string(),
        });

        let json = serde_json::to_value(&workflow).unwrap();
        assert_eq!(json["checkpoint"]["type"], "external");
        assert_eq!(json["checkpoint"]["tool_name"], "redis_checkpoint");
        assert_eq!(json["entry_points"].as_array().unwrap().len(), 2);

        let parsed: WorkflowDefinition = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.entry_points.len(), 2);
        assert_eq!(parsed.entry_points[0].id, "grade_only");
        assert_eq!(parsed.entry_points[1].id, "review_and_grade");
    }

    #[tokio::test]
    async fn multi_entry_workflow_full_json_roundtrip() {
        let workflow = grading_workflow();
        let json_str = serde_json::to_string_pretty(&workflow).unwrap();
        let parsed: WorkflowDefinition = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.id, "grading-pipeline");
        assert_eq!(parsed.steps.len(), 6);
        assert_eq!(parsed.entry_points.len(), 2);

        // Entry points preserved
        let ep = parsed.entry_point("grade_only").unwrap();
        assert_eq!(ep.starts_at, "grade");
        assert_eq!(ep.required_inputs, vec!["activity_id"]);
        assert!(ep.preset_results.contains_key("configure_eval"));
        assert!(ep.preset_results.contains_key("review"));

        let ep2 = parsed.entry_point("review_and_grade").unwrap();
        assert_eq!(ep2.starts_at, "review");
    }

    #[test]
    fn step_kind_reply_round_trips() {
        let json = serde_json::json!({
            "type": "reply",
            "text": "Your classes:",
            "buttons_from": "{steps.list.result.classes}",
            "button_template": {
                "kind": "callback", "label": "{item.name}",
                "callback_data": "wf:open_class:{item.id}"
            }
        });
        let kind: StepKind = serde_json::from_value(json.clone()).unwrap();
        match &kind {
            StepKind::Reply { text, buttons, buttons_from, button_template } => {
                assert_eq!(text, "Your classes:");
                assert!(buttons.is_empty());
                assert_eq!(buttons_from.as_deref(), Some("{steps.list.result.classes}"));
                assert!(button_template.is_some());
            }
            _ => panic!("expected Reply"),
        }
        assert_eq!(serde_json::to_value(&kind).unwrap(), json);
    }

    #[test]
    fn step_kind_reply_text_only() {
        let kind: StepKind = serde_json::from_value(
            serde_json::json!({"type":"reply","text":"Hi"}),
        )
        .unwrap();
        assert!(matches!(kind, StepKind::Reply { .. }));
    }

    #[test]
    fn entry_point_parses_slash_trigger() {
        let json = serde_json::json!({
            "id": "join", "label": "Join", "starts_at": "ask_code",
            "trigger": {"type": "slash", "name": "/join"}
        });
        let ep: EntryPoint = serde_json::from_value(json).unwrap();
        assert_eq!(ep.id, "join");
        assert!(matches!(
            ep.trigger,
            Some(distri_types::channel_commands::ChannelTrigger::Slash { .. })
        ));
    }

    #[test]
    fn entry_point_trigger_defaults_none() {
        let json = serde_json::json!({"id":"x","label":"X","starts_at":"s"});
        let ep: EntryPoint = serde_json::from_value(json).unwrap();
        assert!(ep.trigger.is_none());
    }
}
