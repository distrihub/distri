#[cfg(test)]
mod tests {
    use crate::*;
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
            Ok(StepResult::done(
                serde_json::json!({ "step_id": step.id }),
            ))
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
        let workflow = WorkflowDefinition::new("test", steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);

        let status = runner.run_all(&workflow.id).await.unwrap();
        assert_eq!(status, WorkflowStatus::Completed);

        let final_state = runner.get_state(&workflow.id).await.unwrap().unwrap();
        assert_eq!(final_state.status, WorkflowStatus::Completed);
        assert!(final_state.steps.iter().all(|s| s.status == StepStatus::Done));
    }

    #[tokio::test]
    async fn parallel_steps_all_execute() {
        let steps = vec![
            WorkflowStep::api_call("a", "Step A", "GET", "/a").parallel(),
            WorkflowStep::api_call("b", "Step B", "GET", "/b").parallel(),
            WorkflowStep::api_call("c", "Step C", "GET", "/c").parallel(),
        ];
        let workflow = WorkflowDefinition::new("test", steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let call_count = executor.call_count.clone();
        let runner = WorkflowRunner::new(store, executor);

        let results = runner.run_next(&workflow.id).await.unwrap();
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
        let workflow = WorkflowDefinition::new("test", steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);

        let r1 = runner.run_next(&workflow.id).await.unwrap();
        assert_eq!(r1.len(), 1);
        assert_eq!(r1[0].0, "fetch");

        let r2 = runner.run_next(&workflow.id).await.unwrap();
        assert_eq!(r2.len(), 1);
        assert_eq!(r2[0].0, "process");

        let r3 = runner.run_next(&workflow.id).await.unwrap();
        assert_eq!(r3.len(), 1);
        assert_eq!(r3[0].0, "save");
    }

    #[tokio::test]
    async fn parallel_with_join_dependency() {
        let steps = vec![
            WorkflowStep::api_call("a", "Step A", "GET", "/a").parallel(),
            WorkflowStep::api_call("b", "Step B", "GET", "/b").parallel(),
            WorkflowStep::api_call("c", "Join step", "POST", "/c")
                .with_depends_on(vec!["a", "b"]),
        ];
        let workflow = WorkflowDefinition::new("test", steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);

        let r1 = runner.run_next(&workflow.id).await.unwrap();
        assert_eq!(r1.len(), 2);

        let r2 = runner.run_next(&workflow.id).await.unwrap();
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
        let workflow = WorkflowDefinition::new("test", steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::with_failures(vec!["fail"]);
        let runner = WorkflowRunner::new(store, executor);

        let status = runner.run_all(&workflow.id).await.unwrap();
        assert_eq!(status, WorkflowStatus::Failed);

        let state = runner.get_state(&workflow.id).await.unwrap().unwrap();
        assert_eq!(state.steps[0].status, StepStatus::Done);
        assert_eq!(state.steps[1].status, StepStatus::Failed);
        assert_eq!(state.steps[2].status, StepStatus::Pending);
    }

    #[tokio::test]
    async fn context_propagates_between_steps() {
        let steps = vec![
            WorkflowStep::api_call("step1", "First", "GET", "/1"),
            WorkflowStep::api_call("step2", "Second", "GET", "/2"),
        ];
        let workflow = WorkflowDefinition::new("test", steps)
            .with_context(serde_json::json!({ "initial": true }));
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);

        runner.run_all(&workflow.id).await.unwrap();

        let state = runner.get_state(&workflow.id).await.unwrap().unwrap();
        assert_eq!(state.context["initial"], true);
        assert_eq!(state.context["step1_done"], true);
        assert_eq!(state.context["step2_done"], true);
    }

    #[tokio::test]
    async fn run_next_on_completed_returns_empty() {
        let steps = vec![WorkflowStep::api_call("only", "Only step", "GET", "/")];
        let workflow = WorkflowDefinition::new("test", steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);

        runner.run_next(&workflow.id).await.unwrap();
        let results = runner.run_next(&workflow.id).await.unwrap();
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
        let workflow = WorkflowDefinition::new("import", steps)
            .with_context(serde_json::json!({"doc_id": "abc123"}));

        let json = serde_json::to_string_pretty(&workflow).unwrap();
        let parsed: WorkflowDefinition = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.workflow_type, "import");
        assert_eq!(parsed.steps.len(), 2);
        assert_eq!(parsed.steps[1].depends_on, vec!["read"]);
        assert_eq!(parsed.steps[1].execution, StepExecution::Parallel);
    }

    #[tokio::test]
    async fn notes_are_recorded() {
        let mut workflow = WorkflowDefinition::new("test", vec![]);
        workflow.add_note("step1", "Detected 10 essays");
        workflow.add_note("step2", "Created 10 submissions");

        assert_eq!(workflow.notes.len(), 2);
        assert_eq!(workflow.notes[0].message, "Detected 10 essays");
    }

    #[tokio::test]
    async fn empty_workflow_is_immediately_complete() {
        let workflow = WorkflowDefinition::new("empty", vec![]);
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
        let steps = vec![
            WorkflowStep::api_call("step1", "Needs shell", "GET", "/1").with_requires(vec![
                StepRequirement::native("shell"),
            ]),
        ];
        let workflow = WorkflowDefinition::new("test", steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        // Executor only supports network, not shell
        let executor = SkillAwareExecutor::with_skills(vec!["native:network"]);
        let runner = WorkflowRunner::new(store, executor);

        let results = runner.run_next(&workflow.id).await.unwrap();
        assert!(results.is_empty());

        let state = runner.get_state(&workflow.id).await.unwrap().unwrap();
        assert_eq!(state.steps[0].status, StepStatus::Blocked);
        assert!(state.steps[0].error.as_ref().unwrap().contains("native:shell"));
    }

    #[tokio::test]
    async fn requirements_met_allows_execution() {
        let steps = vec![
            WorkflowStep::api_call("step1", "Needs network", "GET", "/1").with_requires(vec![
                StepRequirement::native("network"),
            ]),
        ];
        let workflow = WorkflowDefinition::new("test", steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = SkillAwareExecutor::with_skills(vec!["native:network"]);
        let runner = WorkflowRunner::new(store, executor);

        let results = runner.run_next(&workflow.id).await.unwrap();
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
        let workflow = WorkflowDefinition::new("test", steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = SkillAwareExecutor::with_skills(vec!["native:network"]);
        let runner = WorkflowRunner::new(store, executor);

        let results = runner.run_next(&workflow.id).await.unwrap();
        // Only net_step should execute
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "net_step");

        let state = runner.get_state(&workflow.id).await.unwrap().unwrap();
        assert_eq!(state.steps[1].status, StepStatus::Blocked);
    }

    #[tokio::test]
    async fn blocked_workflow_status() {
        let steps = vec![
            WorkflowStep::api_call("step1", "Needs browser", "GET", "/1").with_requires(vec![
                StepRequirement::native("browser"),
            ]),
        ];
        let workflow = WorkflowDefinition::new("test", steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = SkillAwareExecutor::with_skills(vec!["native:network"]);
        let runner = WorkflowRunner::new(store, executor);

        let status = runner.run_all(&workflow.id).await.unwrap();
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
        let workflow = WorkflowDefinition::new("test", steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = SkillAwareExecutor::with_skills(vec!["native:network"]);
        let runner = WorkflowRunner::new(store, executor);

        let status = runner.run_all(&workflow.id).await.unwrap();
        assert_eq!(status, WorkflowStatus::Blocked);

        let state = runner.get_state(&workflow.id).await.unwrap().unwrap();
        assert_eq!(state.steps[0].status, StepStatus::Blocked);
        // Step waiting on blocked is still pending but workflow is stuck
        assert_eq!(state.steps[1].status, StepStatus::Pending);
        assert!(state.is_stuck());
    }

    #[tokio::test]
    async fn no_requirements_uses_default_executor() {
        // Steps without requires should work with any executor (backward compat)
        let steps = vec![
            WorkflowStep::api_call("step1", "No reqs", "GET", "/1"),
        ];
        let workflow = WorkflowDefinition::new("test", steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);

        let status = runner.run_all(&workflow.id).await.unwrap();
        assert_eq!(status, WorkflowStatus::Completed);
    }

    // ========================================================================
    // New Tests: ToolCall StepKind
    // ========================================================================

    #[tokio::test]
    async fn tool_call_step_executes() {
        let steps = vec![
            WorkflowStep::tool_call(
                "call_api",
                "Call API request tool",
                "api_request",
                serde_json::json!({"method": "GET", "path": "/v1/skills"}),
            ),
        ];
        let workflow = WorkflowDefinition::new("test", steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);

        let status = runner.run_all(&workflow.id).await.unwrap();
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
        let workflow = WorkflowDefinition::new("test", vec![]);
        match workflow.checkpoint {
            CheckpointStrategy::Internal { ttl_secs } => {
                assert_eq!(ttl_secs, None);
            }
            _ => panic!("Expected Internal checkpoint strategy"),
        }
    }

    #[tokio::test]
    async fn checkpoint_strategy_serializes() {
        let workflow = WorkflowDefinition::new("test", vec![])
            .with_checkpoint(CheckpointStrategy::External {
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
        let workflow = WorkflowDefinition::new("test", vec![])
            .with_checkpoint(CheckpointStrategy::Internal {
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
        let workflow = WorkflowDefinition::new(
            "test",
            vec![WorkflowStep::api_call("s", "Step", "GET", "/")],
        );
        assert!(!workflow.is_stuck());
    }

    #[tokio::test]
    async fn not_stuck_when_all_done() {
        let mut workflow = WorkflowDefinition::new(
            "test",
            vec![WorkflowStep::api_call("s", "Step", "GET", "/")],
        );
        workflow.steps[0].status = StepStatus::Done;
        assert!(!workflow.is_stuck());
    }

    #[tokio::test]
    async fn stuck_when_only_blocked_steps_remain() {
        let mut workflow = WorkflowDefinition::new(
            "test",
            vec![
                WorkflowStep::api_call("s1", "Step 1", "GET", "/1"),
                WorkflowStep::api_call("s2", "Step 2", "GET", "/2"),
            ],
        );
        workflow.steps[0].status = StepStatus::Done;
        workflow.steps[1].status = StepStatus::Blocked;
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

        let workflow = WorkflowDefinition::new("pipeline", steps)
            .with_context(serde_json::json!({"source": "api"}));

        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let executor = MockExecutor::new();
        let runner = WorkflowRunner::new(store, executor);

        let status = runner.run_all(&workflow.id).await.unwrap();
        assert_eq!(status, WorkflowStatus::Completed);

        let state = runner.get_state(&workflow.id).await.unwrap().unwrap();
        assert!(state.steps.iter().all(|s| s.status == StepStatus::Done));
    }

    #[tokio::test]
    async fn requirement_serialization_roundtrip() {
        let req = StepRequirement::connection("google", "drive")
            .with_permissions(vec!["drive.readonly"]);

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
            "workflow_type": "bulk_import",
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

        let workflow: WorkflowDefinition = serde_json::from_value(json).unwrap();
        assert_eq!(workflow.id, "test-001");
        assert!(workflow.steps[0].requires.is_empty());
        // Checkpoint defaults to Internal
        match workflow.checkpoint {
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
        let workflow = WorkflowDefinition::new("test", vec![])
            .with_context(serde_json::json!({"existing": true}))
            .with_input(serde_json::json!({"file_id": "abc", "class_id": "xyz"}))
            .unwrap();

        assert_eq!(workflow.context["existing"], true);
        assert_eq!(workflow.context["file_id"], "abc");
        assert_eq!(workflow.context["class_id"], "xyz");
    }

    #[tokio::test]
    async fn workflow_serializes_with_input_schema() {
        let mut workflow = WorkflowDefinition::new("import", vec![]);
        workflow.input_schema = Some(serde_json::json!({
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
        let mut workflow = WorkflowDefinition::new("test", vec![]);
        workflow.input_schema = Some(serde_json::json!({
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
        let mut workflow = WorkflowDefinition::new("test", vec![]);
        workflow.input_schema = Some(serde_json::json!({
            "type": "object",
            "properties": { "count": { "type": "integer" } }
        }));

        let result = workflow.with_input(serde_json::json!({ "count": "not a number" }));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn input_validation_accepts_valid_input() {
        let mut workflow = WorkflowDefinition::new("test", vec![]);
        workflow.input_schema = Some(serde_json::json!({
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
        let workflow = WorkflowDefinition::new("test", vec![]);
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
        let workflow = WorkflowDefinition::new("test", steps);
        let store = InMemoryStore::new();
        store.save(&workflow).await.unwrap();

        let sink = CollectingSink { events: Mutex::new(vec![]) };
        let executor = MockExecutor::new();
        let runner = WorkflowRunner::with_events(store, executor, sink);

        runner.run_all(&workflow.id).await.unwrap();

        let events = runner.events.events.lock().unwrap();
        // Should have: started, step1_started, step1_completed, step2_started, step2_completed, workflow_completed
        assert!(events.len() >= 5, "Expected at least 5 events, got {}", events.len());

        // First event should be WorkflowStarted
        matches!(&events[0], WorkflowEvent::WorkflowStarted { .. });
        // Last should be WorkflowCompleted
        matches!(events.last().unwrap(), WorkflowEvent::WorkflowCompleted { .. });
    }
}
