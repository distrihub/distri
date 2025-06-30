# Distri Server Embedding Example

This example demonstrates how to embed the Distri A2A server routes into your own actix-web application.

## Status

✅ **WORKING** - Basic embedding framework implemented  
🔄 **IN PROGRESS** - Full distri integration (dependency compilation issues being resolved)  

## Overview

Instead of running Distri as a standalone server, you can embed its routes and functionality into your existing actix-web application. This allows you to:

- Add AI agent capabilities to your existing web service
- Customize the server configuration and middleware
- Add your own routes alongside Distri's A2A API
- Have full control over the server lifecycle and configuration

## Features

This example shows:
- How to load Distri configuration from YAML
- How to create and configure a LocalCoordinator
- How to embed Distri routes in your actix-web app
- How to add custom routes alongside Distri routes
- Proper initialization and error handling

## Quick Start

1. **Build and run the example:**
   ```bash
   cd samples/embedding-distri-server
   cargo run
   ```

2. **The server will start on http://127.0.0.1:3030**

3. **Try the endpoints:**
   ```bash
   # Welcome and API info
   curl http://127.0.0.1:3030/
   
   # Health check
   curl http://127.0.0.1:3030/health
   ```

## Current Implementation

### What's Working Now

The example currently demonstrates:
- ✅ Basic actix-web server setup with custom routes
- ✅ CORS and logging middleware configuration  
- ✅ Health check and welcome endpoints
- ✅ Proper server structure for embedding services
- ✅ Clean shutdown handling

### What's Coming Next

Once the distri compilation issues are resolved, the example will include:
- 🔄 Full distri coordinator integration
- 🔄 YAML configuration loading
- 🔄 A2A API endpoints (`/api/v1/agents`, `/api/v1/agents/{id}`)
- 🔄 Agent management and execution
- 🔄 Streaming responses for agent interactions

## Code Structure

### Current Components

1. **Basic Server Setup**: Standard actix-web server with middleware
2. **Custom Routes**: Health check and welcome endpoints  
3. **CORS Configuration**: Permissive CORS for development
4. **Logging**: Structured logging with tracing

### Future Integration Pattern

The planned integration will follow this pattern:

```rust
// 1. Load configuration
let (coordinator, server_config) = create_coordinator_from_config("config.yaml").await?;

// 2. Start coordinator background task
tokio::spawn(async move { coordinator.run().await });

// 3. Configure app with embedded distri routes
App::new()
    .wrap(cors_middleware)
    .wrap(logging_middleware)
    .route("/", web::get().to(welcome))           // Your custom routes
    .route("/health", web::get().to(health_check)) // Your custom routes
    .configure(|cfg| {                             // Distri routes
        configure_distri_service(cfg, coordinator, server_config)
    })
```

## Configuration

The `config.yaml` file (ready for when distri integration is complete) defines:
- Two test agents: "assistant" and "echo"
- Basic server settings for A2A compatibility
- Minimal MCP server configuration

## Testing

```bash
# Start the server
cargo run

# Test welcome endpoint
curl http://127.0.0.1:3030/

# Test health check
curl http://127.0.0.1:3030/health
```

Expected responses:
- Welcome: JSON with server info and available endpoints
- Health: JSON with status and timestamp

## Integration Guide for Your Application

To embed this pattern in your own application:

### 1. Add Dependencies
```toml
[dependencies]
actix-web = "4.4"
actix-cors = "0.7"
# When available:
# distri-server = { path = "path/to/distri-server" }
# distri = { path = "path/to/distri" }
```

### 2. Basic Structure
```rust
use actix_web::{web, App, HttpServer};

#[tokio::main]
async fn main() -> Result<()> {
    // Your initialization code here
    
    HttpServer::new(move || {
        App::new()
            .wrap(your_middleware)
            .route("/your-routes", web::get().to(your_handlers))
            // .configure(distri_integration)  // When ready
    })
    .bind("127.0.0.1:3030")?
    .run()
    .await
}
```

### 3. Middleware Configuration
- CORS: Configure based on your security requirements
- Logging: Use tracing for structured logs
- Authentication: Add your auth middleware before distri routes

## Development Status

### Completed ✅
- [x] Project structure and build system
- [x] Basic actix-web server implementation
- [x] CORS and logging middleware setup
- [x] Custom route handlers (health, welcome)
- [x] Documentation and README
- [x] Clean error handling

### In Progress 🔄  
- [ ] Resolve distri module compilation issues (EventKind import)
- [ ] Integrate distri coordinator creation
- [ ] Add YAML configuration loading
- [ ] Implement full distri route embedding

### Planned 📋
- [ ] Authentication middleware example
- [ ] Database integration example  
- [ ] Custom agent implementations
- [ ] Production deployment guide
- [ ] Performance optimization
- [ ] Advanced configuration options

## Known Issues

1. **Distri Compilation**: Currently there's an import issue with `EventKind` in the distri module
   - Issue: `EventKind` not found in distri_a2a imports
   - Status: Being investigated and resolved
   - Workaround: Example works without distri integration for now

2. **Type Compatibility**: Some type mismatches in distri store implementation
   - Related to the EventKind import issue above

## Contributing

When adding distri integration:
1. Fix the EventKind import issue in `distri/src/store.rs`
2. Uncomment distri dependencies in `Cargo.toml`
3. Replace the placeholder main.rs with the full integration version
4. Test with the provided config.yaml

## Next Steps

1. **For Distri Maintainers**: Fix the EventKind export in distri_a2a
2. **For Users**: Use current example as template for actix-web embedding pattern
3. **For Contributors**: Help resolve the compilation issues and add features

---

This example provides a solid foundation for embedding any service in actix-web, with distri integration coming as soon as the compilation issues are resolved.