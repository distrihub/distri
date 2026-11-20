# Calling agents: A2A and the CLI

Moved from the README. Every agent is an A2A 1.0 JSON-RPC endpoint at `/v1/agents/<name>`.
A self-hosted `distri-server` listens on `http://127.0.0.1:8081/v1` (`--port` or `DISTRI_PORT` to change it).



Every agent is an [A2A 1.0](https://a2a-protocol.org/) JSON-RPC endpoint. Any A2A client
(e.g. a2a-rs's `a2acli`) works unchanged; distri's additions are opt-in
[A2A extensions](docs/a2a-extensions/README.md) a client activates with the `A2A-Extensions` header.

**Send a message**

```bash
curl -X POST https://api.distri.dev/v1/agents/shop_agent \
  -H "Authorization: Bearer $DISTRI_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": "1",
    "method": "SendMessage",
    "params": {
      "message": {
        "messageId": "m-1",
        "role": "ROLE_USER",
        "parts": [{ "text": "What is in my cart?" }]
      }
    }
  }'
```

**Stream the reply** over SSE: use `"method": "SendStreamingMessage"` and `curl -N`. Add
`-H "A2A-Extensions: https://distri.dev/a2a/extensions/event-stream/v1"` for distri's full event
stream. (The A2A 0.x method names, `message/send` …, still work for one release.)

**Self-hosted:** the same calls work against `http://127.0.0.1:8081/v1/agents/shop_agent`.

```bash
curl http://127.0.0.1:8081/v1/agents                                        # list agents
curl http://127.0.0.1:8081/v1/agents/shop_agent/.well-known/agent-card.json  # A2A agent card
```

**From the CLI**

```bash
distri run --agent shop_agent --task "What is in my cart?"   # run one task
distri tui shop_agent                                        # chat in the terminal
distri push                                                      # sync agents, skills and templates
distri traces list                                               # inspect runs
distri serve                                                     # run the server and UI locally
```

**A2A from the CLI**, by domain (`--agent` defaults to `$DISTRI_AGENT`, else `distri`; `--json`
prints the protocol JSON, one frame per line when streaming; `--extensions` activates A2A
extensions by name or URI, and the ones the server activated are reported on stderr)

```bash
distri agents card shop_agent [--extended] [--json]          # the public A2A card
distri run --agent shop_agent --task "…" --json              # SendMessage (waits for the task)
distri run --task "…" --stream [--extensions event-stream]       # SendStreamingMessage: one line per frame
distri run --task "…" --async                                    # print the task id and return
distri run --task "…" --context-id <thread> --task-id <id> --data-part '{"k":1}' --metadata @meta.json
distri tasks get <task_id> [--history 5]                         # GetTask
distri tasks list [--context <thread>] [--state input-required] [--limit 20]   # ListTasks
distri tasks cancel <task_id>                                    # CancelTask
distri tasks subscribe <task_id>                                 # SubscribeToTask (event-stream)
distri tasks push-config list|get|set|delete <task_id> …         # push configs (set --url [--token])
distri tasks pending <task_id>                                   # pending calls (durable-runs extension)
```

`distri run` without those flags is the interactive runner (local tools, terminal rendering); any
of `--json`, `--stream`, `--async`, `--metadata`, `--data-part`, `--extensions` makes it a plain,
non-interactive A2A send.

With durable runs on (`DISTRI_DURABLE_RUNS=1`), the agent card lists the A2A extension
`https://distri.dev/a2a/extensions/durable-runs/v1` in `capabilities.extensions`; its `params`
name the pending-calls and claim endpoints.



## CLI reference

```bash
distri                                   # interactive TUI
distri tui my_agent                      # chat with one agent
distri run --agent my_agent --task "…"   # run a single task

distri push                              # sync agents, skills and templates
distri push --dry-run                    # preview what would upload
distri agents list
distri skills list
distri traces list                       # inspect runs
distri agents card my_agent              # the agent's public A2A card
distri tasks list                        # A2A tasks: list/get/cancel/subscribe/pending
distri secrets list
distri connections list

distri update                            # pull the latest server and UI
distri version
```



