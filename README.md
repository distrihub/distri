# Distri

![](https://img.shields.io/badge/Platform-macOS%20%7C%20Linux%20%7C%20Windows-blue?style=flat-square) ![](https://img.shields.io/badge/Runtime-Rust-orange?style=flat-square) ![](https://img.shields.io/badge/Protocol-A2A-green?style=flat-square)

Distri is an open-source AI agent framework built in Rust. Define agents in markdown, connect your product functions as tools, and run them from the CLI, as an API server, or embedded in your React UI.

**[Documentation](https://distri.dev/docs/)** · **[Website](https://distri.dev/)** · **[Distri Cloud](https://app.distri.dev)**

![Distri Dashboard](https://distri.dev/img/page_home.png)

---

## Why Distri

- **Agents as markdown** — Define behavior, tools, and model settings in a single `.md` file with TOML frontmatter
- **Built-in tool system** — Builtins (search, shell, browser), dynamic HTTP tools, external client-side tools, and MCP support
- **Skills** — On-demand knowledge documents agents load at runtime via `load_skill`, with automatic re-injection after context compaction
- **Sub-agents** — Agents delegate to specialized child agents with isolated context and token budgets
- **Remote execution** — Run agents in sandboxed containers for untrusted code or resource isolation
- **React SDK** — Drop-in `<Chat />` component with streaming, tool renderers, voice, and developer mode
- **A2A compatible** — Implements the [A2A protocol](https://google.github.io/a2a/) for agent discovery and invocation
- **Self-hosted** — Run on your own infrastructure with SQLite. No external dependencies required.

---

## Installation

```bash
# Install script (macOS / Linux)
curl -fsSL https://distri.dev/install.sh | sh

# Homebrew
brew tap distrihub/distri && brew install distri

# Verify
distri --version
```

<details>
<summary>Other platforms</summary>

**Direct download:**
```bash
# darwin-arm64 | darwin-x86_64 | linux-arm64 | linux-x86_64
TARGET=darwin-arm64
curl -L "https://github.com/distrihub/distri/releases/latest/download/distri-${TARGET}.tar.gz" -o distri.tar.gz
sudo tar -xzf distri.tar.gz -C /usr/local/bin distri
```

**Windows (PowerShell):**
```powershell
Invoke-WebRequest https://github.com/distrihub/distri/releases/latest/download/distri-windows-x86_64.zip -OutFile distri.zip
Expand-Archive distri.zip -DestinationPath $Env:LOCALAPPDATA\distri -Force
$Env:Path += ";$Env:LOCALAPPDATA\distri"
```
</details>

---

## Run

Once installed, start the server and web UI:

```bash
distri serve
```

The first run downloads the matching `distri-server` binary and UI bundle from
[github.com/distrihub/distri/releases](https://github.com/distrihub/distri/releases)
into `~/.distri/`. Opens your browser automatically to `http://localhost:7777`.

### Serve options

| Flag | Behavior |
|------|----------|
| `distri serve` | Default: resolve+spawn distri-server, serve the UI, open browser |
| `--port 8080` | Override port (default 7777) |
| `--no-ui` | API-only mode (no web interface) |
| `--no-browser` | Don't auto-open the browser |
| `--server-version 0.5.3` | Pin a specific server version |
| `--ui-version 0.5.7` | Pin a specific UI version |

### Lifecycle commands

| Command | Behavior |
|---------|----------|
| `distri serve` | Resolve+spawn distri-server, serve the UI |
| `distri update` | Pull the latest server + UI within the compat range |
| `distri update --pre` | Allow pre-release versions |
| `distri version` | Print installed CLI / server / UI versions |
| `distri uninstall` | Wipe `~/.distri/{bin, ui, cache}` |

---

## Quick Start

### Run a built-in agent

```bash
distri run search --task "What are the top programming languages in 2026?"
```

### Define your own agent

Create `agents/my_agent.md`:

```markdown
---
name = "my_agent"
description = "A helpful assistant with web search"
max_iterations = 15

[tools]
builtin = ["final", "search"]
---

# My Agent

You are a helpful assistant. Use the search tool to find current information.

{{task}}
```

### Run it

```bash
# Interactive chat
distri tui my_agent

# Single task
distri run --agent my_agent --task "Find the latest SpaceX launch date"

# Start as API server
distri serve --port 8080
```

### Push to Distri Cloud

```bash
distri login
distri agents push agents/my_agent.md
```

---

## Built-in Agents

| Agent | Purpose | Example |
|-------|---------|---------|
| **search** | Web search + scrape | `distri run search --task "latest AI news"` |
| **fast_search** | Quick single-query lookups | `distri run fast_search --task "population of Tokyo"` |
| **web** | Browser automation + scraping | `distri run web --task "scrape HN top 5 stories"` |
| **code** | Sandboxed code execution | `distri run code --task "sum of primes below 1000"` |
| **deepresearch** | Multi-phase research with sub-agent delegation | `distri run deepresearch --task "state of quantum computing"` |
| **distri** | Master orchestrator | `distri run distri --task "find and calculate..."` |
| **coder** | Full coding agent (shell + web + files) | `distri run coder --task "build a REST API"` |
| **agent_designer** | Design new agent definitions | `distri run agent_designer --task "design a stock alert agent"` |

> Requires `BROWSR_BASE_URL` and `BROWSR_API_KEY` for search, browser, and shell tools. Configured automatically on Distri Cloud.

---

## Agent Definition Format

Agents are markdown files with TOML frontmatter:

```markdown
---
name = "my_agent"
description = "What this agent does"
max_iterations = 25
context_size = 80000
sub_agents = ["search", "code"]     # delegate to other agents

[tools]
builtin = ["final", "search", "load_skill"]
external = ["Read", "Write", "Edit"]  # client-side tools

[[tools.dynamic]]
name = "my_api"
type = "http"
description = "Call my API"
config = { base_url = "$API_URL", headers = { "Authorization" = "Bearer $TOKEN" } }

[[available_skills]]
id = "*"
name = "*"
---

# System Prompt

Your instructions here. Supports Handlebars templates:
- {{task}} — the user's message
- {{> skills}} — available skills listing
- {{> connections}} — available connections
```

See [Agent Definition docs](https://distri.dev/docs/concepts/agent-definition) for the full schema.

---

## Skills

Skills are reusable instruction documents agents load on demand:

```markdown
---
name = "data_pipeline"
description = "How to run the data transformation pipeline"
tags = ["data", "etl"]
context = "inline"
---

# Data Pipeline

## Steps
1. Fetch data with `my_api` tool...
2. Transform using Python...
```

```bash
# Push skills
distri skills push skills/data_pipeline.md
distri skills list
```

Agents call `load_skill("data_pipeline")` at runtime. Two modes:
- **Inline** (default) — content injected into conversation, survives context compaction
- **Fork** — spawns isolated child agent with skill as instructions

---

## React SDK (DistriJS)

Embed agents in your React app:

```bash
npm install @distri/react
```

```tsx
import { useAgent, Chat } from '@distri/react';

function App() {
  const { agent } = useAgent({ agentIdOrDef: 'my_agent' });
  return <Chat agent={agent} threadId="conversation-1" />;
}
```

Features: SSE streaming, tool execution with custom renderers, voice I/O, developer mode, browser-side file tools (IndexedDB-backed Read/Write/Edit/Glob/Grep).

See the [DistriJS docs](https://distri.dev/docs/guides/client/guide-distri-provider) for full integration guide.

---

## CLI Reference

### Server & UI

```bash
distri serve                        # Start server + UI on port 7777
distri serve --port 8080            # Custom port
distri serve --no-ui                # API-only
distri update                       # Pull latest server + UI
distri version                      # Show versions
distri uninstall                    # Clean ~/.distri/
```

### Agent & skill management

```bash
distri                              # Interactive TUI
distri run --task "..." [--agent A] # Run a task
distri run --task "..." --remote    # Run in sandboxed container

distri agents list / push / delete  # Manage agents
distri skills list [-a] / push      # Manage skills
```

### Debugging & tools

```bash
distri traces list / show ID [-v]   # Debug with trace viewer
distri tools list / invoke          # Inspect and test tools
```

### Connections & config

```bash
distri connections list / token     # OAuth connections
distri secrets list / set / delete  # Manage secrets
distri workflows run / push         # DAG workflows

distri login                        # Auth with Distri Cloud
distri profile list / use / config  # Multi-profile management
```

---

## Architecture

```
distri/
├── distri-types/          # Shared type definitions
├── distri-cli/            # CLI binary
├── distri/                # Client library
├── distri-formatter/      # Output formatting
│
├── server/
│   ├── distri-core/       # Agent engine (orchestrator, loop, tools, LLM)
│   ├── distri-server/     # HTTP server (actix-web, A2A endpoints)
│   ├── distri-stores/     # Storage backends (SQLite, Postgres)
│   ├── distri-auth/       # Auth providers
│   ├── distri-parsers/    # Parsing utilities
│   └── agents/            # Built-in agent definitions (.md)
│
├── distrijs/              # TypeScript SDK
│   ├── @distri/core       # Agent client, A2A streaming
│   ├── @distri/react      # React hooks, Chat component, browser tools
│   └── @distri/components # Shared UI (shadcn/ui)
│
└── samples/               # Example applications
    ├── maps-demo/         # Google Maps + AI chat
    └── scraper/           # Web scraping agent
```

### Execution Flow

```
User message → AgentOrchestrator
  → Load agent definition
  → Create ExecutorContext (thread, task, run)
  → AgentLoop
    → PlanningStrategy.plan() → LLM produces plan
    → ExecutionStrategy.execute_step() per step
      → Tool calls (builtin, dynamic, external, MCP)
      → Event streaming (SSE)
    → Loop until final tool or max iterations
  → Return result
```

---

## Development

```bash
# Build
cargo build

# Run tests
cargo test

# Test a specific crate
cargo test -p distri-core

# DistriJS development
cd distrijs && pnpm install && pnpm dev
```

---

## Samples

| Sample | Type | Description |
|--------|------|-------------|
| [maps-demo](./samples/maps-demo) | React/Vite | Interactive Google Maps with AI chat |
| [scraper](./samples/scraper) | CLI/Rust | Web scraping and data extraction agent |

---

## Licensing

- Root and common components: [MIT License](LICENSE)
- Server components (`server/`): [Elastic License 2.0](server/LICENSE) — follows [fair-code principles](https://faircode.io/)

---

## Links

- [Documentation](https://distri.dev/docs/)
- [Distri Cloud](https://app.distri.dev)
- [GitHub Releases](https://github.com/distrihub/distri/releases)
- [Issues](https://github.com/distrihub/distri/issues)
