# DeepSearch Agent Examples

This directory contains **two complete examples** demonstrating different approaches to building intelligent research agents with the distri framework.

## Overview

Both examples implement a **DeepSearch agent** that combines web search and content scraping to provide comprehensive research answers. The key difference is **how** they implement the multi-step workflow:

| Example | Type | Orchestration | Configuration | Best For |
|---------|------|--------------|---------------|----------|
| **YAML Agent** | Standard Agent | LLM-driven | YAML file | Quick prototypes, flexible reasoning |
| **Custom Agent** | CustomAgent | Code-driven | Rust implementation | Complex workflows, deterministic logic |

## Examples

### 1. 🔍 YAML-based Standard Agent (`yaml_agent_example`)

Uses the **built-in distri Agent** (`Agent::new_local`) with YAML configuration.

**How it works:**
- Agent loads configuration from `deep-search-agent.yaml`
- LLM receives system prompt with tool descriptions
- LLM decides when and how to call search/scrape tools
- Built-in executor handles tool orchestration automatically

**Key files:**
- `deep-search-agent.yaml` - Agent configuration
- `src/bin/yaml_agent_example.rs` - Example runner

**Run:**
```bash
cargo run --bin yaml_agent_example
```

**Features:**
- ✅ YAML-driven configuration
- ✅ LLM-powered tool decision making
- ✅ Built-in tool execution handling
- ✅ No custom code required
- ✅ Flexible reasoning patterns

### 2. 🤖 Custom Agent Implementation (`custom_agent_example`)

Implements the **CustomAgent trait** with explicit workflow logic.

**How it works:**
- Agent implements `CustomAgent::step()` method
- Explicit 3-phase workflow: Search → Scrape → Synthesize
- Analyzes conversation history to track state
- Programmatically generates tool calls
- Full control over execution flow

**Key files:**
- `src/bin/custom_agent_example.rs` - Complete implementation

**Run:**
```bash
cargo run --bin custom_agent_example
```

**Features:**
- ✅ Implements `CustomAgent` trait
- ✅ Multi-step workflow management
- ✅ Conversation state analysis
- ✅ Dynamic tool call generation
- ✅ Deterministic execution logic
- ✅ Full programmatic control

## Prerequisites

### For Full Functionality (with live MCP servers):

1. **Install MCP servers:**
   ```bash
   # Clone and build mcp-servers
   git clone https://github.com/distrihub/mcp-servers
   cd mcp-servers
   cargo build --release
   
   # Add to PATH
   export PATH="$PATH:/path/to/mcp-servers/target/release"
   ```

2. **Set API keys:**
   ```bash
   export TAVILY_API_KEY="your_tavily_api_key"
   ```

3. **Update YAML config** (for YAML example):
   - Ensure `deep-search-agent.yaml` points to correct MCP server binaries
   - Verify environment variable placeholders

### For Testing/Demo (without MCP servers):

Both examples include **mock implementations** that demonstrate the workflow patterns without requiring external dependencies.

## Architecture Comparison

### YAML Agent Architecture
```
User Query → YAML Config → Standard Agent → LLM Executor → Tool Calls → Response
                ↑                           ↓
            System Prompt              Tool Responses
```

**Advantages:**
- Simple configuration
- No Rust code required
- LLM handles complex reasoning
- Easy to modify prompts

**Limitations:**
- Less predictable execution
- Limited workflow control
- Dependent on LLM reasoning quality

### Custom Agent Architecture
```
User Query → CustomAgent::step() → Workflow State Analysis → Tool Calls → Response
                ↑                        ↓
            Rust Logic              Phase Management
```

**Advantages:**
- Deterministic workflow execution
- Full programmatic control
- Complex multi-step patterns
- Efficient state management

**Limitations:**
- Requires Rust implementation
- More complex to build
- Less flexible than LLM reasoning

## Technical Implementation Details

### YAML Agent Implementation

```yaml
agents:
  - name: "deep_search"
    system_prompt: |
      You are DeepSearch with access to web search and scraping tools.
      1. First, use 'search' to find relevant sources
      2. Then, use 'scrape' to extract detailed content  
      3. Finally, synthesize comprehensive answers
    mcp_servers:
      - name: "mcp-tavily"
      - name: "mcp-spider"
```

The agent uses `Agent::new_local()` and relies on the built-in `LLMExecutor` for tool orchestration.

### Custom Agent Implementation

```rust
#[async_trait]
impl CustomAgent for DeepSearchCustomAgent {
    async fn step(&self, messages, params, context, session_store) -> Result<StepResult, AgentError> {
        let state = self.analyze_workflow_state(messages);
        let query = self.extract_user_query(messages)?;

        // Phase 1: Search
        if !state.search_requested {
            return Ok(StepResult::ToolCalls(vec![self.create_search_tool_call(&query)]));
        }

        // Phase 2: Scrape  
        if state.search_completed && !state.scrape_requested {
            let urls = self.extract_urls_from_search(&state.search_results);
            return Ok(StepResult::ToolCalls(self.create_scrape_tool_calls(&urls)));
        }

        // Phase 3: Synthesize
        if state.search_completed {
            return Ok(StepResult::Finish(self.synthesize_response(&query, &state)));
        }

        Ok(StepResult::Continue(vec![]))
    }
}
```

The agent implements explicit workflow logic and uses `AgentRecord::Runnable()` for registration.

## Running the Examples

### YAML Agent Example
```bash
cd samples/distri-search
cargo run --bin yaml_agent_example
```

**Expected output:**
```
🔍 DeepSearch Agent - YAML Configuration Example
================================================

✅ Configuration loaded successfully
✅ Infrastructure initialized  
✅ DeepSearch agent registered

🤖 Testing DeepSearch Agent
==========================
Query: What are the latest developments in artificial intelligence safety research?

🔄 Executing task...
✅ Task completed successfully!
```

### Custom Agent Example  
```bash
cd samples/distri-search
cargo run --bin custom_agent_example
```

**Expected output:**
```
🤖 DeepSearch Agent - Custom Agent Example
==========================================

✅ Infrastructure initialized
✅ Custom agent registered

🔬 Testing Custom DeepSearch Agent
=================================  
Query: What are the key challenges in AI alignment research?

🚀 Executing custom agent workflow...
✅ Custom agent execution completed!

🎯 Example Summary
=================
• ✅ Implementing the CustomAgent trait
• ✅ Multi-step workflow management
• ✅ Conversation state analysis
• ✅ Dynamic tool call generation
```

## Configuration Files

### deep-search-agent.yaml

Complete YAML configuration for the standard agent approach:

- **Agent definition** with system prompt and tool mappings
- **MCP server configurations** for mcp-tavily and mcp-spider
- **Environment variable** placeholders for API keys
- **Server settings** for distri infrastructure

## Use Cases

### YAML Agent is ideal for:
- **Rapid prototyping** of research workflows
- **Non-technical configuration** by domain experts
- **Flexible reasoning** patterns that adapt to different queries
- **Educational examples** and demonstrations
- **Simple deployment** scenarios

### Custom Agent is ideal for:
- **Production workflows** requiring deterministic behavior
- **Complex multi-step processes** with specific business logic
- **Performance-critical** applications
- **Integration** with existing Rust codebases
- **Advanced state management** requirements

## Extending the Examples

### Adding New Tools
1. **YAML approach**: Add to `mcp_servers` section in YAML
2. **Custom approach**: Extend tool call creation methods

### Modifying Workflow
1. **YAML approach**: Update system prompt instructions
2. **Custom approach**: Modify `step()` method logic

### Adding State Persistence
1. **YAML approach**: Configure session store in YAML
2. **Custom approach**: Use `session_store` parameter in `step()`

## Troubleshooting

**Common issues:**

1. **"deep_search agent not found"**: Ensure YAML file is in working directory
2. **"Tool execution failed"**: Check MCP server installation and API keys
3. **"Max iterations reached"**: Increase `max_iterations` in configuration
4. **Compilation errors**: Verify distri framework dependency versions

**For YAML Agent:**
- Check YAML syntax and agent name matching
- Verify MCP server binary paths
- Ensure environment variables are set

**For Custom Agent:**
- Verify CustomAgent trait implementation
- Check tool call JSON format
- Debug state analysis logic

## Performance Considerations

### YAML Agent
- **Pros**: Fast to develop and modify
- **Cons**: LLM overhead for each decision

### Custom Agent  
- **Pros**: Efficient execution, predictable performance
- **Cons**: Development overhead for complex logic

## Conclusion

These examples demonstrate the **flexibility of the distri framework** in supporting both configuration-driven and code-driven agent development approaches. Choose the approach that best fits your use case:

- **YAML Agent** for rapid development and flexible reasoning
- **Custom Agent** for production workflows and deterministic control

Both approaches integrate seamlessly with the distri ecosystem and can be deployed using the same infrastructure.