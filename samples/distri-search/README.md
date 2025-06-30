# DeepSearch Agent

An intelligent research agent that combines web search and scraping capabilities to provide comprehensive answers to user queries.

## Overview

DeepSearch is a custom agent built on the distri framework that demonstrates how to:
- Implement a `CustomAgent` that uses multiple MCP tools
- Coordinate between search and scraping operations
- Maintain state across agent execution steps
- Parse and synthesize information from multiple sources

## Features

- **Web Search Integration**: Uses mcp-tavily for intelligent web search
- **Content Scraping**: Uses mcp-spider for detailed content extraction
- **Multi-step Processing**: Automatically searches first, then scrapes top results
- **Structured Output**: Provides organized, well-formatted responses with source citations

## Architecture

The DeepSearch agent follows a multi-step execution pattern:

1. **Search Phase**: Uses Tavily API to find relevant web sources
2. **Scraping Phase**: Extracts detailed content from top-ranked sources  
3. **Synthesis Phase**: Combines search results and scraped content into comprehensive response

## Prerequisites

To run DeepSearch, you need:

1. **MCP Servers**: 
   - `mcp-tavily` (for web search)
   - `mcp-spider` (for web scraping)

2. **API Keys**:
   - Tavily API key (set as `TAVILY_API_KEY` environment variable)

3. **MCP Server Binaries**:
   - Build or install the mcp-tavily and mcp-spider servers from the [mcp-servers repository](https://github.com/distrihub/mcp-servers)

## Installation

1. **Clone and build the mcp-servers**:
   ```bash
   git clone https://github.com/distrihub/mcp-servers
   cd mcp-servers
   cargo build --release
   ```

2. **Set environment variables**:
   ```bash
   export TAVILY_API_KEY="your_tavily_api_key"
   export PATH="$PATH:/path/to/mcp-servers/target/release"
   ```

3. **Build the DeepSearch sample**:
   ```bash
   cd samples/distri-search
   cargo build --release
   ```

## Usage

### Running as Standalone

```bash
cd samples/distri-search
cargo run
```

This will run a test scenario with a predefined query.

### Using with Configuration File

```bash
# Start with the configuration file
distri-cli run --config deep-search-config.yaml
```

### Integrating in Code

```rust
use distri_search::DeepSearchAgent;
use distri::types::{AgentDefinition, AgentRecord, McpDefinition};

// Create the agent
let deep_search_agent = DeepSearchAgent::new();

// Create agent definition
let agent_def = AgentDefinition {
    name: "deep_search".to_string(),
    description: "Research agent with search and scrape".to_string(),
    mcp_servers: vec![
        McpDefinition {
            name: "mcp-tavily".to_string(),
            filter: ToolsFilter::All,
            r#type: McpServerType::Tool,
        },
        McpDefinition {
            name: "mcp-spider".to_string(),
            filter: ToolsFilter::All,
            r#type: McpServerType::Tool,
        },
    ],
    // ... other configuration
};

// Register as runnable agent
let agent_record = AgentRecord::Runnable(agent_def, Box::new(deep_search_agent));
```

## Configuration

The `deep-search-config.yaml` file demonstrates how to configure:

- Agent definition with system prompts
- MCP server connections (mcp-tavily and mcp-spider)
- Environment variables for API keys
- Tool filtering and parameters

## Agent Behavior

When given a query, DeepSearch:

1. **Extracts the search query** from user messages
2. **Performs web search** using Tavily API to find relevant sources
3. **Ranks results** by relevance score
4. **Scrapes top sources** (configurable, default 3) for detailed content
5. **Synthesizes information** into a structured response with:
   - Search overview with source summaries
   - Detailed analysis from scraped content
   - Source citations and relevance scores

## Example Output

```markdown
# DeepSearch Results for: What are the latest developments in AI model alignment?

## Search Overview
Found 5 relevant sources:

1. **AI Alignment Research Progress 2024** (Score: 0.95)
   - Latest research on constitutional AI and RLHF improvements
   - Source: https://example.com/ai-alignment-2024

2. **Constitutional AI Advances** (Score: 0.87)
   - New techniques for training helpful, harmless, and honest AI systems
   - Source: https://anthropic.com/research/constitutional-ai

## Detailed Analysis

### AI Alignment Research Progress 2024
**Source:** https://example.com/ai-alignment-2024

Recent advances in AI alignment include improved constitutional AI methods,
better reward modeling techniques, and novel approaches to scalable oversight...

---

## Summary

Based on the search results and detailed content analysis above, I've provided
a comprehensive overview of the latest AI alignment developments...
```

## Customization

You can customize DeepSearch by:

- **Modifying search parameters**: Adjust `max_sources` in the configuration
- **Changing scraping behavior**: Modify `select_urls_for_scraping` logic
- **Customizing output format**: Update `generate_comprehensive_response`
- **Adding filters**: Implement content filtering or source validation
- **Extending tool usage**: Add support for additional MCP tools

## Troubleshooting

Common issues:

1. **MCP servers not found**: Ensure mcp-tavily and mcp-spider are in PATH
2. **API key errors**: Verify TAVILY_API_KEY is set correctly
3. **Search failures**: Check network connectivity and API quotas
4. **Scraping timeouts**: Some sites may block or slow down scraping requests

## Contributing

This is a sample implementation. To contribute:

1. Fork the repository
2. Create your feature branch
3. Add tests for new functionality
4. Submit a pull request

## License

This sample follows the same license as the main distri project.