# cancel_tree fixture

Cancel a run tree from the distri TUI end to end, with no model key
(distri#154, #155). `stub_llm.py` is a scripted OpenAI-compatible endpoint:

- `cancel_parent` calls `invoke_agent` on `cancel_slow` (in the background
  when the message says "background"), then `final`.
- `cancel_slow`'s model answers `final` only after `SLOW_SECS` (default 300),
  so the child stays working long enough to cancel.

## Run it

```bash
cargo build -p distri-server-cli -p distri-cli   # from the repo root
cp -R testing/fixtures/cancel_tree /tmp/cancel_tree && cd /tmp/cancel_tree
SLOW_SECS=40 python3 -I stub_llm.py 18931 &
CURRENT_WORKING_DIR=$PWD <repo>/target/debug/distri-server &       # port 18930
<repo>/target/debug/distri --base-url http://127.0.0.1:18930/v1 tui cancel_parent
```

The server keeps its database in `.distri/` here; start from a fresh copy each
time.

## What to check

**Esc on a running tree (#154).** Send `Start the slow child and wait for it.`,
wait for `◐ working cancel_slow`, press Esc. The chat says the run was canceled;
ctrl+t shows the root and the child `⊘ canceled`. On the server both rows are
`canceled` and the root has one terminal event, `run_error` `CANCELLED`, with
no `EXECUTION_ERROR` after it:

```bash
sqlite3 .distri/distri.db "select id, parent_task_id, status from tasks"
sqlite3 .distri/distri.db "select substr(task_id,1,8), json_extract(payload,'$.event.type'),
  json_extract(payload,'$.event.code') from task_messages
  where json_extract(payload,'$.event.type') in ('run_error','run_finished')"
```

**x then y on a finished root with a live child (#155).** Send `Start the slow
child in the background.`; the root ends `✓ done` while the child is
`◐ working`. Press ctrl+t, keep the root selected, press x: the status line asks
`… has finished · cancel its live child …? y/n`. Press y: the child shows
`⊘ canceled`, the chat says the background agent was canceled, and the footer
no longer counts it as running. On the server the root stays `completed`, the
child is `canceled`, and it stays `canceled` after its model answers
(`SLOW_SECS` later): no `run_finished` for the child.
