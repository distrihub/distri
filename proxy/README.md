# MCP Proxy

A flexible proxy server for Model Context Protocol (MCP) that supports various transport methods including stdio, SSE, and WebSocket connections. This proxy allows you to connect to various MCP servers that provide capabilities like file operations, database access, development tools, and more.

## Installation

### As a Binary

```bash
# Install from crates.io
cargo install mcp-proxy
```

## Usage

### Running as a Binary

1. Create a configuration file (e.g., `proxy.yaml`). Here are some examples from the official MCP servers:

```yaml
port: 3004
servers:
  # Data and File Systems
  filesystem:
    type: stdio
    command: npx
    args:
      - -y
      - "@modelcontextprotocol/server-filesystem"
      - "."  # Root directory to expose

  postgres:
    type: stdio
    command: npx
    args:
      - -y
      - "@modelcontextprotocol/server-postgres"
    env_vars:
      DATABASE_URL: "${POSTGRES_URL}"

  # Development Tools
  git:
    type: stdio
    command: npx
    args:
      - -y
      - "@modelcontextprotocol/server-git"

  github:
    type: stdio
    command: npx
    args:
      - -y
      - "@modelcontextprotocol/server-github"
    env_vars:
      GITHUB_PERSONAL_ACCESS_TOKEN: "${GITHUB_TOKEN}"

  # Web and Browser Automation
  puppeteer:
    type: stdio
    command: npx
    args:
      - -y
      - "@modelcontextprotocol/server-puppeteer"

  fetch:
    type: stdio
    command: npx
    args:
      - -y
      - "@modelcontextprotocol/server-fetch"

  # AI and Specialized Tools
  everart:
    type: stdio
    command: npx
    args:
      - -y
      - "@modelcontextprotocol/server-everart"
    env_vars:
      API_KEY: "${EVERART_API_KEY}"

timeout:
  list: 120  # seconds
  call: 60   # seconds
```

2. Run the proxy:

```bash
# List configured servers
mcp-proxy -c proxy.yaml list

# Run the proxy server
mcp-proxy -c proxy.yaml run
```

## Available MCP Servers

The Model Context Protocol provides various server implementations that enable Large Language Models (LLMs) to securely access tools and data sources:

### Reference Implementations

1. **Data and File Systems**
   - **Filesystem** - Secure file operations with configurable access controls
   - **PostgreSQL** - Read-only database access with schema inspection
   - **SQLite** - Database interaction and business intelligence
   - **Google Drive** - File access and search capabilities

2. **Development Tools**
   - **Git** - Read, search, and manipulate Git repositories
   - **GitHub** - Repository management and GitHub API integration
   - **GitLab** - GitLab API integration for project management
   - **Sentry** - Retrieve and analyze issues from Sentry.io

3. **Web and Browser Automation**
   - **Brave Search** - Web and local search using Brave's API
   - **Fetch** - Web content fetching optimized for LLM usage
   - **Puppeteer** - Browser automation and web scraping

4. **Productivity and Communication**
   - **Slack** - Channel management and messaging
   - **Google Maps** - Location services and directions
   - **Memory** - Knowledge graph-based persistent memory

5. **AI and Specialized Tools**
   - **EverArt** - AI image generation using various models
   - **Sequential Thinking** - Dynamic problem-solving
   - **AWS KB Retrieval** - AWS Knowledge Base using Bedrock Agent Runtime

### Official Platform Integrations

- **Axiom** - Query logs and event data
- **Browserbase** - Cloud browser automation
- **Cloudflare** - Manage Cloudflare resources
- **E2B** - Secure cloud sandboxes
- **Neon** - Serverless Postgres platform
- **Obsidian** - Markdown notes access
- **Qdrant** - Vector search engine
- **Raygun** - Crash reporting
- **Search1API** - Unified search API
- **Stripe** - Payment processing
- **Tinybird** - Serverless ClickHouse
- **Weaviate** - Agentic RAG through Weaviate

### Community Servers

The MCP ecosystem includes community-maintained servers:

- **Docker** - Container management
- **Kubernetes** - Container orchestration
- **Linear** - Project management
- **Snowflake** - Data warehouse
- **Spotify** - Music playback control
- **Todoist** - Task management

> Note: Community servers are not officially tested or endorsed. Use them at your own discretion.

## Configuration

The proxy supports three types of server configurations:

1. **stdio**: Execute a local command and communicate via standard input/output
   ```yaml
   type: stdio
   command: string        # Command to execute (e.g., npx, python)
   args: string[]        # Command arguments
   env_vars?: object     # Environment variables
   ```

2. **SSE (Server-Sent Events)**:
   ```yaml
   type: sse
   url: string           # SSE endpoint URL
   headers?: object      # HTTP headers (e.g., authentication)
   ```

3. **WebSocket**:
   ```yaml
   type: ws
   url: string           # WebSocket endpoint URL
   headers?: object      # Connection headers
   ```

### Environment Variables

You can use environment variables in your configuration using `${VAR_NAME}` syntax:

```yaml
servers:
  github:
    type: stdio
    command: npx
    args:
      - -y
      - "@modelcontextprotocol/server-github"
    env_vars:
      GITHUB_PERSONAL_ACCESS_TOKEN: "${GITHUB_TOKEN}"
```

### Timeouts

Configure operation timeouts:
```yaml
timeout:
  list: 120  # seconds for list operations
  call: 60   # seconds for method calls
```

## Testing

To test the proxy server:

```bash
# List available methods
curl -X POST http://127.0.0.1:3004 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc": "2.0", "method": "tools/list", "params": {}, "id": 1}'

# Test filesystem operations
curl -X POST http://127.0.0.1:3004 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "filesystem/list",
    "params": {"path": "."},
    "id": 1
  }'
```

## Additional Resources

- [MCP Examples](https://modelcontextprotocol.io/examples) - Official examples and documentation
- [MCP Servers Repository](https://github.com/modelcontextprotocol/servers) - Complete collection of reference implementations
- [MCP CLI](https://github.com/modelcontextprotocol/cli) - Command-line inspector for testing
- [MCP Get](https://github.com/modelcontextprotocol/get) - Tool for installing and managing servers

## Using as a Library

Add to your `Cargo.toml`:

```toml
[dependencies]
mcp-proxy = "0.1.0"
```

### Basic Usage

```rust
use mcp_proxy::{McpProxy, ProxyServerConfig};
use std::sync::Arc;

// Initialize from YAML string
let config_str = r#"
port: 3004
servers:
  filesystem:
    type: stdio
    command: "npx"
    args: ["-y", "@modelcontextprotocol/server-filesystem", "."]
  github:
    type: stdio
    command: "npx"
    args: ["-y", "@modelcontextprotocol/server-github"]
    env_vars:
      GITHUB_PERSONAL_ACCESS_TOKEN: "${GITHUB_TOKEN}"
"#;

// Parse configuration
let config: ProxyServerConfig = serde_yaml::from_str(config_str)?;
let config = Arc::new(config);

// Initialize the proxy
let proxy = McpProxy::initialize(config).await?;

// Or create directly with new() if you have pre-initialized connections
let proxy = McpProxy::new(servers);

// Use with async-mcp HTTP server
use async_mcp::run_http_server;

run_http_server(port, None, move |transport| {
    let proxy = proxy.clone();
    async move {
        let server = proxy.build(transport).await?;
        Ok(server)
    }
}).await?;
```