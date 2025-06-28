# Migration from async-mcp to rust-mcp-sdk

## Overview
Successfully migrated from `async-mcp` to `rust-mcp-sdk` v0.4.6, a mature community implementation of the Model Context Protocol for Rust.

## Why rust-mcp-sdk instead of official SDK?
- The official `rmcp` SDK from modelcontextprotocol/rust-sdk is still in early development ("Wait for the first release")
- `rust-mcp-sdk` provides comprehensive features with good documentation
- Supports resources, tools, prompts, server/client implementations
- Has active community support and examples

## Key Changes Required

### 1. Dependencies Updated
- Root Cargo.toml: `async-mcp` → `rust-mcp-sdk = { version = "0.4", features = ["server", "macros"] }`
- All workspace members updated accordingly

### 2. API Differences (rust-mcp-sdk vs async-mcp)
Based on rust-mcp-sdk documentation, the main patterns are:

#### Server Creation (Old vs New)
```rust
// OLD (async-mcp)
use async_mcp::server::{Server, ServerBuilder};
let mut server = Server::builder(transport)
    .capabilities(ServerCapabilities { tools: Some(json!({})), ..Default::default() })
    .request_handler("tools/list", handler)
    .build();

// NEW (rust-mcp-sdk)  
use rust_mcp_sdk::server_runtime;
use rust_mcp_sdk::transport::StdioTransport;
use rust_mcp_sdk::ServerHandler;

let server_details = InitializeResult { /* ... */ };
let transport = StdioTransport::new(TransportOptions::default())?;
let handler = MyServerHandler {};
let server = server_runtime::create_server(server_details, transport, handler);
```

#### Tool Registration (Old vs New)
```rust
// OLD (async-mcp)
server.register_tool(tool_definition, |req: CallToolRequest| {
    Box::pin(async move { /* handler */ })
});

// NEW (rust-mcp-sdk)
impl ServerHandler for MyHandler {
    async fn handle_call_tool_request(&self, request: CallToolRequest, runtime: &dyn McpServer) -> Result<CallToolResult, CallToolError> {
        // handler logic
    }
}
```

### 3. Status
- ✅ Dependencies updated
- 🔄 Code migration in progress
- ⚠️  Need to update imports and API calls
- ⚠️  Need to implement ServerHandler traits

### 4. Components to Migrate
1. twitter-mcp/src/server.rs
2. code-mcp/src/server.rs  
3. proxy/src/server.rs
4. distri/src/tools.rs (client usage)
5. distri/src/servers/ (various server builders)

### 5. Benefits of Migration
- Better resource support (async-mcp had limited resource support)
- More robust tool handling
- Better documentation and examples
- Active community maintenance
- Support for latest MCP protocol features

## Next Steps
1. Update imports to use rust-mcp-sdk
2. Implement ServerHandler traits for each MCP server
3. Update client code to use new client API
4. Test all functionality
5. Update configuration if needed