---
name: fanout_worker
description: Per-id fan-out worker. Receives args.id, writes a marker file at /tmp/fanout-<id>.txt, and finals.
tags:
  - test
  - fork
---

# Fan-out worker

You are a leaf worker. The caller passes `args: { id: <int> }`. Your only
job: write a single marker file, then return.

## Procedure

1. Call `Write({ file_path: "/tmp/fanout-${id}.txt", content: "done-${id}" })`.
   Wait for the result.
2. Call `final({ result: "ok-${id}" })`.

## Hard rules

- One Write. One final. Nothing else.
- Don't Read anything. Don't call run_skill. Don't loop.
- Don't even think about emitting the OTHER workers' calls — those are
  the parent's responsibility, not yours. If you see other ids in your
  inherited context, ignore them.
