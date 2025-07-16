# Code Agent Sample

This sample demonstrates a custom agent that can reason and execute JavaScript/TypeScript code, similar to smolagents but integrated with the Distri framework.

## Overview

The CodeAgent is an AI agent that combines LLM reasoning with code execution capabilities. It can:

1. **Analyze tasks** and determine the best approach (code execution, LLM reasoning, or hybrid)
2. **Generate and execute JavaScript/TypeScript code** in a sandboxed environment using Deno
3. **Validate code syntax** and structure
4. **Analyze code complexity** and provide suggestions
5. **Use both code and natural language reasoning** to solve problems

## Features

### Reasoning Modes

The CodeAgent supports three reasoning modes:

- **Hybrid** (default): Uses both code execution and LLM reasoning
- **Code Only**: Prioritizes code execution for problem solving
- **LLM Only**: Uses traditional LLM reasoning

### Code Execution

- **Sandboxed Environment**: Code runs in a secure Deno sandbox
- **Context Injection**: Variables can be passed to the code execution environment
- **Function Definitions**: Custom functions can be defined and used
- **Error Handling**: Graceful fallback to LLM reasoning if code execution fails

### Code Analysis

- **Syntax Validation**: Basic syntax checking for JavaScript/TypeScript
- **Complexity Analysis**: Measures code complexity and provides suggestions
- **Structure Analysis**: Analyzes code structure and provides recommendations

## Prerequisites

1. **Deno**: The code execution requires Deno to be installed
   ```bash
   curl -fsSL https://deno.land/x/install/install.sh | sh
   ```

2. **Environment Variables**: Set up your OpenAI API key
   ```bash
   export OPENAI_API_KEY="your-api-key-here"
   ```

## Usage

### Running the Sample

```bash
cd samples/code-agent
cargo run
```

### Using Different Agent Types

The sample includes three different agent configurations:

1. **code-agent**: Default hybrid reasoning
2. **code-agent-hybrid**: Explicit hybrid reasoning
3. **code-agent-code-only**: Code-first approach

### Example Tasks

The sample demonstrates various types of tasks:

- **Computational Tasks**: Factorial calculation, prime number generation
- **Algorithmic Tasks**: Sorting functions, complexity analysis
- **Explanatory Tasks**: Programming concepts, best practices

## Configuration

The agents are configured in `definition.yaml`:

```yaml
agents:
  - name: "code-agent"
    description: "An AI agent that can reason and execute JavaScript/TypeScript code"
    agent_type: "code_agent"
    system_prompt: |
      You are a CodeAgent, an AI assistant that can reason and execute JavaScript/TypeScript code.
      # ... detailed prompt ...
    model_settings:
      model: "gpt-4o-mini"
      temperature: 0.1
      max_tokens: 2000
```

## Architecture

### Components

1. **CodeAgent**: Main agent implementation that orchestrates reasoning and execution
2. **CodeExecutor**: Handles code execution in the sandboxed environment
3. **JsSandbox**: Manages the Deno sandbox for secure code execution
4. **CodeValidator**: Validates code syntax and structure
5. **CodeAnalyzer**: Analyzes code complexity and provides suggestions

### Integration

The CodeAgent integrates seamlessly with the existing Distri framework:

- Uses the standard agent factory pattern
- Implements the `BaseAgent` trait
- Supports all existing agent features (streaming, events, etc.)
- Can be used with any existing tools and MCP servers

## Customization

### Adding Custom Functions

You can add custom functions to the CodeExecutor:

```rust
use distri::agent::code::{CodeExecutor, FunctionDefinition};

let mut executor = CodeExecutor::new(context);
executor.add_function(
    FunctionDefinition::new("custom_function".to_string())
        .with_description("A custom function".to_string())
        .with_parameters(serde_json::json!({
            "type": "object",
            "properties": {
                "param": {"type": "string"}
            }
        }))
);
```

### Custom Reasoning Modes

You can create custom reasoning modes by extending the `ReasoningMode` enum and implementing the corresponding logic in the `CodeAgent`.

## Security

- **Sandboxed Execution**: All code runs in a Deno sandbox with restricted permissions
- **Timeout Protection**: Code execution is limited by configurable timeouts
- **Error Isolation**: Code execution errors don't affect the agent's stability
- **Context Validation**: Input validation prevents malicious code injection

## Limitations

- **Deno Dependency**: Requires Deno to be installed on the system
- **Basic Validation**: Current syntax validation is basic (could be enhanced with proper parsers)
- **Limited Sandbox**: The Deno sandbox provides basic isolation but could be enhanced
- **No Persistent State**: Code execution is stateless between calls

## Future Enhancements

- **Enhanced Code Analysis**: Integration with proper JavaScript/TypeScript parsers
- **Advanced Sandboxing**: More sophisticated security measures
- **Code Caching**: Cache frequently used code snippets
- **Multi-language Support**: Support for other programming languages
- **Interactive Debugging**: Step-through debugging capabilities