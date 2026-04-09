# Local Tools Available to All Agents — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make CLI-registered local tools (Bash, Read, Write, Edit, Glob, Grep, execute_command) available to any agent running locally, not just `distri_runner`.

**Architecture:** Three changes: (1) register tool handlers under wildcard `"*"` agent so any agent can use them, (2) always include local tool names in `external_tool_names` so the stream client intercepts them, (3) make `is_external_tool()` also check the registry as a fallback. This fixes the 120s timeout when non-distri_runner agents call local tools.

**Tech Stack:** Rust, distri-cli, distri client library

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `distri-cli/src/tools/mod.rs` | Modify lines 24-38 | Register handlers under `"*"` + export `LOCAL_TOOL_NAMES` constant |
| `distri-cli/src/main.rs` | Modify lines 407-434 | Add local tool names to `external_tool_names` in run command |
| `distri-cli/src/chat.rs` | Modify lines 478-489, 574-607 | Add local tool names to `external_tool_names` in chat mode |
| `distri/src/client_stream.rs` | Modify lines 152-155 | Fallback to registry check in `is_external_tool()` |

---

### Task 1: Register local tools under wildcard agent

**Files:**
- Modify: `distri-cli/src/tools/mod.rs:24-38`

- [ ] **Step 1: Add `LOCAL_TOOL_NAMES` constant and change `register_all` to use `"*"`**

In `distri-cli/src/tools/mod.rs`, add a constant listing all local tool names and change `register_all` to register under `"*"`:

```rust
/// Names of all tools the CLI registers locally.
/// Used to ensure the stream client intercepts these tool calls
/// regardless of which agent is running.
pub const LOCAL_TOOL_NAMES: &[&str] = &[
    "Bash", "Read", "Write", "Edit", "Glob", "Grep", "execute_command",
];

/// Register all local CLI tools and return their definitions (with prompts).
pub fn register_all(
    registry: &ExternalToolRegistry,
    _agent_id: &str,
    workspace_root: &Path,
) -> Vec<ToolDefinition> {
    // Register under "*" so handlers are available to ALL agents,
    // not just the initially-launched one.
    bash::register(registry, "*", workspace_root);
    read::register(registry, "*", workspace_root);
    write::register(registry, "*", workspace_root);
    edit::register(registry, "*", workspace_root);
    glob::register(registry, "*", workspace_root);
    grep::register(registry, "*", workspace_root);
    register_execute_command(registry, "*", workspace_root);

    tool_definitions()
}
```

- [ ] **Step 2: Build to verify it compiles**

Run: `cargo check -p distri-cli`
Expected: compiles with no errors (possibly a warning about unused `_agent_id`)

- [ ] **Step 3: Update `validate_external_tools` to check wildcard**

The `validate_external_tools` function at line 280 calls `registry.has_tool(agent_id, name)`. Since we now register under `"*"`, and `has_tool` already checks the wildcard key (see `external_tools_runtime.rs:76`), this should work without changes. Verify by reading `has_tool`:

```rust
// In external_tools_runtime.rs — already handles wildcard:
pub fn has_tool(&self, agent: &str, tool_name: &str) -> bool {
    // ...
    guard.contains_key(&(agent.to_string(), tool_name.to_string()))
        || guard.contains_key(&("*".to_string(), tool_name.to_string()))
}
```

No code change needed — just verify this during testing.

- [ ] **Step 4: Commit**

```bash
git add distri-cli/src/tools/mod.rs
git commit -m "fix: register local CLI tools under wildcard agent

Local tools (Bash, Read, Write, etc.) were registered under the
initial agent name only. Other agents couldn't use them because
ExternalToolRegistry keyed handlers by (agent_id, tool_name).
Register under '*' so any agent can resolve them."
```

---

### Task 2: Include local tool names in `external_tool_names` for run command

**Files:**
- Modify: `distri-cli/src/main.rs:407-434`

- [ ] **Step 1: Import `LOCAL_TOOL_NAMES` and add to external set**

At the top of `main.rs`, the import already exists: `use tools::{register_all, register_approval_handler, validate_external_tools};`. Add `LOCAL_TOOL_NAMES`:

```rust
use tools::{register_all, register_approval_handler, validate_external_tools, LOCAL_TOOL_NAMES};
```

Then after the `external_tool_names` extraction loop (after line 425, before line 427), add:

```rust
            // Always include locally-registered CLI tools so the stream
            // client intercepts them regardless of agent definition.
            for name in LOCAL_TOOL_NAMES {
                external_tool_names.insert(name.to_string());
            }
```

- [ ] **Step 2: Build to verify**

Run: `cargo check -p distri-cli`
Expected: compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add distri-cli/src/main.rs
git commit -m "fix: always include local tool names in external set (run command)

When running non-distri_runner agents, the external_tool_names set
was populated only from the agent definition. If the agent didn't
list Read/Write/etc in its external config, the CLI ignored those
tool calls, causing 120s server timeouts."
```

---

### Task 3: Include local tool names in `external_tool_names` for chat mode

**Files:**
- Modify: `distri-cli/src/chat.rs:478-489, 574-607`

- [ ] **Step 1: Import `LOCAL_TOOL_NAMES`**

Update the import at the top of `chat.rs`:

```rust
use crate::tools::{register_all, register_approval_handler, validate_external_tools, LOCAL_TOOL_NAMES};
```

- [ ] **Step 2: Fix initial chat setup (lines ~482-489)**

After `register_all` and before creating `stream_client`, change the chat setup to register under wildcard and set external tool names:

The `register_all` call at line 482 already passes `&current_agent`, but since Task 1 changed it to use `"*"` internally, this is fine. We need to set `external_tool_names` on the stream client. Currently (line 487-489) it doesn't call `with_external_tool_names`. Add it:

```rust
    let tool_defs = register_all(&registry, &current_agent, &workspace_path);
    app.add_tool_definitions(tool_defs);

    let stream_config = config.clone().with_timeout(60);
    let http_client = stream_config.build_http_client()?;
    // Seed external tool names with locally-registered tools
    let initial_external: std::collections::HashSet<String> =
        LOCAL_TOOL_NAMES.iter().map(|s| s.to_string()).collect();
    let mut stream_client = AgentStreamClient::from_config(config.clone())
        .with_http_client(http_client)
        .with_tool_registry(registry)
        .with_external_tool_names(initial_external);
```

- [ ] **Step 3: Fix per-message agent resolution (lines ~582-607)**

In the per-message loop where `external_tool_names` is rebuilt for each message, add the local tool names after extracting from the agent definition. After the `ext_names` loop (around line 590) and before the `validate_external_tools` call:

```rust
                    if let Some(tools) = &def.tools {
                        if let Some(ext) = &tools.external {
                            for name in ext {
                                if name != "*" {
                                    ext_names.insert(name.clone());
                                }
                            }
                        }
                    }
                }
                // Always include locally-registered CLI tools
                for name in LOCAL_TOOL_NAMES {
                    ext_names.insert(name.to_string());
                }
```

- [ ] **Step 4: Build to verify**

Run: `cargo check -p distri-cli`
Expected: compiles with no errors

- [ ] **Step 5: Commit**

```bash
git add distri-cli/src/chat.rs
git commit -m "fix: always include local tool names in external set (chat mode)

Same fix as the run command — chat mode also needs local tool names
in external_tool_names so the stream client intercepts them for any
agent, not just those that explicitly list them."
```

---

### Task 4: Add registry fallback in `is_external_tool`

**Files:**
- Modify: `distri/src/client_stream.rs:152-155`

- [ ] **Step 1: Update `is_external_tool` to check registry as fallback**

```rust
    /// Returns true if this tool name is declared external (client-handled).
    fn is_external_tool(&self, tool_name: &str) -> bool {
        self.external_tool_names.contains(tool_name)
            || self.tool_registry.as_ref()
                .map_or(false, |r| r.has_tool("*", tool_name))
    }
```

This is defense-in-depth: even if `external_tool_names` somehow misses a tool, the client will still intercept it if a handler is registered.

- [ ] **Step 2: Build to verify**

Run: `cargo check -p distri`
Expected: compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add distri/src/client_stream.rs
git commit -m "fix: is_external_tool falls back to registry check

Defense-in-depth: if external_tool_names doesn't contain a tool name,
also check if there's a handler registered in the tool registry.
Prevents silent timeouts if the name sets get out of sync."
```

---

### Task 5: Verify end-to-end

- [ ] **Step 1: Full workspace build**

Run: `cargo build`
Expected: builds cleanly

- [ ] **Step 2: Run tests**

Run: `cargo test -p distri-cli && cargo test -p distri`
Expected: all tests pass

- [ ] **Step 3: Manual verification**

Run the CLI with a non-distri_runner agent that has external tools and verify that Read/Write/Edit etc. no longer timeout. For example:

```bash
distri run --agent zippy "read the file ./Cargo.toml"
```

Expected: the agent successfully reads the file instead of timing out after 120s.
