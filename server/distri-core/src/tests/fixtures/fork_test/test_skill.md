---
name: test_fork_skill
description: Smoke-test skill — instructs the worker to log one entry then call final.
tags:
  - test
---

# Test Fork Skill

The caller passed `args: { tag: "<value>" }`, which has been substituted into
`${tag}` below.

## Procedure

1. Call `log_to_memory({ tag: "${tag}" })` exactly once.
2. Call `final({ result: "logged ${tag}" })`.

Do not call any other tool. Do not loop.
