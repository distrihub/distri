# Distri

![](https://img.shields.io/badge/Platform-macOS%20%7C%20Linux%20%7C%20Windows-blue?style=flat-square) ![](https://img.shields.io/badge/Runtime-Distri%20CLI-orange?style=flat-square) ![](https://img.shields.io/badge/Protocol-A2A-green?style=flat-square)

Distri is an A2A-compatible agent framework built in Rust. Build agents with simple markdown definitions and integrate them within your React frontend. Tools can be added as either backend tools via Deno or frontend tools, making building agents much easier.

**[Documentation](https://distri.dev/docs/)** · **[Website](https://distri.dev/)** · **[A2A Protocol](https://a2a-protocol.org/)**

![Distri Dashboard](https://distri.dev/img/page_home.png)

## Architecture

Distri is organized as a monorepo with Rust crates and TypeScript packages:

```
distri/
├── Rust Crates
│   ├── distri-cli          # CLI tool for agent management
│   ├── distri-a2a          # A2A protocol implementation
│   ├── distri-types        # Shared type definitions
│   ├── distri-filesystem   # File system utilities
│   └── distri              # Core library
│
├── Server (ELv2 License)
│   ├── distri-server       # A2A-compatible agent server
│   ├── distri-core         # Core server logic
│   ├── distri-stores       # Storage backends
│   ├── distri-plugins      # Plugin system
│   └── distri-plugin-executor  # Deno-based plugin runtime
│
├── DistriJS (TypeScript)
│   ├── @distri/core        # Agent client & A2A integration
│   ├── @distri/react       # React hooks & components
│   ├── @distri/components  # Shared UI components (shadcn/ui)
│   └── @distri/fs          # File system tools for frontend
│
├── samples/                # Example applications
└── plugins/                # Integration plugins
```

## Screenshots

<table>
  <tr>
    <td><img src="https://distri.dev/img/page_agents.png" alt="Agents" width="400"/></td>
    <td><img src="https://distri.dev/img/page_chat.png" alt="Chat" width="400"/></td>
  </tr>
  <tr>
    <td align="center"><b>Agent Library</b></td>
    <td align="center"><b>Chat Interface</b></td>
  </tr>
  <tr>
    <td><img src="https://distri.dev/img/page_threads.png" alt="Threads" width="400"/></td>
    <td><img src="https://distri.dev/img/embedded.png" alt="Embedded" width="400"/></td>
  </tr>
  <tr>
    <td align="center"><b>Conversation Threads</b></td>
    <td align="center"><b>Embedded Mode</b></td>
  </tr>
</table>

## Installation

### Prebuilt binary (recommended)
```bash
curl -fsSL https://distri.dev/install.sh | sh
```

Pin a specific version or choose an install location:
```bash
DISTRI_VERSION=0.3.0 DISTRI_INSTALL_DIR=/usr/local/bin sh -c "$(curl -fsSL https://distri.dev/install.sh)"
```

### Direct download (macOS / Linux)
```bash
# darwin-arm64 | darwin-x86_64 | linux-arm64 | linux-x86_64
TARGET=darwin-arm64
curl -L "https://github.com/distrihub/distri/releases/latest/download/distri-${TARGET}.tar.gz" -o distri.tar.gz
sudo tar -xzf distri.tar.gz -C /usr/local/bin distri
```

### Homebrew (macOS / Linux)
```bash
brew tap distrihub/distri
brew install distri
```

### Windows (PowerShell)
```powershell
Invoke-WebRequest https://github.com/distrihub/distri/releases/latest/download/distri-windows-x86_64.zip -OutFile distri.zip
Expand-Archive distri.zip -DestinationPath $Env:LOCALAPPDATA\distri -Force
$Env:Path += ";$Env:LOCALAPPDATA\distri"
```

### Verify installation
```bash
distri --version
distri help
```

## Quick Start

```bash
# Navigate to your project
cd path/to/your/project

# Push agents to Distri Cloud
distri push

# Run a task
distri run <agent-name> --task "Your task"
```

## Built-in Agents

These agents are auto-loaded in both `distri-server` (self-hosted) and Distri Cloud. Access them via the CLI client (`distri run`) or the server directly (`distri-server run`).

### fast_search — Quick lookups
```bash
distri run fast_search --task "What is the population of Tokyo?"
```

### search — Search + scrape
```bash
distri run search --task "What are the top 3 programming languages in 2026?"
```

### web — Browser automation + scraping
```bash
distri run web --task "Scrape https://news.ycombinator.com and extract the top 5 story titles and links"
```

### code — Sandboxed code execution (Python, bash, JavaScript)
```bash
distri run code --task "Calculate the sum of all prime numbers below 1000 using Python"
```

### deepresearch — Multi-phase research with sub-agent delegation
```bash
distri run deepresearch --task "Research the current state of quantum computing. What are the top 3 companies and their latest breakthroughs?"
```

### distri — Master orchestrator
```bash
distri run distri --task "Find the latest SpaceX launch date and calculate how many days from now"
```

### agent_designer — Design new agents
```bash
distri run agent_designer --task "Design an agent that monitors stock prices and sends alerts when they cross a threshold"
```

> **Environment:** `BROWSR_BASE_URL` and `BROWSR_API_KEY` are required for search, scrape, browser, and shell tools. When using Distri Cloud these are configured automatically.

## Samples

| Sample | Type | Description | Demo |
|--------|------|-------------|------|
| [maps-demo](./samples/maps-demo) | React/Vite | Interactive Google Maps with AI chat | [Live](https://distrihub.github.io/distri/samples/maps) |
| [coder](./samples/coder) | CLI/Rust | Code generation assistant | - |
| [scraper](./samples/scraper) | CLI/Rust | Web scraping and data extraction agent | - |

### Embedding in iframe
```html
<iframe
  src="https://distrihub.github.io/distri/samples/maps"
  width="100%"
  height="600"
  frameborder="0">
</iframe>
```

## Plugins

Ready-to-use integrations in `plugins/`:

| Plugin | Description |
|--------|-------------|
| `slack` | Slack messaging and workflows |
| `notion` | Notion pages and databases |
| `gmail` | Gmail read/send capabilities |
| `google-calendar` | Calendar events management |
| `google-docs` | Document creation and editing |
| `google-sheets` | Spreadsheet operations |
| `postgresql` | Database queries |
| `clickhouse` | Analytics database |

## Development

### Building Rust crates
```bash
cargo build
```

### Publishing crates
```bash
cargo publish -p distri-a2a
cargo publish -p distri-types
cargo publish -p distri-filesystem
cargo publish -p distri
```

### DistriJS development
```bash
cd distrijs
pnpm install
pnpm dev
```

## Releases

Check the [GitHub Releases](https://github.com/distrihub/distri/releases) page for the latest binaries and changelog.

## Licensing

- Repository root and common components: [MIT License](LICENSE)
- Server components (`server/`): [Elastic License 2.0](server/LICENSE) - follows [fair-code principles](https://faircode.io/)

## Support

Questions or feedback? [Open an issue](https://github.com/distrihub/distri/issues).
