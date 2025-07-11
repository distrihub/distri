# Distri Project Analysis

## Overview
Distri is a Rust-based framework for building and composing AI agents that is compatible with both A2A (Agent-to-Agent) and MCP (Multi-Agent Communication Protocol) standards. The project aims to enable developers to create, publish, and combine agent capabilities in a composable manner.

## Architecture

### Core Components
1. **Distri Server** - A2A-compliant agent server with task management
2. **Distri CLI** - Command-line interface for agent management and execution
3. **Distri Frontend** - React application using AG-UI for agent interaction (referenced but not in workspace)
4. **Local Coordinator** - Manages agent execution and tool integration
5. **MCP Proxy** - Provides proxy functionality for stdio commands

### Workspace Structure
```
distri/
├── distri/           # Main library crate
├── distri-cli/       # Command line interface
├── distri-server/    # Server implementation
├── distri-a2a/       # A2A protocol types
├── proxy/            # MCP proxy
└── samples/          # Example agents (search, twitter-bot)
```

## Key Features

### Implemented ✅
- **A2A Protocol Compliance**: Agent cards, message handling, task management, JSON-RPC
- **Task Store**: HashMap-based storage with thread-safe operations and task lifecycle management
- **Event Streaming**: Server-Sent Events (SSE) for real-time updates
- **Agent Management**: Dynamic create, update, and management APIs
- **CLI Interface**: List agents, run agents, proxy functionality
- **MCP Integration**: Tool registry and MCP server support

### Planned/In Progress ⏳
- **Error Handling**: Proper A2A error codes and messages
- **Agent Discovery**: Dynamic agent registration
- **Redis Backend**: Distributed task storage
- **Message History**: Full conversation history per task
- **Authentication**: JWT-based security
- **Rust Scripts**: Advanced workflow configurations

## Recent Additions (Agent Management)

### New APIs
- `PUT /api/v1/agents/{agent_id}` - Update existing agent
- `POST /api/v1/agents` - Create new agent
- `GET /api/v1/schema/agent` - Get agent definition schema

### CLI Enhancements
- `distri update-agents` - Update all agents from config
- Automatic agent refresh when loading from config
- Better synchronization between config files and agent store

### Store Updates
- New `update` method in `AgentStore` trait
- Enhanced `AgentExecutor` with `update_agent` method
- Support for both InMemoryAgentStore and RedisAgentStore

## Technical Stack
- **Language**: Rust
- **Async Runtime**: Tokio
- **Serialization**: Serde
- **Protocols**: JSON-RPC, A2A, MCP
- **Architecture**: Multi-crate workspace
- **Dependencies**: async-mcp, anyhow, tracing

## Current Development Status
⚠️ **Early Development**: APIs and protocols may change as the project evolves. The framework is functional but still stabilizing.

## Usage Patterns

### Configuration-Based (Current)
Agents are defined in YAML configuration files with:
- Agent metadata (name, description)
- System prompts
- Model settings
- MCP server configurations

### Script-Based (Coming Soon)
Rust scripts for advanced workflows and custom agent behaviors.

## Integration Points
- **AI Gateway**: Connected to Langdb AI Gateway with 250+ LLMs
- **MCP Ecosystem**: Compatible with MCP tools and servers
- **Event Streaming**: Real-time updates via SSE
- **JSON-RPC**: Standard protocol for agent communication

## Operational Model
The system supports both configuration-driven and API-driven agent management, allowing for:
- Static agent definitions in config files
- Dynamic agent creation and updates via REST API
- Real-time monitoring and task management
- Composable agent workflows

## Key Strengths
1. **Multi-Protocol Support**: Both A2A and MCP compatibility
2. **Composable Design**: Agents can be combined and reused
3. **Real-time Updates**: SSE-based event streaming
4. **Dynamic Management**: Runtime agent updates without restart
5. **Rust Performance**: Built in Rust for reliability and performance
6. **Modular Architecture**: Clear separation of concerns across crates

This project represents a sophisticated attempt to create a standardized, composable agent framework that can serve as infrastructure for AI agent ecosystems.