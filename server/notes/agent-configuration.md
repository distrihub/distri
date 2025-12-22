# Agent Configuration Architecture

## Overview

Agents are configured using TOML files that define their behavior, model settings, and execution parameters. Agent discovery is now scoped to the active workspace (`CURRENT_WORKING_DIR`, defaults to the repo root or `examples/` during development). Every workspace must expose an `agents/` directory at its root so `distri-cli` can invoke `load_agents_dir()` before the embedded runtime starts, alongside `src/mod.ts` and `plugins/` to complete the contract for a runnable Distri workspace.

## Agent Definition Structure

### Basic Configuration (`agents/my_agent.toml`)

```toml
name = "my_agent"
version = "0.1.0"
description = "AI assistant with tool access"

instructions = """
You are a helpful AI assistant with access to tools.

CRITICAL: When you complete your task, call final("result") to end execution.
Available tools will be automatically detected from Distri dependencies.
"""

[strategy]
reasoning_depth = "standard"  # shallow, standard, deep

[strategy.execution_mode]
type = "tools"  # tools, code

[model_settings]
model = "gpt-4.1-mini"
temperature = 0.7
max_tokens = 4000

max_iterations = 5
```

## Strategy Configuration

### Reasoning Depth
- **shallow**: Direct, concise responses with minimal steps
- **standard**: Balanced approach with moderate reasoning
- **deep**: Comprehensive analysis with detailed reasoning

### Execution Modes
- **tools**: Agent has access to tools from Distri dependencies
- **code**: Agent can execute code (TypeScript/JavaScript)

## Model Settings

### Supported Models
- `gpt-4.1-mini`: Fast, cost-effective model
- `gpt-4o`: Advanced reasoning capabilities
- `claude-3-sonnet`: Alternative LLM provider

### Parameters
- `temperature`: Creativity/randomness (0.0-1.0)
- `max_tokens`: Maximum response length
- `top_p`: Nucleus sampling parameter

## Execution Limits

### Step Management
- `max_steps`: Maximum number of reasoning/action cycles
- Agents receive step count warnings
- Must use completion tools (`final()`) before reaching limit

## Tool Integration

Tools are automatically available to agents based on:
1. **Distri Dependencies**: Tools from dependent packages
2. **Built-in Tools**: Core system tools (file operations, etc.)
3. **Completion Tools**: `final()` for tools mode, `final_answer()` for code mode
