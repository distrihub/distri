# Distri Server Embedding Example

This example demonstrates how to embed the Distri A2A server routes into your own actix-web application.


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

