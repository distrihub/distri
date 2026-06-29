# Sub-agent dispatch — end-to-end test report

Run: 2026-05-10. Branch: `fix/invocation`. Server: local watchexec build, postgres+redis. Default model: workspace-set `qwen3.6-plus` (Alibaba DashScope).

## TL;DR

`invoke_agent` is now the single LLM-facing dispatch tool. Its surface is **two shapes** with no `join`:

```jsonc
// single dispatch — common case
{ "agent": { "type": "named", "agent_id": "..." }, "message": { ... } }

// fan-out — N targets in parallel
{ "targets": [ { "agent": ..., "message": ... }, ... ] }
```

The orchestrator infers `Join::Single` (1 target) or `Join::All` (N targets). **Dispatch is always synchronous** — control returns once every target finishes. Detached/fire-and-forget is a CLIENT concern: the CLI/TUI/SDK can disconnect from the SSE stream and reconnect via the A2A `tasks/get` + `tasks/resubscribe` primitives. Agents never emit a "spawn-and-walk-away" tool call.

## Variants tested

| # | Agent | Shape | Result | Time | Tokens | Cost |
|---|---|---|---|---|---|---|
| A | `single_invoke_test` | `{agent, message}` shorthand → 1 Named worker | ✅ | 10.6s | ~3.3k | $0.0016 |
| B | `fanout_named_test_agent` | `{targets: [...]}` × 3 Named workers | ✅ | 12.7s | ~5.4k | $0.0013 |
| C | `fanout_test_agent` | `{targets: [...]}` × 3 AdHoc workers (inline body) | ✅ | 19.6s | ~7.5k | $0.0022 |
| D | `tool_search_mock_agent` | 30 deferred mock tools, discovered via `tool_search` | ✅ | ~9s | ~14k | $0.0046 |
| E | `fanout_image_test_agent` | `{targets: [...]}` × 3 AdHoc image workers | ✅ | ~3:51 | ~120k | $0.0044 |
| baseline | `image_test_agent` | single agent, no dispatch | ✅ | 9.1s | 7.7k | $0.0013 |

All five variants pass. Variant D was rewritten this session — the previous version (`fanout_tool_search_agent`) was a bad test premise; the replacement uses a proper deferred-tool-discovery scenario with mock tools.

## Variant A — single dispatch with shorthand

```bash
distri run --agent single_invoke_test --task "go"
```

LLM emitted exactly:
```json
{
  "agent": { "agent_id": "fanout_worker_agent", "type": "named" },
  "message": { "parts": [...], "role": "user" }
}
```

Orchestrator built `Invocation { targets: [_], join: Single, ... }`, dispatched the worker, returned `InvocationResult::Scalar`. Tool result body:
```json
{"kind":"scalar","result":{"content":"ok-99","status":"completed","task_id":"…"}}
```

Parent finalled with `"ok-99"`. **Two-turn flow, ~1.2k input tokens for the parent's last turn.** Cleanest possible dispatch.

## Variant B — fan-out to 3 Named workers

```bash
distri run --agent fanout_named_test_agent --task "ids: 1, 2, 3"
```

LLM emitted `{ context: "independent", targets: [3 Named entries] }` — no `join`. Orchestrator inferred `Join::All`, spawned 3 worker tasks in parallel, awaited all 3, returned `InvocationResult::Vector` with results in input order. Each worker wrote `/tmp/fanout-{id}.txt` and returned `ok-{id}`.

Parent's tool result included:
```json
{"kind":"vector","results":[
  {"content":"ok-1","status":"completed","task_id":"…"},
  {"content":"ok-2","status":"completed","task_id":"…"},
  {"content":"ok-3","status":"completed","task_id":"…"}
]}
```

**Total parent tokens: ~1.6k in + 51 out for the final turn. Cheapest fan-out path.**

## Variant C — fan-out with AdHoc inline-bodied workers

```bash
distri run --agent fanout_test_agent --task "ids: 1, 2, 3"
```

Same shape as Variant B but each target is `{ type: "ad_hoc", system_prompt: "...full worker behavior...", tools: { builtin: ["final"], external: ["Write"] } }`. The `tools` field is a per-target override that scopes the worker to exactly `final` + `Write` so qwen can't wander off into Bash/Read.

Parent total: ~2.3k in + 53 out. **+700 tokens vs. Variant B** because the parent re-emits the inline system_prompt for every target — that's the cost of self-contained workers (no separate skill/agent file to maintain).

## Variant D — deferred-loading + tool_search discovery

```bash
distri run --agent tool_search_mock_agent --task "What is the current weather in Tokyo?"
distri run --agent tool_search_mock_agent --task "Convert 100 USD to EUR"
distri run --agent tool_search_mock_agent --task "Open a GitHub issue on anthropics/claude-code titled 'CLI bug' with body 'tool_search returns empty'"
distri run --agent tool_search_mock_agent --task "Translate 'hello world' to Japanese"
distri run --agent tool_search_mock_agent --task "Look up the latest stock price for NVDA"
```

Agent has 30 mock dynamic tools spanning weather / stock / currency / translate / geocode / email / SMS / calendar / DB / knowledge-base / GitHub / Slack / Stripe / S3 / DNS / Wikipedia / math / UUID, plus `final` + `tool_search`. `tool_delivery_mode = "tool_search"` forces deferred loading: only `final` and `tool_search` get full schemas in the prompt; the other 30 are listed by name + description only. Each mock tool returns a canned realistic response on every call regardless of input.

Across all 5 task variants the agent followed the same pattern (~4 LLM turns, ~14k total prompt tokens):
1. `tool_search({query: "<keywords>"})` returns matching deferred tools (e.g. `tool_search("translate text") → 3 tools`).
2. Sometimes a second exploratory search `tool_search("?")` (qwen quirk; harmless).
3. Direct call to the discovered tool with correct parameters guessed from the description (e.g. `github_create_issue({owner, repo, title, body})`).
4. `final({result: "..."})` summarising the canned response.

**What this proves:**
- The deferred-tool listing block in the system prompt successfully advertises 30 tool names + descriptions while keeping their full schemas out of the prompt.
- `tool_search` keyword search returns the right deferred entries (verified via `distri traces export` — the full `{tools_found: N, tools: [...]}` payload reaches the LLM correctly; the CLI was previously misrendering this as "Found 0 tools" due to a `Part::Text`-vs-`Part::Data` formatter bug, now fixed in `distri-formatter/src/renderers/platform.rs`).
- The `mock` dynamic-tool factory + scenario registry materialise as real callable tools that route through the same `dynamic_factory.rs` path as production HTTP factories.

**One observation worth flagging:** qwen3.6-plus tends to *skip* the `tool_search({names: [...]})` exact-name-lookup step (which returns the full parameter schema). It guesses parameter names from the description in the deferred listing instead. For the mock scenarios with descriptive parameter names (`city`, `ticker`, `owner`/`repo`), guessing works. For real APIs with cryptic params, agents should be steered toward the exact-name step — either by tightening the system prompt or by making the deferred listing omit a hint that descriptions are sufficient.

**Variant D infrastructure (this session):**
- `distri-types/src/mock_tool.rs` — typed `MockFactoryConfig` (scenario id + optional inline overrides).
- `distri-core/src/tools/mock_tool.rs` — 30-entry `SCENARIO_REGISTRY`; `build_mock_tool()` resolves config → `MockTool`; `MockTool` impls `Tool` + `ExecutorContextTool` and returns the canned response on every call.
- `distri-core/src/tools/dynamic_factory.rs` — wires `factory_type = "mock"` into `create_dynamic_tool()` + `validate_dynamic_tool()`.
- `distri-core/src/tests/mock_tool.rs` — 5 unit tests pinning scenario lookup, custom override, unknown-scenario stub, and malformed-config rejection.
- `distri-formatter/src/renderers/platform.rs` — `tool_search` formatter now parses `Part::Text(json_string)` for `tools_found`.

## Variant E — image fan-out

```bash
distri run --agent fanout_image_test_agent \
  --task "Identify the people in: docs/testing/execution/test_image.png, \
                                  docs/testing/execution/test_image_2.png, \
                                  docs/testing/execution/test_image_3.png"
```

3 AdHoc workers in parallel; each calls `load_skill("detect_image_person")` → `Read(<path>)` (returns `Part::Image`) → `final(<name>)`.

Result: `"Donald Trump, Narendra Modi, Person detected (unidentified)"`. The vision pipeline (tool_result image → next-turn `image_url` content part) works through the dispatch boundary — workers see the image bytes their parent never touched.

The third worker failed to identify (qwen vision quality, not a dispatch issue). One worker also went exploratory with Bash/python3 skin-tone analysis before emitting `final` — the AdHoc system_prompt for this variant was less constrained than Variant C's, allowing wildcard external tools. **For predictable behavior, scope `tools.external` as Variant C does.**

## Implementation summary (this branch)

What changed from the previous Invocation-axes design:

### `InvokeAgentTool` schema — no `join`

```rust
fn get_parameters(&self) -> Value {
    json!({
        "type": "object",
        "properties": {
            "agent":   { /* AgentRef */ },        // single-dispatch shorthand
            "message": { /* Message */ },         // single-dispatch shorthand
            "targets": { /* Vec<Target> */ },     // fan-out form
            "context": { "enum": ["independent", "inherited", "shared"] },
            "executor": { /* ExecutorHint */ },
            "tools":    { /* ToolPolicy */ }
        }
        // no `required` — either {agent,message} OR {targets} is accepted
    })
}
```

`InvokeAgentInput::into_invocation()` validates: must be exactly one of the two shapes (mixing both errors), targets non-empty, etc. `Join` is inferred:
- 1 target → `Join::Single` → returns `InvocationResult::Scalar`
- N targets → `Join::All` → returns `InvocationResult::Vector` (input order)

### `Join::Detached` — kept in the typed model, removed from LLM surface

The `Invocation { join: Join::Detached }` constructor still exists for programmatic callers (the CLI's eventual `--detached` flag, the TUI's "watch later" UX). The LLM tool simply never builds it — the LLM's tool-call → tool-result loop is fundamentally synchronous, so a "fire-and-forget" surface there confuses more than it helps.

### Supervisor tools — opt-in

`get_task` / `wait_task` / `cancel_task` / `list_my_tasks` live in `get_builtin_tools()` so an agent that explicitly declares `tools.builtin = ["wait_task", ...]` resolves them by name. They are **NOT** in any auto-included default. A leaf agent on the sync invoke path never has children to wait on, so it doesn't see them. CLI / TUI / SDK call those primitives through the API directly (planned: `distri tasks list/watch/cancel`), not via an agent's tool surface.

### Files touched

| File | Change |
|---|---|
| `distri/server/distri-core/src/tools/invoke_agent.rs` | New `InvokeAgentInput` parser; doc-string rewrite; schema drops `join` |
| `distri/server/distri-core/src/tools/builtin.rs` | Doc clarifying supervisor tools are opt-in via name lookup |
| `distri/server/distri-core/src/tests/invoke_agent_tool.rs` | Replaced detached-routing test with single-shorthand + fan-out + mixed-input + empty-input persistence tests |
| `distri/server/distri-core/src/tests/universal_agent_access.rs` | `call_agent` → `invoke_agent` |
| `distri/server/agents/distri.md` | Tool description + delegation example use the shorthand |
| `distri/docs/testing/execution/tests/agents/single_invoke_test.md` | Shorthand `{agent, message}` |
| `distri/docs/testing/execution/tests/agents/fanout_*.md` | Drop `join` field; add a "dispatch is sync" note |
| `distri/docs/testing/execution/tests/agents/fanout_detached_supervisor_agent.md` | **Deleted** — LLM-facing detached is gone |
| `distri/docs/testing/execution/tests/agents/fanout_tool_search_agent.md` | **Deleted** — replaced by `tool_search_mock_agent.md` |
| `distri/distri-types/src/mock_tool.rs` | New typed `MockFactoryConfig` |
| `distri/distri-types/src/lib.rs` | Register `mock_tool` module |
| `distri/server/distri-core/src/tools/mock_tool.rs` | New `MockTool` + 30-scenario registry + `build_mock_tool()` |
| `distri/server/distri-core/src/tools/mod.rs` | Register `mock_tool` module |
| `distri/server/distri-core/src/tools/dynamic_factory.rs` | Wire `factory_type = "mock"` into create + validate |
| `distri/server/distri-core/src/tests/mock_tool.rs` | 5 unit tests for the mock factory |
| `distri/server/distri-core/src/tests/mod.rs` | Register `mock_tool` test module |
| `distri/distri-formatter/src/renderers/platform.rs` | `tool_search` formatter parses `Part::Text(json)` for the count |
| `distri/docs/testing/execution/tests/agents/tool_search_mock_agent.md` | **New** — proper deferred-loading test using 30 mock tools |

Test results: `cargo test -p distri-core --lib` → **329 passed / 0 failed / 1 ignored** (+5 from MockTool unit tests). All five live variants pass against the local server.

## Architectural decisions (final)

### Q1: ContextScope — keep it.
Three values (`independent` / `inherited` / `shared`) cover three real shapes (fresh, copy-history, shared-thread). Today only `independent` is wired; the other two are placeholders for the planning skill's eventual sub-task scope rules. Dropping the field would have to be re-added later under a worse name.

### Q2: `Join::All` vs N parallel `Join::Single` tool-calls — keep `Join::All`.
qwen and weaker models do not reliably emit N parallel `tool_calls` in a single assistant turn. Forcing fan-out into `targets[]` lets us guarantee parallelism on any provider, and the orchestrator's tokio::spawn loop is already dialed in for it. The downside (LLM has to learn two shapes) is paid once in the system prompt; the parallel-tool-calls path would require N independent assistant turns to merge results, doubling the round-trips on slow providers.

### Q3: `wait` / detached — caller's choice via metadata.
The orchestrator-side dispatch is unconditionally sync. The CLIENT picks whether to follow the SSE stream:

- **Default (sync):** CLI's `distri run` follows `message/stream` to terminal, prints the final result.
- **Detached (planned):** `distri run --detached` calls `message/send` (returns `task_id`), then exits. User can later `distri tasks watch <task_id>` / `distri tasks list` / `distri tasks cancel <task_id>`.
- **TUI (planned):** ambient "running tasks" panel built on the same A2A endpoints. Detach/reattach is a UX affordance, not a protocol mode.

The LLM tool surface stays `{agent, message}` (or `{targets: [...]}`) only. No `wait`, no `join`. The agent's job is to ask for work; deciding whether the human watches is one layer up.

## Inspecting traces

```bash
distri traces list --limit 10
distri traces show <trace-id>
distri traces export <trace-id> -o /tmp/run.json
```

Run trace IDs (this session — your local trace IDs will differ):
- Variant A `single_invoke_test`: 18 spans, 10.6s
- Variant B `fanout_named_test_agent`: 9 spans (top-level), 3 × 9-span worker traces, 12.7s wall
- Variant C `fanout_test_agent`: 9 spans, 19.6s — three `_adhoc_base` worker traces hanging off it
- Variant D `fanout_tool_search_agent`: 41 spans, 57.7s — bloated by the search-thrash loop
- Variant E `fanout_image_test_agent`: 9 spans top-level, 3 × `_adhoc_base` worker traces (~3 min each due to image-prompt size)

DB inspection — child rows live in `tasks` with `parent_task_id` set and `invocation` blob carrying the typed Invocation:

```sql
SELECT id, parent_task_id, status, invocation->>'join' as join_kind
FROM tasks
WHERE parent_task_id = '<parent_task_id>'
ORDER BY created_at;
```

`invocation->>'join'` is always `"single"` or `"all"` post this branch — `"detached"` only appears for tasks created via the (future) CLI `--detached` flag, never via an LLM-emitted tool call.

## Known follow-ups (NOT in this branch)

1. **CLI `--detached` + `distri tasks watch/list/cancel`:** consume the existing A2A primitives. ~1 day of work.
2. **TUI watch view:** ambient running-tasks panel in `distri-tui`. Out of scope for this report.
3. **distrijs `useChat({mode: 'detached'})`:** abort the SSE stream early on the client; server-side task continues. Same A2A endpoints as the CLI path.
4. **System-prompt nudge for tool_search exact-name lookup:** qwen tends to skip the schema-load step when descriptions are clear. For real APIs with cryptic params this would fail — consider tightening the prompt or adding a stronger hint that deferred tools must be loaded before calling.
5. **More mock scenarios:** the registry has 30 entries; expand to 50+ if a future test wants to stress prompt-cache behavior under heavier deferred-tool listings.
