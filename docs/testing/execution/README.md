# Tool-Result Image Flow — End-to-End Test

This is the smoke test for the **tool-result → multimodal LLM** pipeline.
It exercises the chain that broke in the grading-import flow: a tool
returns `[Part::Data, Part::Image]`, the next planning turn must include
that image as a real `image_url` content part on the chat completion so
the model can see it.

If this test passes (the model correctly identifies the person in
`test_image.png`), the pipeline is wired correctly end-to-end:

- The CLI's `Read` tool sniffs the file extension and returns the image
  as `Part::Image(FileType::Bytes{…})` alongside a small `Part::Data`
  descriptor.
- `agent/strategy/execution/default.rs::handle_tools_action` does NOT
  duplicate-paste tool-result parts onto the assistant message. (Pre-fix
  bug: those loose copies got dropped by the LLM client because
  `llm.rs::MessageRole::Assistant` has no `Part::Image` branch.)
- `compact_for_storage` keeps inline images in the scratchpad so the
  next planning turn can read them. (Pre-fix bug: storage stripped
  images; the latest-entry carve-out in the formatter was dead code.)
- `compact_for_history` strips images **only** for non-latest entries
  during prompt construction.
- `llm.rs:1290-1304` extracts `Part::Image` from inside the tool message
  and ships it on a follow-up user message with `[Tool result images:]`.

Doc cross-references: see
[`docs/execution/modes.md`](../../execution/modes.md)
for the four `CallMode` semantics, and
[`docs/execution/scratchpad.md`](../../execution/scratchpad.md)
for the storage vs display compaction rules.

---

## Files

> **Fixtures moved.** As of 2026-05-10 the agent + skill markdowns and
> the `test_image*.png` files live in
> [`integration-tests/fixtures/`](../../../../integration-tests/fixtures/)
> next to the Rust regression suite. The shell-driven flow below still
> works — just point `distri push` at the new directory.

| Path | Role |
|---|---|
| `integration-tests/fixtures/test_image.png` | Test fixture. A photo of Donald Trump (filename intentionally generic — must not leak the answer to the model). |
| `integration-tests/fixtures/agents/image_test_agent.md` | Test agent. Has `final`, `load_skill` (builtin) + `Read` (CLI external tool). References `detect_image_person` skill via `available_skills`. |
| `integration-tests/fixtures/skills/detect_image_person.md` | Leaf-worker skill. Body tells the agent to call `Read` on the path, look at the returned image, and `final({"result": "<name>"})`. |
| `distri-cli/src/tools/read.rs` | CLI `Read` tool. Auto-detects image extensions and returns `Part::Image` instead of trying to UTF-8-decode bytes as text. |

---

## One-time setup

1. **A running `distri-server`** on `localhost:1341` with the latest
   binary (it must include the `default.rs::handle_tools_action`
   duplication-removal fix and the `compact_for_storage` keeps-images
   fix). Rebuild + restart if the binary on disk is newer than the
   running process:

   ```sh
   cargo build -p distri-server
   # then restart however you launch it; e.g. with tee'd logs:
   #   target/debug/distri-server 2>&1 | tee /tmp/distri-server.log
   ```

2. **A fresh CLI binary** with the image-aware `Read` tool:

   ```sh
   cargo install --bin distri --path distri-cli --force --locked --debug
   ```

3. **`.env` sourced** so the CLI talks to the right workspace and has
   provider credentials. Without it, `distri traces show` silently
   resolves to the wrong workspace and reports "no spans found":

   ```sh
   set -a && source .env && set +a
   ```

   Required vars: `DISTRI_API_KEY`, `DISTRI_BASE_URL`,
   `DISTRI_WORKSPACE_ID` (the one your server is running with).

4. **Agent + skill pushed once** to the server:

   ```sh
   distri push integration-tests/fixtures
   ```

   Subsequent edits to either file → re-run the same command. The same
   files are pushed automatically by the Rust regression suite via
   `integration_tests::fixtures::{push_agent, push_skill}` — see
   [`integration-tests/README.md`](../../../../integration-tests/README.md).

---

## Running the test

```sh
distri run --agent image_test_agent \
  --task "Identify the person in integration-tests/fixtures/test_image.png"
```

The agent extracts the path from the user task and calls `Read` on it.
Relative paths resolve against the workspace root.

### Expected output

```
Streaming agent 'image_test_agent' via http://localhost:1341/v1
◆ image_test_agent:
  ⎿  # Detect image person                  ← load_skill returned the skill body
⏺ Read("/.../docs/testing/execution/test_image.png")
  ⎿  Read 0 lines                            ← Read returned the image (token count is misleading; image was attached as vision part)
◆ image_test_agent:
⚠ Context critical: 933% used (186K/20.0K)   ← see "known noise" below
Donald Trump
```

The final line **must say "Donald Trump"** (or include it). If it says
anything else, the model didn't actually see the image — see
"Diagnosing failures" below.

---

## Inspecting the trace

```sh
distri traces list --limit 3
distri traces show <trace-id>      # gantt view of all spans
distri traces export <trace-id> -o /tmp/run.json    # full LLM call dump
```

What you should see in the export for the **second LLM call** (the one
that produced "Donald Trump"):

- `msg<assistant>` parts = `[ToolCall(Read), …]`. **No loose
  `Part::Image`** on the assistant message. (If you see one, the
  duplication fix at `default.rs:107` regressed.)
- `msg<tool>` for the `Read` tool_result has `inner_parts` including
  `image` — actually after the otel exporter's 500 KB span-payload cap
  (`hooks/otel.rs:282`) you'll see the image truncated or replaced with
  a placeholder string in the *trace UI*, but the actual API call
  carried the full image. The assertion is always:
  **the model said "Donald Trump"** — that's the only proof that
  matters.

If `distri traces show <id>` returns "No spans found" but the run
clearly succeeded:

- You forgot to `source .env`. The CLI fell back to your default
  profile's workspace, which doesn't own those spans. `set -a && source
  .env && set +a` and retry.

---

## Diagnosing failures

| Symptom | Likely cause | Where to look |
|---|---|---|
| Final result is generic ("a man in a suit", "I cannot identify…") | Image didn't reach the model. | `default.rs:107` duplication is back, OR `compact_for_storage` is stripping images. Re-grep both call sites; rebuild + restart `distri-server`. |
| Trace export shows `inner_parts=['data']` and the model fails | Image stripped from the tool_result before the LLM call. | Check the running binary's `compact_for_storage` keeps `Part::Image`. (Test in `distri-types/src/tests/tool_result_storage_tests.rs::compact_for_storage_keeps_inline_image` pins this.) |
| `distri traces show <id>` returns "No spans found" | Wrong workspace context. | `set -a && source .env && set +a` then retry. |
| `distri push` succeeds but the server doesn't see the new agent | The CLI hit a different workspace than the server. | Check `DISTRI_WORKSPACE_ID` matches what the server logs say it loaded the agent under (`Found agent: … workspace=…`). |
| LLM error: invalid_request_error / DeploymentNotFound | Workspace's default model isn't deployed for your provider account. | `distri agents list` to see the resolved provider/model; either change the workspace default model or add the deployment in your provider console. |
| `Context critical: 933% used (186K/20.0K)` warning | The agent's `context_size` budget is too small for the image's base64 size. The call still succeeds because the actual provider context window is much larger; this is a soft warning. | If it bothers you, raise `context_size` on `image_test_agent.md` or accept the noise — the test still passes. |

---

## Known noise (not blocking)

- **`ALIBABA_CLOUD_API_KEY secret not found for provider 'alibaba_cloud'`** in the server log: that's a warning logged when the
  `distri` (system) agent's provider resolution runs without the
  Alibaba secret in your local DB. Unrelated to this test.
- **`Read 0 lines`** in the CLI output for image files: the CLI's
  pretty-print line-counts the descriptor part, which has 0 lines
  (it's a Data part, not text). The image itself goes through as a
  separate Image part. Cosmetic.
- **`Context critical: …%`**: see the table above.

---

## What this test does NOT cover

- Parallel fan-out via `mode: "fork"`. That's the importer / grader use
  case, tested separately by re-running the grading-import flow against
  `zippy_browser`.
- Tool-result image flow when the image comes from `db_get` (browser
  IDB) instead of `Read` (filesystem). Same code path on the server
  side; covered conceptually by passing this test, but the browser-side
  version is exercised only by the live editor-ui flow.
- LLM-side OCR accuracy. We assert "the model can see the image" via a
  recognisable face. Text extraction quality is a separate concern.

---

## Note on paths

All shell snippets are relative to the repo root. Don't substitute
absolute paths.



## Sample output 
```bash
distri run --agent image_test_agent  --task "who is the perosn in docs/testing/execution/test_image.png"
Streaming agent 'image_test_agent' via http://localhost:1341/v1
◆ image_test_agent:
  ⎿  # Detect image person
  ↳ 1.2K in, 110 out
◆ image_test_agent:
⏺ Read("docs/testing/execution/test_image.png")
  ⎿  Read 0 lines
  ↳ 1.6K in, 87 out
◆ image_test_agent:
  ↳ 2.0K in, 71 out
⚠ Context critical: 933% used (186K/20.0K)

Donald Trump

$ distri traces show --latest

  Trace: execute image_test_agent (cef11446b97efa18)
  11 spans · 7.3s · 6.7ktokens · $0.0014
  ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

  [Agent] image_test_agent  f51ac174bfce9543    3.3k↑
    → Identify the person in docs/testing/execution/test_image.p...
    ← Donald Trump
          ████████████████████████████████████████████████████████████████████████████████ 7.3s
    [Step] Step 0  8f567e2dcd4a3c37
           █░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 5ms
      [Plan] Planning (initial)  2affe6bc0535ef51
             ███████████████████████████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 3.6s
    [LLM] qwen3.6-plus  9e157f0cc23cb758    1.3k↑  $0.000600
      → Identify the person in docs/testing/execution/test_image.p...
      ← detect_image_person
          █░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 5ms
    [Tool] load_skill  d62e3b4c90caa33b
      ← # Detect image person
           ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░█░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 12ms
    [Step] Step 1  8bb03012813c1e28
           ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░██░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 208ms
    [Tool] Read  ca91f6c5dc077bd3
      ← docs/testing/execution/test_image.png
           ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░█░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 48ms
    [Step] Step 2  06582cc5ead4617b
           ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░█░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 5ms
      [Plan] Planning (initial)  1739ac0913f57e0f
             ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░████████████████████████████████████░░ 3.3s
    [LLM] qwen3.6-plus  a7ee0b0f95a4262e    2.1k↑  $0.000800
      → [{"id":"93226c92-631b-41c1-9869-59bc202b376a","name":null,"role":"system","parts":[{"part_type":"tex...
      ← Donald Trump
          ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░█░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 102ms
    [Tool] final  149171225a5b675a
           ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░█ 6ms
  ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

  INPUT:
  Identify the person in docs/testing/execution/test_image.png

  OUTPUT:
  Donald Trump
  ```
