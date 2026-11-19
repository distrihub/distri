# SRE agent

> **"Create an SRE agent for https://api.example.com that I can ask from Telegram; check /health every 5 minutes, and if it fails, alert me and start investigating."**

Type that into distri's web chat, paste [`SKILL.md`](SKILL.md) into your coding agent, or run `distri apply` on this folder. All three apply the same blueprint.

## What it sets up

| What | Name | Notes |
|---|---|---|
| Agent | `sre_agent` | Server agent, read-only. Checks URLs, reads GitHub, follows the [`sre_runbook`](skills/sre_runbook.md) skill. |
| Workflow | `sre_health_check` | Calls the health URL every `every_minutes`, alerts once when it goes down, asks `sre_agent` to investigate, and tells you when it recovers. |
| Chat | Telegram | Connect with one link to @distri_bot. You can ask the agent anything there, and alerts arrive there too. |
| Connection | GitHub (optional) | Lets the agent read recent commits and deploys while it investigates. |

```
schedule ─► check (api_call, expect 200) ─┬─ went down? ─► alert ─► investigate (sre_agent) ─► report
                                          ├─ recovered? ─► "✅ recovered"
                                          └─ remember was_ok (state kept between runs)
```

## Run it

```bash
distri login
distri apply samples/sre-agent --set service_url=https://api.example.com/health --wait --test
```

`--wait` waits while you open the Telegram link it prints. `--test` then runs the workflow once in test mode, and you get "Your SRE agent … is connected" on Telegram.

| Param | Default | |
|---|---|---|
| `service_url` | required | Must return 200 when healthy. |
| `service_name` | `the service` | Used in messages. |
| `every_minutes` | `5` | |
| `repo` | none | `owner/name`, used with the GitHub connection. |

## Test

[`test/case.md`](test/case.md) is the use case run on each release. The workflow itself is covered by `distri-core`'s `sre_agent_sample_workflow_end_to_end` test: hello on the test run, one alert, no repeat alert, then recovered.
