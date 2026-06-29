# TypeScript Plugin Architecture

## Overview

TypeScript plugins provide a flexible way to implement tools and workflows using JavaScript/TypeScript runtime. Plugins now live under `${CURRENT_WORKING_DIR}/plugins/` so that switching workspaces (for example, setting `CURRENT_WORKING_DIR=examples`) immediately swaps the agent + plugin catalog. Each workspace must also expose `agents/` for markdown prompt packs and `src/mod.ts` for programmatic registrations to keep Distri embeddable.

### Workspace Layout

```
${CURRENT_WORKING_DIR}/
├── agents/          # Markdown agents discovered via distri-cli load_agents_dir()
├── src/
│   └── mod.ts       # Optional TypeScript entry for tools/workflows/agents
└── plugins/
    └── my_package/  # See package structure below
```

## DistriPlugin Interface

### Core Structure (`src/index.ts`)

```typescript
import { DistriPlugin, DistriWorkflow } from "./base.ts";
import myWorkflow from "./workflows/my_workflow.ts";

// Tool implementations would be imported similarly
const plugin: DistriPlugin = {
    tools: {},  // Tools object (can be array or object)
    workflows: [myWorkflow]  // Array of workflow implementations
};

export default plugin;
```

### Workflow Structure (`src/workflows/my_workflow.ts`)

```typescript
import { DistriWorkflow, callAgent, callTool } from "../base.ts";

async function run(input: any, context: any): Promise<any> {
    console.log('🚀 Starting workflow:', input);
    
    try {
        // Call agents or tools using runtime functions
        const result = await callAgent({
            agent_name: 'my_agent',
            task: input.task,
            session_id: context.session_id
        });
        
        return {
            success: true,
            result: result,
            timestamp: new Date().toISOString()
        };
    } catch (error: any) {
        return {
            success: false,
            error: error.message,
            timestamp: new Date().toISOString()
        };
    }
}

const myWorkflow: DistriWorkflow = {
    name: "my_workflow",
    description: "My workflow description",
    version: "1.0.0",
    
    async execute(params: any, context: any): Promise<any> {
        return await run(params, context);
    },
    
    getParameters(): any {
        return {
            task: { type: "string", required: true, description: "Task to execute" }
        };
    }
};

export default myWorkflow;
```

## Runtime Environment

### Execution Engine
- Uses `rustyscript` for JavaScript/TypeScript execution
- Worker pattern for isolation and safety
- Module loading with custom import provider

### Context and Tools
- `ToolContext` provides agent_id, session_id, task_id, run_id
- Session store access for persistent data
- Error handling and logging capabilities

## Plugin Configuration

### Package Structure
```
my_package/
├── distri.toml                # Package manifest
├── agents/                    # Agent configuration files
│   └── my_agent.toml         
└── src/                       # TypeScript source files
    ├── index.ts              # Plugin entrypoint
    ├── base.ts               # Base types and functions  
    └── workflows/            # Workflow implementations
        └── my_workflow.ts    
```

### distri.toml Format
```toml
package = "my_package"
version = "0.1.0"
description = "My package description"
agents = ["agents/my_agent.toml"]    # List of agent config files

[entrypoints]                        # Required for TypeScript plugins
type = "ts"                          # Must be "ts" for TypeScript
path = "src/index.ts"               # Path to TypeScript entrypoint

[dependencies]                       # Optional path dependencies
other_package = { path = "../other_package" }
```

### Agent Configuration (agents/my_agent.md)
```markdown
---
name = "my_agent"              # ⚠️ MUST use underscores, NOT hyphens!
description = "My agent description"
max_iterations = 10            # Use max_iterations, NOT max_steps
instructions = '''
Your agent instructions here...

Always call final() with your result to complete execution.
'''

[model_settings]
model = "gpt-4.1-mini"
temperature = 0.7
max_tokens = 1000

[strategy]
reasoning_depth = "standard"   # NOT preset = "simple"

[strategy.execution_mode]
type = "code"                  # or "tools" 
language = "typescript"        # when type = "code"
---

# My Agent

Agent documentation here.
```

### ⚠️ Agent Configuration Critical Rules

1. **File Extension**: Must be `.md` (markdown) with TOML frontmatter
2. **TOML Syntax**: Use `=` not `:` in frontmatter
3. **Agent Names**: Must use `underscores`, NOT `hyphens` (becomes function name)
4. **Field Names**: Use `max_iterations` not `max_steps`, `reasoning_depth` not `preset`
5. **Markdown Content**: Must include content after closing `---`

## Plugin Discovery

1. **Entrypoint Loading**: Module registered at actual file path (`package_path/src/index.ts`)
2. **Import Resolution**: Relative imports work naturally (`./workflows/file.ts`, `../base.ts`)
3. **Module Execution**: Runtime loads and executes TypeScript module
4. **Export Discovery**: Extracts tools and workflows from `DistriPlugin`
5. **Registration**: Tools/workflows registered in plugin system

## Import Resolution - SOLVED ✅

**Key Solution**: Register plugin modules at their actual file paths instead of synthetic paths.

### How It Works
1. **Module Registration**: Plugins are registered at their actual workspace paths (for example, `${CURRENT_WORKING_DIR}/plugins/my_package/src/index.ts`) rather than synthetic names.
2. **Relative Imports**: Standard filesystem relative imports work: `./workflows/file.ts`, `../base.ts`
3. **Import Provider**: Simple relative path resolution using standard filesystem operations

## Integration Points

- **Agent Calling**: `callAgent(agentName, task)` for agent-to-agent communication
- **Tool Calling**: `callTool(packageName, toolName, params)` for tool composition
- **Context Access**: Full execution context including session management
- **Error Handling**: Graceful error propagation and logging
