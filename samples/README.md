# Samples: deploy an agent with one sentence

Each folder here is a **blueprint**: one sentence sets up an agent, its workflows and where it lives (a Telegram chat, a website). Use it in whichever way suits you:

- **distri web chat:** type the sentence. distri proposes the blueprint, you confirm it, then open the links it gives you.
- **A coding agent** (Claude Code, Codex, Cursor): paste the sample's `SKILL.md`.
- **The CLI:** `distri apply samples/<name> --set …`, or `distri apply <name>` for a sample the server ships (`distri apply --list` shows them).

| Sample | The sentence | Sets up |
|---|---|---|
| [sre-agent](sre-agent) | *Create an SRE agent for my API that I can ask from Telegram; check /health every 5 minutes, and if it fails, alert me and start investigating.* | A server agent, a scheduled health check that branches (alert once, investigate, then recovered) and a Telegram chat |
| [website-support](website-support) | *Create an onboarding agent for my product on example.com that answers questions from our docs and opens a support ticket when it can't help.* | A web agent embedded on your site that calls a server workflow as a tool to file Linear tickets, and Telegram for ticket notices |
| [signal-digest](signal-digest) | *Every weekday at 8am, send me a Telegram digest of the posts worth reading about … on Hacker News and Reddit.* | A scheduled workflow with a TypeScript code step, a writer agent, and Telegram |

## A blueprint folder

```
<name>/
  blueprint.toml   # name, the sentence, params (asked when missing), channel / embed, connections, secrets, test
  agents/*.md      # agent definitions
  workflows/*.json # workflow agents (steps: api_call, code, condition, agent_run, notify, set_state, …)
  skills/*.md
  tools/*.ts       # @distri/tool modules used by `code` steps (run with Deno; net only, as declared)
  SKILL.md         # for coding agents
  README.md
  test/case.md     # the use case checked on each release
  test/expect.toml # what a blueprint written from the sentence must contain
```

`{{params.x}}` in any file is filled in when the blueprint is applied. Workflow placeholders (`{input.x}`, `{steps.id.x}`, `{state.x}`, `{env.X}`) are resolved at run time.

Older one-file workflow examples are in [`../examples/workflows`](../examples/workflows).
