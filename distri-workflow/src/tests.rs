#[cfg(test)]
mod tests {
    use crate::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Mock executor that records which steps were executed.
    struct MockExecutor {
        call_count: Arc<AtomicUsize>,
        fail_steps: Vec<String>,
    }

    impl MockExecutor {
        fn new() -> Self {
            Self { call_count: Arc::new(AtomicUsize::new(0)), fail_steps: vec![] }
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
        async fn execute(&self, step: &WorkflowStep, context: &serde_json::Value) -> Result<StepResult, String> {
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
        // All 3 parallel steps should execute in one run_next call
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

        // First run: only "fetch" is runnable
        let r1 = runner.run_next(&workflow.id).await.unwrap();
        assert_eq!(r1.len(), 1);
        assert_eq!(r1[0].0, "fetch");

        // Second run: "process" is now unblocked
        let r2 = runner.run_next(&workflow.id).await.unwrap();
        assert_eq!(r2.len(), 1);
        assert_eq!(r2[0].0, "process");

        // Third run: "save" is now unblocked
        let r3 = runner.run_next(&workflow.id).await.unwrap();
        assert_eq!(r3.len(), 1);
        assert_eq!(r3[0].0, "save");
    }

    #[tokio::test]
    async fn parallel_with_join_dependency() {
        // A and B run in parallel, C depends on both
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

        // First: A and B run in parallel
        let r1 = runner.run_next(&workflow.id).await.unwrap();
        assert_eq!(r1.len(), 2);

        // Second: C can now run (both deps done)
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
        assert_eq!(state.steps[2].status, StepStatus::Pending); // never reached
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
        // Context should have initial + step1_done + step2_done
        assert_eq!(state.context["initial"], true);
        assert_eq!(state.context["step1_done"], true);
        assert_eq!(state.context["step2_done"], true);
    }

    #[tokio::test]
    async fn run_next_on_completed_returns_empty() {
        let steps = vec![
            WorkflowStep::api_call("only", "Only step", "GET", "/"),
        ];
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
}
