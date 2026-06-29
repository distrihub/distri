# distri-workflow — Architecture

## Overview

A workflow engine that defines, executes, and tracks multi-step workflows as data. Workflows are JSON documents describing a DAG of steps. Steps can be API calls, agent runs, scripts, or conditions. The engine handles sequential/parallel execution, dependency resolution, context propagation, and state persistence.

## Core Concepts

### Workflow as Data

A workflow is a JSON document — not code. It describes what to do, not how. This means:

- Workflows are portable (pass them between services, store in DB, send to agents)
- Workflows are inspectable (UI can render progress from the JSON)
- Workflows are resumable (pick up from the last completed step after a crash)
- Workflows are versionable (diff, audit, rollback)

### Separation of Concerns

```
WorkflowDefinition    — WHAT to do (data)
StepExecutor trait     — HOW to do it (implementation)
WorkflowStateStore    — WHERE to persist state (storage)
WorkflowRunner        — orchestration (engine)
```

The engine doesn't know how to make API calls or run agents. It delegates to the `StepExecutor` trait. The engine doesn't know where state is stored — it delegates to `WorkflowStateStore`.

## Types

### WorkflowDefinition

```rust
WorkflowDefinition {
    id: String,                    // unique workflow instance ID
    workflow_type: String,         // "bulk_import", "grading", etc.
    status: WorkflowStatus,        // pending | running | paused | completed | failed
    current_step: usize,           // index of current/last step
    context: serde_json::Value,    // shared data between steps
    steps: Vec<WorkflowStep>,      // the step DAG
    notes: Vec<WorkflowNote>,      // log entries
    created_at, updated_at,
}
```

### WorkflowStep

```rust
WorkflowStep {
    id: String,                    // unique step ID (e.g., "read_doc")
    label: String,                 // human-readable (e.g., "Read Google Doc")
    kind: StepKind,                // what this step does
    status: StepStatus,            // pending | running | done | failed | skipped
    result: Option<Value>,         // step output (after execution)
    error: Option<String>,         // error message (if failed)
    depends_on: Vec<String>,       // step IDs that must complete first
    execution: StepExecution,      // sequential | parallel
}
```

### StepKind — What a step does

```rust
enum StepKind {
    // HTTP API call
    ApiCall { method, url, body?, headers? },

    // Shell script / command
    Script { command, args },

    // Delegate to a Distri agent
    AgentRun { agent_id, prompt, tools },

    // Conditional branch (evaluate expression against context)
    Condition { expression, if_true, if_false? },

    // Manual checkpoint / marker
    Checkpoint { message },
}
```

### StepExecution — How steps run

- **Sequential** (default): waits for the previous step to complete
- **Parallel**: can run concurrently with other parallel steps at the same level

Dependencies (`depends_on`) override execution mode — a parallel step still waits for its dependencies.

## Execution Model

### Sequential Steps

```
Step A → Step B → Step C
```

Steps run one at a time. B doesn't start until A completes.

### Parallel Steps

```
Step A ──┐
Step B ──┼── Step D (depends_on: [A, B, C])
Step C ──┘
```

A, B, C all marked `parallel`. They run in one batch. D has `depends_on: [A, B, C]` — it waits for all three.

### Dependency Resolution

`WorkflowDefinition::runnable_steps()` returns all steps whose:
1. Status is `pending`
2. All `depends_on` step IDs have status `done`

The runner calls `runnable_steps()`, executes them, then calls it again for the next batch.

### Context Propagation

Steps share data via `workflow.context` (a JSON object). When a step completes, its `StepResult.context_updates` are merged into the context:

```
Step 1: returns context_updates: { "doc_content": "..." }
Step 2: can read context.doc_content
```

This is how data flows between steps without coupling them.

## Traits

### WorkflowStateStore

```rust
trait WorkflowStateStore {
    async fn load(workflow_id) -> Option<WorkflowDefinition>;
    async fn save(workflow) -> ();
    async fn commit_step(workflow_id, step_index, result) -> ();
}
```

Implementations:
- **InMemoryStore** — for testing
- **RedisStore** — transient state during execution (fast reads/writes)
- **DbStore** — permanent state (e.g., activity.config JSONB column)

The runner uses Redis during execution for speed, but the application can load/save to DB for durability.

### StepExecutor

```rust
trait StepExecutor {
    async fn execute(step, context) -> StepResult;
}
```

Implementations:
- **PrintExecutor** — logs steps (CLI testing)
- **HttpExecutor** — makes real HTTP API calls
- **AgentExecutor** — delegates to Distri agent runtime
- **CompositeExecutor** — dispatches to different executors based on StepKind

## API Integration (distri-cloud)

### Endpoints

```
POST /v1/workflows              — create a workflow
GET  /v1/workflows/{id}         — get workflow state
POST /v1/workflows/{id}/run     — run next step(s)
POST /v1/workflows/{id}/run-all — run all steps to completion
GET  /v1/workflows/{id}/events  — SSE stream of step progress
```

### Storage

- During execution: Redis (fast state updates as steps complete)
- Permanent: PostgreSQL `workflows` table or application-specific storage
- The `WorkflowStateStore` trait abstracts this — callers don't care

### Agent Integration

When a step has `kind: AgentRun`, the executor:
1. Creates a Distri agent session
2. Sends the prompt with available tools
3. Waits for the agent to complete
4. Returns the agent's output as the step result

This means workflows can mix API calls with AI agent runs seamlessly.

## Client Libraries

### Rust (distri crate)

```rust
use distri::workflow::*;

let workflow = WorkflowDefinition::new("import", steps)
    .with_context(json!({ "file_id": "abc" }));

let runner = WorkflowRunner::new(store, executor);
runner.run_all(&workflow.id).await;
```

### TypeScript (@distri/core)

```typescript
import { WorkflowDefinition, WorkflowStep, workflowProgress } from '@distri/core'
```

### React (@distri/react)

```tsx
import { useWorkflow, WorkflowProgress } from '@distri/react'

const { workflow, runAll, isRunning, progress } = useWorkflow({
  workflow: initialWorkflow,
  onExecuteStep: async (stepId, step, context) => {
    // Make API call, run agent, etc.
    return { status: 'done', result: { ... } }
  },
})

return <WorkflowProgress workflow={workflow} />
```

### CLI

```bash
distri workflow run workflow.json           # run all steps
distri workflow run workflow.json --step    # step-by-step (interactive)
distri workflow status workflow.json        # show progress
```

## Example: Bulk Import Workflow

```json
{
  "workflow_type": "bulk_import",
  "context": { "file_id": "1JxO60...", "class_id": "177c47..." },
  "steps": [
    {
      "id": "read_doc",
      "label": "Read Google Doc",
      "kind": { "type": "api_call", "method": "GET", "url": "/admin/google-drive/files/{file_id}/content" },
      "depends_on": []
    },
    {
      "id": "detect",
      "label": "Detect lesson structure",
      "kind": { "type": "agent_run", "agent_id": "activity_importer", "prompt": "Analyze this document..." },
      "depends_on": ["read_doc"]
    },
    {
      "id": "create_activity",
      "label": "Create activity with questions",
      "kind": { "type": "api_call", "method": "POST", "url": "/admin/activities", "body": { ... } },
      "depends_on": ["detect"]
    },
    {
      "id": "add_submissions",
      "label": "Add student submissions",
      "kind": { "type": "api_call", "method": "POST", "url": "/admin/activities/{activity_id}/bulk-submissions" },
      "depends_on": ["create_activity"]
    }
  ]
}
```

## Design Principles

1. **Workflows are data, not code** — JSON in, JSON out
2. **Traits for everything** — executor, storage, all pluggable
3. **Resumable by default** — state persisted after every step
4. **Observable** — UI can render progress from the workflow JSON
5. **Agent-compatible** — steps can be API calls OR agent runs
6. **Parallel-aware** — steps declare their execution mode + dependencies
