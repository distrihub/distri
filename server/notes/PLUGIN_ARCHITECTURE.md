# Plugin System Architecture

This document describes the architecture and implementation of the Distri Plugin System, which enables execution of tools and workflows written in TypeScript and WebAssembly.

## Overview

The Distri Plugin System provides a unified interface for loading and executing plugins across different runtime environments. All paths are now resolved relative to the workspace root indicated by the `CURRENT_WORKING_DIR` environment variable (defaults to `examples/` when unset so local development always finds the sample workspace). A valid workspace exposes three top-level entries:

1. `agents/` – markdown prompt packs discovered by `distri-cli load_agents_dir()`
2. `src/mod.ts` – a TypeScript module that can register tools/workflows/agents programmatically
3. `plugins/` – nested plugin packages where each plugin owns its own `agents/` + `src/`

Run `distri build` (or call `POST /v1/build`) to register these assets up front so the embedded `distri` runtime never needs to inspect hidden `.distri` folders again. The build step scans `${CURRENT_WORKING_DIR}/plugins`, recompiles each package, stores metadata in the catalog, and registers `src/mod.ts` as the workspace module. Runtime/session data (agent transcripts, artifacts, compiled bundles) now lives under `${CURRENT_WORKING_DIR}/.distri/runtime`, leaving the workspace tree strictly for editable source files. It supports:

- **TypeScript/JavaScript Plugins**: Executed using rustyscript runtime
- **WebAssembly Plugins**: Executed using wasmtime runtime  
- **Unified Plugin Interface**: Common trait for plugin operations
- **Auto-detection**: Automatic plugin type detection from `distri.toml` manifest

> **CLI responsibilities**: `distri-cli` now exposes helpers such as `load_agents_dir()` and `load_plugins_dir()` that accept `CURRENT_WORKING_DIR`-relative paths. The CLI registers agents/plugins before `distri` is embedded so the runtime remains agnostic of how those directories were discovered.

## Workspace-Driven UI/API

- `distri-server` now surfaces workspace **agents** (markdown specs) and **files** (raw markdown/TypeScript/etc.) instead of the previous "skills" view. The former skills endpoint must aggregate both plugin entries and loose files so the frontend can show a single Files workspace. We'll reintroduce a new "skills" concept later as partial agents, but the API surface is now explicitly files-first.
- `distrijs/packages/fs` components (`FileWorkspace.tsx`, `FileWorkspaceWithChat.tsx`) talk directly to the server: edits land in IndexedDB immediately, and the explicit **Save** action pushes the buffered content to the backend workspace rooted at `CURRENT_WORKING_DIR`.
- Because object storage is filesystem-backed locally, pointing `CURRENT_WORKING_DIR=examples` provides a self-contained sandbox without touching the real project tree. Cloud deployments will switch the filesystem adapter to an object store, but the interface remains identical; only the `.distri/runtime` namespace is transient per session.
- The orchestrator only hydrates one workspace per process. `distri-cli` is responsible for selecting the correct path (via `CURRENT_WORKING_DIR`) and registering all agents/plugins/files before invoking embedded Distri components.

## Architecture Components

### Core Modules

```
distri-plugin-executor/
├── src/
│   ├── lib.rs              # Public API and exports
│   ├── plugin_trait.rs     # PluginExecutor trait definition
│   ├── plugin_system.rs    # TypeScript plugin executor
│   ├── wasm_system.rs      # WebAssembly plugin executor
│   ├── unified_system.rs   # Unified plugin system
│   └── modules/            # Embedded JavaScript modules
│       ├── base.ts         # Core distri types
│       ├── execute.ts      # Execute functions

```

### Plugin Executor Trait

```rust
#[async_trait::async_trait]
pub trait PluginExecutor: Send + Sync {
    /// Load a plugin from a package directory
    async fn load_plugin(&mut self, package_path: &Path) -> Result<String>;
    
    /// Get information about loaded tools and workflows
    async fn get_plugin_info(&self, package_name: &str) -> Result<PluginInfo>;
    
    /// Execute a plugin item (tool or workflow)
    async fn execute_plugin(&self, package_name: &str, item_name: &str, 
                           item_type: &str, context: ExecutionContext) -> Result<Value>;
    
    /// Get list of loaded plugin names
    fn get_loaded_plugins(&self) -> Vec<String>;
}
```

## TypeScript Plugin System

### Architecture

### Execution Flow

1. **Plugin Loading**:
   - Read `distri.toml` to get TypeScript entrypoint
   - Load user plugin code as a module
   - Register module with unique ID
   - Validate plugin structure

2. **Info Extraction**:
   - Execute plugin module to get exports
   - Extract tool and workflow definitions
   - Cache plugin information

3. **Plugin Execution**:
   - Create execution context with call metadata
   - Inject workflow runtime functions (`callAgent`, `callTool`)
   - Execute specific tool or workflow method
   - Return structured result

### Module System

**Base Module (`distri/base.js`)**:
```javascript
// Core interfaces for tools and workflows
export interface Tool {
    name: string;
    description: string;
    version: string;
    execute(toolCall, context): Promise<any>;
    getParameters?(): object;
}

export interface Workflow {
    name: string;
    description: string;
    version: string;
    execute(workflowCall, context): Promise<any>;
    getParameters?(): object;
}

export interface DistriPlugin {
    tools: Tool[];
    workflows: Workflow[];
}
```

**Workflow Module (`distri/workflow.js`)**:
```javascript
// Runtime functions available to workflows
export async function callAgent(agentName, task, context) {
    // Proxy to distri agent execution
}

export async function callTool(toolName, input, context) {
    // Proxy to distri tool execution  
}
```

## WebAssembly Plugin System

### Current Implementation

The WASM system uses wasmtime for execution but needs upgrading to the Component Model.

**Current Structure**:
```rust
pub struct WasmPluginExecutor {
    engine: Engine,
    plugins: HashMap<String, WasmPlugin>,
}

struct WasmPlugin {
    package_name: String,
    store: Store<()>,
    instance: Instance,
}
```

### Planned Component Model Architecture

**Benefits of Component Model**:
- **Memory Management**: Automatic memory handling, no manual allocation
- **Interface Types**: Strong typing between host and guest
- **Composition**: Better module composition and linking
- **Security**: Enhanced sandboxing and capability-based security

**Proposed Structure**:
```rust
pub struct WasmComponentExecutor {
    engine: wasmtime::Engine,
    components: HashMap<String, Component>,
    linker: wasmtime::component::Linker<HostState>,
}
```

**Component Interface** (WIT):
```wit
// tool.wit
interface tool {
    record tool-call {
        id: string,
        input: string,
    }
    
    record execution-context {
        call-id: string,
        agent-id: option<string>,
        session-id: option<string>,
        params: string, // JSON string
    }
    
    record tool-result {
        result: string,
        error: option<string>,
    }
    
    execute-tool: func(call: tool-call, context: execution-context) -> tool-result;
    get-tool-info: func() -> string; // JSON metadata
}
```

## Unified Plugin System

### Auto-Detection Logic

```rust
pub async fn load_plugin(&mut self, package_path: &Path) -> Result<String> {
    let manifest_path = package_path.join("distri.toml");
    let manifest_content = fs::read_to_string(manifest_path).await?;
    let manifest: DistriConfiguration = toml::from_str(&manifest_content)?;
    
    self.ts_executor.load_plugin(package_path).await,
     
}
```

### Build Command Inference

```rust
fn infer_build_command(plugin_type: &PluginType, package_path: &Path) -> Option<String> {
    match plugin_type {
        PluginType::TypeScript => {
            if package_path.join("package.json").exists() {
                Some("npm run build".to_string())
            } else {
                None // TypeScript doesn't require build
            }
        }
     
    }
}
```

## Plugin Package Structure

### TypeScript Plugin

```
my-ts-plugin/
├── distri.toml                 # Package manifest
├── ts/
│   └── index.ts            # Plugin entrypoint
├── package.json            # Optional npm dependencies
└── dist/                   # Build output
    └── manifest.json
```

**distri.toml**:
```toml
package = "my-ts-plugin"
version = "0.1.0"
description = "TypeScript plugin with tools"

[entrypoints]
ts = "ts/index.ts"
```

**ts/index.ts**:
```typescript
import { Tool, Workflow, DistriPlugin } from 'distri/base';

class MyTool implements Tool {
    name = 'my_tool';
    description = 'Example tool implementation';
    version = '1.0.0';
    
    async execute(toolCall: any, context: any) {
        const params = JSON.parse(toolCall.input);
        return {
            result: `Tool executed with: ${JSON.stringify(params)}`,
            timestamp: new Date().toISOString()
        };
    }
    
    getParameters() {
        return {
            type: 'object',
            properties: {
                message: { type: 'string', description: 'Input message' }
            }
        };
    }
}

const dapExports: DistriPlugin = {
    tools: [new MyTool()],
    workflows: []
};

export default dapExports;
```

### WebAssembly Plugin

```
my-wasm-plugin/
├── distri.toml                 # Package manifest
├── Cargo.toml              # Rust project config
├── src/
│   └── lib.rs              # WASM component source
├── wit/
│   └── tool.wit            # Component interface
└── target/
    └── wasm32-wasi/
        └── release/
            └── my_wasm_plugin.wasm
```

**distri.toml**:
```toml
package = "my-wasm-plugin"
version = "0.1.0"
description = "WebAssembly plugin with tools"

[entrypoints]
wasm = "target/wasm32-wasi/release"
```

**Cargo.toml**:
```toml
[package]
name = "my-wasm-plugin"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.11"
serde = { workspace = true }
serde_json = { workspace = true }
```

**src/lib.rs** (Component Model):
```rust
wit_bindgen::generate!({
    path: "../wit",
});

use exports::tool::{ToolCall, ExecutionContext, ToolResult};

struct MyWasmPlugin;

impl exports::Tool for MyWasmPlugin {
    fn execute_tool(call: ToolCall, context: ExecutionContext) -> ToolResult {
        let input: serde_json::Value = serde_json::from_str(&call.input)
            .unwrap_or_default();
            
        let result = format!("WASM tool executed with: {}", call.input);
        
        ToolResult {
            result,
            error: None,
        }
    }
    
    fn get_tool_info() -> String {
        serde_json::json!({
            "name": "my_wasm_tool",
            "description": "Example WASM tool",
            "version": "1.0.0",
            "parameters": {
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "Input message"
                    }
                }
            }
        }).to_string()
    }
}

export!(MyWasmPlugin with_types_in exports);
```

## Planned Improvements

### 1. Concrete Execution Methods

**Current**: Generic `execute_plugin(item_name, item_type, context)`
**Planned**: Specific methods for different execution types

```rust
#[async_trait::async_trait]
pub trait PluginExecutor: Send + Sync {
    // ... existing methods ...
    
    /// Execute a specific tool
    async fn execute_tool(&self, package_name: &str, tool_name: &str, 
                         tool_call: ToolCall) -> Result<ToolResult>;
    
    /// Execute a workflow (TypeScript only initially)  
    async fn execute_workflow(&self, package_name: &str, workflow_name: &str,
                             workflow_call: WorkflowCall) -> Result<WorkflowResult>;
}
```

### 2. WASM Component Model Integration

**Upgrade Path**:
1. Migrate from `wasmtime::Instance` to `wasmtime::component::Component`
2. Define WIT interfaces for tools and workflows
3. Implement host functions for distri integration
4. Add component linking and composition support

### 3. Distri Integration

**Integration Points**:
- **Agent Execution**: Plugins can call other agents via distri
- **Tool Chaining**: Tools can invoke other tools in the system
- **Workflow Orchestration**: Complex multi-step workflows
- **Resource Access**: Controlled access to system resources

**Integration Architecture**:
```
Distri Runtime
├── Agent Manager
├── Plugin System (distri-plugin-executor)
│   ├── TypeScript Executor
│   ├── WASM Component Executor
│   └── Unified System
└── Resource Manager
    ├── File System Access
    ├── Network Access  
    └── Database Access
```

## Testing Strategy

### Unit Tests
- Plugin loading and validation
- Tool/workflow execution
- Error handling and edge cases
- Memory management (especially WASM)

### Integration Tests  
- End-to-end plugin execution via distri
- Inter-plugin communication
- Resource access validation
- Performance benchmarking

### Test Plugin Implementation
Create a reference WASM plugin that demonstrates:
- Tool implementation with component model
- Proper memory management
- Host function integration
- Error handling patterns

## Security Considerations

### Sandboxing
- **TypeScript**: V8 isolate sandboxing via rustyscript
- **WASM**: Component model capability-based security
- **Resource Access**: Controlled via host functions

### Capability Model
- Plugins declare required capabilities in manifest
- Runtime enforces capability restrictions
- Fine-grained permission system

### Memory Safety
- **TypeScript**: Automatic garbage collection
- **WASM Component**: Linear memory with bounds checking
- **Host**: Rust's memory safety guarantees

## Performance Considerations

### Runtime Overhead
- **Plugin Loading**: Amortized cost via caching
- **Execution**: Minimal overhead for native WASM
- **Memory Usage**: Isolated per-plugin memory spaces

### Optimization Strategies
- Plugin preloading for frequently used tools
- Result caching for deterministic operations
- Connection pooling for network resources
- Lazy loading of plugin dependencies

## Migration Path

### Phase 1: Concrete Methods (Current)
- Replace `execute_plugin` with `execute_tool` and `execute_workflow`
- Add TypeScript workflow execution support
- Maintain backward compatibility

### Phase 2: WASM Component Model
- Implement component model executor
- Define standard WIT interfaces
- Migrate existing WASM plugins

### Phase 3: Distri Integration
- Deep integration with distri runtime
- Advanced workflow orchestration
- Production-ready plugin marketplace

### Phase 4: Advanced Features
- Plugin composition and dependency injection
- Dynamic plugin loading and hot reloading  
- Advanced security and resource management
