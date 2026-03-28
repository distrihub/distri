# CLI Prettier Output — Claude Code Style

## Problem

The CLI output is noisy and hard to scan. Tool results dump too much JSON, platform tools show raw data, and there's no visual hierarchy. Compare with Claude Code's clean output:

```
⏺ Bash(mkdir -p /path/to/dir)
  ⎿  Done

⏺ Read(distri-cli/src/main.rs)
  ⎿  (250 lines)
```

## Design Goals

1. **Tree-style rendering**: `⏺` for tool call, `⎿` for indented result (like Claude Code)
2. **Tool-aware summaries**: Each tool type gets a one-liner summary, not raw output
3. **Truncate aggressively**: Show what matters, hide the rest
4. **Suppress noise**: No "completed in Xms", no "Tool calls for message X", no raw JSON

## Files to Modify

All changes in `distri/distri/src/`:
- `printer.rs` — EventPrinter methods (tool_start, tool_end, handle_tool_calls, handle_event)
- `renderers/mod.rs` — Dispatcher + new platform/skill renderers
- `renderers/tool_result.rs` — Generic fallback
- `renderers/browser.rs` — Browsr tool renderers
- `renderers/shell.rs` — Shell renderers
- New: `renderers/platform.rs` — Platform tool renderers

## Changes

### 1. Tool Call Start (`printer.rs::tool_start`)

**Before:**
```
⏺ load_skill ({"skill_name": "send_email", "workspace_id": "abc"})
```

**After — Tool-specific one-liners:**
```
⏺ load_skill("send_email")
⏺ run_skill_script("send_email", step=2)
⏺ list_skills(...)
⏺ list_agents(...)
⏺ create_skill("my_skill")
⏺ delete_skill("my_skill")
⏺ browsr_scrape("https://example.com")
⏺ browsr_browser(goto "https://example.com")
⏺ execute_shell("ls -la")
⏺ search("rust async patterns")
⏺ write_to_storage("config.json")
⏺ read_from_storage("config.json")
⏺ tool_search("email tools")
⏺ inject_connection_env("google", scopes=["gmail.send"])
⏺ final(...)
```

Implementation: Add a `fn format_tool_call(name: &str, input: &serde_json::Value) -> String` that pattern-matches on tool name and extracts the relevant field(s) for a compact display. Falls back to current `format_tool_input` for unknown tools.

```rust
fn format_tool_call(&self, name: &str, input: &serde_json::Value) -> String {
    match name {
        "load_skill" => {
            let skill = input.get("skill_name").and_then(|v| v.as_str()).unwrap_or("?");
            format!("load_skill(\"{}\")", skill)
        }
        "run_skill_script" => {
            let skill = input.get("skill_name").and_then(|v| v.as_str()).unwrap_or("?");
            let step = input.get("step_index").and_then(|v| v.as_u64());
            match step {
                Some(s) => format!("run_skill_script(\"{}\", step={})", skill, s),
                None => format!("run_skill_script(\"{}\")", skill),
            }
        }
        "create_skill" | "delete_skill" => {
            let skill = input.get("name").or(input.get("skill_name"))
                .and_then(|v| v.as_str()).unwrap_or("?");
            format!("{}(\"{}\")", name, skill)
        }
        "browsr_scrape" | "browsr_crawl" => {
            let url = input.get("url").and_then(|v| v.as_str()).unwrap_or("?");
            format!("{}(\"{}\")", name, truncate(url, 60))
        }
        "browsr_browser" | "browser_step" => {
            let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            let url = input.get("url").and_then(|v| v.as_str());
            match url {
                Some(u) => format!("{}({} \"{}\")", name, action, truncate(u, 50)),
                None => format!("{}({})", name, action),
            }
        }
        "execute_shell" => {
            let cmd = input.get("command").and_then(|v| v.as_str()).unwrap_or("?");
            format!("execute_shell(\"{}\")", truncate(cmd, 60))
        }
        "start_shell" => format!("start_shell(...)"),
        "stop_shell" => format!("stop_shell(...)"),
        "search" => {
            let q = input.get("query").and_then(|v| v.as_str()).unwrap_or("?");
            format!("search(\"{}\")", truncate(q, 60))
        }
        "write_to_storage" | "read_from_storage" => {
            let key = input.get("key").or(input.get("path"))
                .and_then(|v| v.as_str()).unwrap_or("?");
            format!("{}(\"{}\")", name, key)
        }
        "tool_search" => {
            let q = input.get("query").and_then(|v| v.as_str()).unwrap_or("?");
            format!("tool_search(\"{}\")", truncate(q, 60))
        }
        "inject_connection_env" => {
            let provider = input.get("provider_name").and_then(|v| v.as_str()).unwrap_or("?");
            format!("inject_connection_env(\"{}\")", provider)
        }
        "final" | "reflect" | "console_log" => format!("{}(...)", name),
        "transfer_to_agent" => {
            let to = input.get("agent_name").and_then(|v| v.as_str()).unwrap_or("?");
            format!("transfer_to_agent(\"{}\")", to)
        }
        _ => format!("{}({})", name, self.format_tool_input(input)),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max { format!("{}...", &s[..max]) } else { s.to_string() }
}
```

Update `tool_start` to use it:
```rust
fn tool_start(&mut self, tool_call_id: &str, name: &str, input: &serde_json::Value) {
    println!(
        "{}⏺ {}{}",
        COLOR_YELLOW,
        self.format_tool_call(name, input),
        COLOR_RESET
    );
    // ... rest unchanged
}
```

### 2. Tool Call End (`printer.rs::tool_end`)

**Before:**
```
load_skill completed in 45ms
```

**After:** Remove entirely. The result line (`⎿`) is sufficient. Delete the `tool_end` method body (keep the state tracking but remove the println).

```rust
fn tool_end(&mut self, tool_call_id: &str, success: bool) {
    if let Some(state) = self.state.tool_calls.get_mut(tool_call_id) {
        state.status = if success { ToolCallStatus::Completed } else { ToolCallStatus::Error };
        state.end_time = Some(Instant::now());
        // No output — result line handles display
    }
}
```

### 3. Tool Results — Indented `⎿` prefix (`renderers/`)

**Before:**
```
  {"skill_id": "abc", "name": "send_email", "steps": [...]}
```

**After:**
```
  ⎿  Loaded skill "send_email" (3 steps)
```

All renderers should prefix output with `  ⎿  ` (2 spaces + ⎿ + 2 spaces) instead of `  ` (2 spaces).

Add to `renderers/mod.rs`:
```rust
pub const RESULT_PREFIX: &str = "  ⎿  ";
```

### 4. New Platform Tool Renderer (`renderers/platform.rs`)

Create `renderers/platform.rs` with tool-specific summaries:

```rust
use crate::printer::{COLOR_GRAY, COLOR_GREEN, COLOR_RED, COLOR_RESET};
use crate::renderers::RESULT_PREFIX;
use distri_types::{Part, ToolResponse};

pub fn render_platform_tool(result: &ToolResponse) {
    let name = result.tool_name.as_str();
    let text = result.parts.iter().find_map(|p| {
        if let Part::Text(t) = p { Some(t.as_str()) } else { None }
    });
    let data = result.parts.iter().find_map(|p| {
        if let Part::Data(v) = p { Some(v) } else { None }
    });

    match name {
        "load_skill" => {
            if let Some(d) = data.or(text.and_then(|t| None)) {
                let skill_name = d.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let steps = d.get("steps").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
                println!("{}{}Loaded skill \"{}\" ({} steps){}", COLOR_GRAY, RESULT_PREFIX, skill_name, steps, COLOR_RESET);
            } else if let Some(t) = text {
                render_one_liner(t, result.is_error);
            }
        }
        "run_skill_script" => {
            if result.is_error {
                render_one_liner(text.unwrap_or("error"), true);
            } else {
                println!("{}{}Done{}", COLOR_GRAY, RESULT_PREFIX, COLOR_RESET);
            }
        }
        "list_skills" => {
            if let Some(d) = data {
                let count = d.as_array().map(|a| a.len()).unwrap_or(0);
                println!("{}{}Found {} skills{}", COLOR_GRAY, RESULT_PREFIX, count, COLOR_RESET);
            }
        }
        "list_agents" => {
            if let Some(d) = data {
                let count = d.as_array().map(|a| a.len()).unwrap_or(0);
                println!("{}{}Found {} agents{}", COLOR_GRAY, RESULT_PREFIX, count, COLOR_RESET);
            }
        }
        "create_skill" => {
            if let Some(d) = data {
                let skill_name = d.get("name").and_then(|v| v.as_str()).unwrap_or("skill");
                println!("{}{}Created \"{}\"{}", COLOR_GRAY, RESULT_PREFIX, skill_name, COLOR_RESET);
            } else {
                println!("{}{}Done{}", COLOR_GRAY, RESULT_PREFIX, COLOR_RESET);
            }
        }
        "delete_skill" => println!("{}{}Deleted{}", COLOR_GRAY, RESULT_PREFIX, COLOR_RESET),
        "write_to_storage" => println!("{}{}Saved{}", COLOR_GRAY, RESULT_PREFIX, COLOR_RESET),
        "read_from_storage" => {
            if let Some(t) = text {
                let lines = t.lines().count();
                println!("{}{}({} lines){}", COLOR_GRAY, RESULT_PREFIX, lines, COLOR_RESET);
            }
        }
        "tool_search" => {
            if let Some(d) = data {
                let count = d.as_array().map(|a| a.len()).unwrap_or(0);
                println!("{}{}Found {} tools{}", COLOR_GRAY, RESULT_PREFIX, count, COLOR_RESET);
            }
        }
        "inject_connection_env" => {
            if result.is_error {
                render_one_liner(text.unwrap_or("error"), true);
            } else {
                println!("{}{}Connected{}", COLOR_GREEN, RESULT_PREFIX, COLOR_RESET);
            }
        }
        _ => {
            // Unknown platform tool — show first line
            if let Some(t) = text {
                render_one_liner(t, result.is_error);
            } else {
                println!("{}{}Done{}", COLOR_GRAY, RESULT_PREFIX, COLOR_RESET);
            }
        }
    }
}

fn render_one_liner(text: &str, is_error: bool) {
    let first_line = text.lines().next().unwrap_or("");
    let truncated = if first_line.len() > 100 {
        format!("{}...", &first_line[..100])
    } else {
        first_line.to_string()
    };
    let color = if is_error { COLOR_RED } else { COLOR_GRAY };
    println!("{}{}{}{}", color, RESULT_PREFIX, truncated, COLOR_RESET);
}
```

### 5. Update Dispatch (`renderers/mod.rs`)

```rust
mod platform;

match name {
    // Simple tools — suppress output entirely
    "final" | "reflect" | "console_log" => {}

    // Transfer — no result to show
    "transfer_to_agent" => {}

    // Platform tools
    "tool_search" | "load_skill" | "run_skill_script" | "list_agents" | "list_skills"
    | "create_skill" | "delete_skill" | "write_to_storage" | "read_from_storage"
    | "inject_connection_env" => {
        platform::render_platform_tool(result);
    }

    // Browser / scraping
    "browsr_scrape" | "browsr_crawl" => browser::render_scrape(result),
    "browsr_browser" | "browser_step" => browser::render_browser_step(result),

    // ... rest unchanged but update all to use RESULT_PREFIX
}
```

### 6. Update Existing Renderers to Use `⎿` Prefix

Update all `"  "` prefixes to use `RESULT_PREFIX` in:
- `renderers/shell.rs`
- `renderers/browser.rs`
- `renderers/search.rs`
- `renderers/code.rs`
- `renderers/tool_result.rs`

### 7. Suppress `handle_tool_calls` Noise

**Before:**
```
Tool calls for message abc123
• load_skill ({"skill_name": "send_email"})
• execute_shell ({"command": "ls"})
```

**After:** Remove entirely — the individual `⏺` lines already show this.

```rust
fn handle_tool_calls(&mut self, _tool_calls: &[distri_types::ToolCall], _parent: Option<&str>) {
    // Suppressed — individual ToolExecutionStart events show each call
}
```

### 8. Suppress Step Start/End Noise

**Before:**
```
→ Starting step 1
✔ Step 1 (Step 1) [2345ms]
```

**After:** Remove step start. Keep step completion only on error.

```rust
AgentEventType::StepStarted { .. } => {
    // Suppressed — plan steps are internal
}
AgentEventType::StepCompleted { step_id, success } => {
    if !success {
        if let Some(step) = self.state.steps.get(step_id) {
            println!("{}✖ Step {} failed{}", COLOR_RED, step.index + 1, COLOR_RESET);
        }
    }
}
```

### 9. Suppress Thread/Task Header

**Before:**
```
thread: abc123  task: def456
```

**After:** Remove — internal IDs aren't useful to the user.

```rust
// Remove the printed_header block at the top of handle_event
```

### 10. Clean Up RunFinished

**Before:**
```
14:32:01 [distri] run finished (3 steps, ok)
```

**After:** Only show on error.

```rust
AgentEventType::RunFinished { success, .. } => {
    // Only show if there's something to report
    if !success {
        println!("{}Run completed with errors{}", COLOR_RED, COLOR_RESET);
    }
}
```

## Example: Before vs After

**Before:**
```
thread: e6773d01  task: 9c38bd52
🧠 Planning…
→ Starting step 1
⏺ load_skill ({"skill_name": "send_email", "workspace_id": "855a2f70"})
load_skill completed in 45ms
  {"id": "abc", "name": "send_email", "description": "Send emails", "steps": [{"index": 0, ...}, {"index": 1, ...}]}
⏺ run_skill_script ({"skill_name": "send_email", "step_index": 0, "input": {"to": "user@example.com", "subject": "Hello"}})
run_skill_script completed in 1200ms
  {"success": true, "output": "Email sent successfully to user@example.com"}
Tool calls for message msg123
• final ({"message": "Done"})
⏺ final ({"message": "Done"})
final completed in 2ms
  Done
✔ Step 1 (Step 1) [1300ms]
14:32:01 [distri] run finished (1 steps, ok)
```

**After:**
```
🧠 Planning...
⏺ load_skill("send_email")
  ⎿  Loaded skill "send_email" (2 steps)
⏺ run_skill_script("send_email", step=0)
  ⎿  Done
⏺ final(...)

assistant: Done
```

## Testing

After changes, run `cargo check -p distri` and test manually with:
```bash
cargo run --bin distri -- tui distri
```

Verify with various tool types: skill loading, shell execution, browsr scraping, search, agent transfer.
