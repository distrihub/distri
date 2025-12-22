# WASM Component Architecture

## Overview

WASM components use the Component Model for type-safe, memory-safe execution of tools and workflows. Components are packaged like any other plugin under `${CURRENT_WORKING_DIR}/plugins/<package>/runtime/`, so pointing `CURRENT_WORKING_DIR=examples` automatically swaps in the test components that `distri-cli load_plugins_dir()` registers before the runtime boots.

## WIT Interface Design

### Core Interface (`distri-plugin-executor/wit/distri.wit`)

```wit
package distri:plugin@0.1.0;

interface types {
  record tool-call {
    tool-call-id: string,
    tool-name: string,
    parameters: string, // JSON string
  }

  record execution-context {
    call-id: string,
    agent-id: option<string>,
    session-id: option<string>,
    task-id: option<string>,
    run-id: option<string>,
    params: string, // JSON string
  }

  record tool-execution-result {
    success: bool,
    data: string, // JSON string
    metadata: option<string>, // JSON string
  }
}

interface tool-executor {
  use types.{tool-call, execution-context, tool-execution-result};
  
  execute-tool: func(tool-name: string, tool-call: tool-call, context: execution-context) -> tool-execution-result;
}

world distri-plugin {
  export tool-executor;
}
```

## Component Implementation

### Host Side (Rust)
- `WasmComponentExecutor` loads and executes WASM components
- Type conversion between Rust structs and WIT types
- Component Model runtime using `wasmtime::component`

### Guest Side (WASM)
- Implements `GuestToolExecutor` trait
- Exports functions via Component Model
- Type-safe struct passing without manual memory management

## Build Process

1. **Rust to WASM**: `cargo build --target wasm32-wasip1 --release`
2. **Component Model**: `wasm-tools component new input.wasm -o output.wasm`
3. **Binding Generation**: `wit-bindgen` generates Rust bindings
4. **Loading**: `wasmtime::component::Engine` loads and instantiates components

## Key Benefits

- **Memory Safety**: No manual memory management
- **Type Safety**: WIT enforces interface contracts
- **Sandboxing**: WASM provides secure execution environment
- **Performance**: Near-native execution speed
- **Portability**: Runs on any platform supporting WASM
