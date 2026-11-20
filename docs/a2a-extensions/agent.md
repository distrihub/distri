# agent (descriptor)

`https://distri.dev/a2a/extensions/agent/v1` — on every card; declarative, never activated.

What a client needs to *use* the agent. The card is the public descriptor; the definition
(instructions, tools internals, MCP configuration, model settings, connections) is a developer API
(`GET /agents/{id}` needs the developer permission) and never on the card.

`params`:

| key | |
|---|---|
| `agent_type` | `standard_agent` or `workflow_agent` |
| `external_tools` | client tool names the agent expects (`"*"` excluded) |
| `any_tool` | the agent accepts any client tool (`tools.external = ["*"]`) |
| `runtime` | `web`, `server` or `cli` (absent when unset) |
| `browser_streaming` | the agent streams a browser session |
| `version_no` | the saved version number (hosts that version agents) |

Built by `distri_types::a2a_extensions::AgentDescriptor::of` from the same contract logic as version
checks (`distri_types::agent_contract::contract_of`).
