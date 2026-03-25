# distri-workflow — Roadmap

## Planned Changes

### 1. Richer StepKind::Script

Current `Script` is just `{ command, args }`. It needs to be richer — closer to Browsr's `ShellRequest` pattern with execution context metadata.

**Current:**
```rust
Script {
    command: String,
    args: Vec<String>,
}
```

**Proposed:**
```rust
Script {
    command: String,
    args: Vec<String>,
    /// Working directory
    cwd: Option<String>,
    /// Environment variables
    env: Option<HashMap<String, String>>,
    /// Timeout in seconds
    timeout_secs: Option<u64>,
    /// Expected output format: text | json | stream
    output_format: Option<String>,
    /// Shell type: bash | sh | zsh | powershell
    shell: Option<String>,
}
```

### 2. New StepKind::ToolCall

Add ability to make a Distri tool call as a step — distinct from AgentRun (which is a full agent session) and ApiCall (which is raw HTTP).

```rust
ToolCall {
    /// Tool name (e.g., "read_document", "create_activity")
    tool_name: String,
    /// Tool input parameters
    input: serde_json::Value,
    /// Which agent context to execute in (optional)
    agent_id: Option<String>,
}
```

A `ToolCall` is a single tool invocation without a full agent loop. The executor calls the tool directly and returns the result.

### 3. Step Requirements (`requires`)

Steps should declare what capabilities they need to execute. The runner checks requirements before dispatching to an executor.

```rust
WorkflowStep {
    // ... existing fields ...

    /// Capabilities required to run this step.
    /// The runner checks these against available executors.
    requires: Vec<StepRequirement>,
}

enum StepRequirement {
    /// Needs shell/terminal access
    Shell,
    /// Needs browser automation (Browsr)
    Browser,
    /// Needs network/HTTP access
    Network,
    /// Needs access to a specific agent
    Agent(String),
    /// Needs access to a specific tool
    Tool(String),
    /// Needs Google OAuth token
    GoogleAuth,
    /// Custom capability
    Custom(String),
}
```

**How it works:**

When the runner encounters a step, it checks `requires` against the available executors:

```rust
// Runner checks requirements before executing
fn can_execute(&self, step: &WorkflowStep) -> bool {
    step.requires.iter().all(|req| self.executor.supports(req))
}
```

If requirements aren't met, the step is marked as `blocked` (new status) with a message saying what's missing. The UI can prompt the user to connect the required capability.

**Example:**
```json
{
  "id": "read_doc",
  "kind": { "type": "api_call", "method": "GET", "url": "/admin/google-drive/files/{id}/content" },
  "requires": ["network", "google_auth"]
}
```

```json
{
  "id": "run_tests",
  "kind": { "type": "script", "command": "cargo test", "cwd": "/project" },
  "requires": ["shell"]
}
```

```json
{
  "id": "fill_form",
  "kind": { "type": "script", "command": "browsr navigate https://example.com && browsr click #submit" },
  "requires": ["browser"]
}
```

### 4. New StepStatus::Blocked

Add a `blocked` status for steps whose requirements can't be met:

```rust
enum StepStatus {
    Pending,
    Blocked,    // NEW — requirements not met
    Running,
    Done,
    Failed,
    Skipped,
}
```

### 5. Executor Capability Registration

The `StepExecutor` trait gets a capability method:

```rust
trait StepExecutor {
    async fn execute(&self, step, context) -> StepResult;

    /// What capabilities this executor provides.
    fn capabilities(&self) -> Vec<StepRequirement> {
        vec![] // default: no declared capabilities (accepts everything)
    }

    /// Check if this executor can handle a specific requirement.
    fn supports(&self, requirement: &StepRequirement) -> bool {
        true // default: accepts everything (backward compatible)
    }
}
```

## Priority

1. **StepRequirements** — high priority, needed for multi-environment workflows
2. **StepKind::ToolCall** — high priority, common pattern
3. **Richer Script** — medium, needed for shell-based workflows
4. **StepStatus::Blocked** — medium, follows from requirements
5. **Executor capabilities** — medium, follows from requirements
