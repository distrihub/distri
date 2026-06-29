# distri-workflow — Roadmap

## Implemented

See [design specs](../../docs/superpowers/specs/2026-03-25-workflow-enhancements-design.md) for full details.

### 1. Step Requirements (unified skill model)

`StepRequirement` struct with namespaced skill identifiers:
- `native:shell`, `native:browser`, `native:network` — built-in capabilities
- `{provider}:{service}` — connections (e.g., `google:drive`, `slack:chat`)
- Permissions/scopes per requirement
- Validation at workflow creation

### 2. StepKind::ToolCall

Single tool invocation with optional agent context:
```rust
ToolCall { tool_name, input, agent_id: Option<String> }
```

### 3. Richer StepKind::Script

Added: `cwd`, `env`, `timeout_secs`, `output_format` (Text/Json/Stream), `shell` (Bash/Sh/Zsh).

### 4. StepStatus::Blocked + WorkflowStatus::Blocked

Steps with unmet requirements are `Blocked`. `is_stuck()` detects when no progress is possible.

### 5. Executor Capability Registration

`StepExecutor::supports()` for execution gating. `available_skills()` for UI introspection.

### 6. CheckpointStrategy

- **Internal** — Redis-based, thread+task scoped, TTL
- **External** — client-registered tool call for save/load/list

## In Progress

See [system design spec](../../docs/superpowers/specs/2026-03-25-workflow-system-design.md).

### 7. WorkflowStore + APIs

- `WorkflowStore` trait in distri-types
- CRUD + execution API routes in distri-server
- Postgres store in cloud with public/shared/starred workflows

### 8. Agentic Workflow Steps

- Inline agents with skills and model overrides
- Named agent steps that load agent definitions
- Skill-aware steps that inject skills into agent context

## Future

### 9. Workflow Editor & Visualizer (distri-home)

- **Workflow Canvas** — drag-and-drop visual editor
- **Step Library** — palette of step types (API, Script, Agent, Tool, Condition)
- **Live Execution View** — real-time progress visualization via SSE
- **Template Gallery** — browse and clone public workflow templates
- **Connection Requirement UI** — blocked step shows missing connections with one-click setup

### 10. Advanced Execution

- SSE event streaming for step progress
- Parallel step fan-out with configurable concurrency limits
- Step retry policies (exponential backoff, max retries)
- Workflow composition (sub-workflows as steps)
- Workflow versioning and migration
