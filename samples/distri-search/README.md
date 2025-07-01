# DeepSearch Agent - Simple Example

A simplified example of building a research agent with Distri that combines web search and scraping capabilities.

## Overview

This example demonstrates how to create an intelligent research agent using only YAML configuration (no custom code). The agent automatically:

1. **Search**: Uses web search to find relevant sources
2. **Scrape**: Extracts detailed content from promising URLs  
3. **Synthesize**: Combines information into comprehensive responses

## Quick Start

1. **Install dependencies**:
   ```bash
   cd samples/distri-search
   cargo build
   ```

2. **Set up environment** (optional):
   ```bash
   export TAVILY_API_KEY="your-api-key"  # For better search results
   ```

3. **Run the example**:
   ```bash
   cargo run --bin simple-search
   ```

## How It Works

### YAML Configuration
The agent is defined entirely in `deep-search.yaml`:
- **Agent Definition**: Name, description, system prompt
- **Tools**: Configured to use `web_search` and `crawl` tools
- **Model Settings**: Uses GPT-4o-mini with appropriate limits

### Infrastructure Setup
The `lib.rs` handles:
- Loading the embedded YAML configuration
- Registering MCP servers (mcp-tavily, mcp-spider) 
- Setting up the distri coordinator

### Execution Flow
1. Load agent configuration from YAML
2. Register agent with distri coordinator
3. Submit user query as a task
4. Agent automatically orchestrates search → scrape → synthesis workflow
5. Return comprehensive research results

## Key Features

- ✅ **Zero custom code** - Pure YAML configuration
- ✅ **Automatic workflow** - LLM handles tool orchestration
- ✅ **Built-in tools** - Web search and scraping ready to use
- ✅ **Flexible prompts** - Easy to customize agent behavior
- ✅ **Distri integration** - Uses full framework capabilities

## Files

- `deep-search.yaml` - Agent configuration
- `src/lib.rs` - Infrastructure setup helpers
- `src/bin/simple.rs` - Example execution
- `Cargo.toml` - Dependencies and build config

## Next Steps

To customize the agent:
1. Modify the system prompt in `deep-search.yaml`
2. Adjust model settings (temperature, max_tokens, etc.)
3. Add more MCP tools as needed
4. Experiment with different search/scrape parameters

This simple example shows the power of Distri's YAML-based agent configuration!