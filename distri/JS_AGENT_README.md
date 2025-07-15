# JavaScript Agent Implementation

This document describes the JavaScript agent implementation in distri, which follows a similar pattern to smolagents but uses JavaScript code generation and execution instead of Python.

## Overview

The JavaScript agent is a new type of agent that generates JavaScript code to perform actions instead of using traditional JSON-based tool calls. This approach allows the LLM to think in code and fix itself, providing more flexibility and control over the execution flow.

## Architecture

### Core Components

1. **JsAgent** (`src/coding/js_agent.rs`)
   - Implements the `BaseAgent` trait
   - Generates JavaScript code using LLM
   - Executes code in a sandboxed environment
   - Manages conversation flow and state

2. **JsExecutor** (`src/coding/executor.rs`)
   - Sandboxed JavaScript execution using rustyscript
   - Provides standard JavaScript functions and objects
   - Handles tool function registration
   - Manages variable persistence between executions

3. **JsToolRegistry** (`src/coding/js_tools.rs`)
   - Registers tools as JavaScript functions
   - Generates function schemas for LLM prompts
   - Provides tool descriptions and metadata

### Key Features

- **Code Generation**: LLM generates JavaScript code instead of JSON tool calls
- **Sandboxed Execution**: JavaScript code runs in a secure sandbox
- **Tool Integration**: Tools are available as JavaScript functions
- **State Management**: Variables persist between code executions
- **Error Handling**: Graceful error handling with try-catch blocks
- **Logging**: Console.log support for debugging

## Usage

### Configuration

Create a configuration file with the JavaScript agent:

```yaml
agents:
  - name: js_coding_agent
    description: "A JavaScript coding agent"
    agent_type: "js_agent"
    system_prompt: |
      You are a JavaScript coding agent. Write JavaScript code to solve problems.
    model_settings:
      model: "gpt-4o-mini"
      temperature: 0.1
      max_tokens: 2000
      max_iterations: 5
    mcp_servers:
      - name: filesystem
        filter:
          - "read_file"
          - "write_file"
```

### Code Generation Pattern

The LLM generates JavaScript code like this:

```javascript
try {
    // Your code here
    const result = someFunction(parameters);
    console.log('Result:', result);
    
    if (isFinalResult) {
        finalAnswer(result);
    } else {
        setOutput(result);
    }
} catch (error) {
    console.error('Error:', error);
    setOutput('Error: ' + error.message);
}
```

### Available Functions

- `console.log(...args)`: Log messages for debugging
- `console.error(...args)`: Log error messages
- `finalAnswer(value)`: Set the final answer and stop execution
- `setOutput(value)`: Set intermediate output
- `setVariable(name, value)`: Store variables for future use
- `JSON.stringify(value)`: Convert values to JSON strings
- `JSON.parse(text)`: Parse JSON strings
- Tool functions: All registered tools are available as functions

### Tool Integration

Tools are automatically registered as JavaScript functions. For example, if you have a `read_file` tool:

```javascript
const content = read_file(JSON.stringify({ path: "example.txt" }));
console.log('File content:', content);
```

## Implementation Details

### Function Schema Generation

The agent generates XML-style function schemas for the LLM:

```xml
<function name="read_file">
    <description>Read a file from the filesystem</description>
    <parameters>
        {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file"
                }
            },
            "required": ["path"]
        }
    </parameters>
</function>
```

### Sandboxed Execution

JavaScript code runs in a sandboxed environment with:
- Limited access to standard JavaScript functions
- No access to dangerous operations (file system, network, etc.)
- Controlled variable scope
- Error isolation

### State Management

Variables persist between code executions:
```javascript
// First execution
setVariable("user", "Alice");
setVariable("count", 0);

// Second execution
const user = global_variables.user;
const count = global_variables.count + 1;
setVariable("count", count);
```

## Comparison with smolagents

| Feature | smolagents (Python) | distri (JavaScript) |
|---------|-------------------|-------------------|
| Language | Python | JavaScript |
| Execution | AST-based interpreter | rustyscript sandbox |
| Tool Integration | Function objects | JavaScript functions |
| State Management | Global variables | Persistent variables |
| Error Handling | Exception handling | try-catch blocks |
| Code Generation | Python code | JavaScript code |

## Benefits

1. **Self-Correction**: LLM can fix code errors in subsequent iterations
2. **Flexibility**: Full JavaScript language features available
3. **Debugging**: Console.log support for better debugging
4. **State Persistence**: Variables maintain state across executions
5. **Tool Integration**: Seamless integration with existing tools
6. **Security**: Sandboxed execution prevents malicious code

## Limitations

1. **JavaScript Only**: Limited to JavaScript language features
2. **Sandbox Restrictions**: Some JavaScript features may be restricted
3. **Performance**: JavaScript execution may be slower than native tool calls
4. **Complexity**: More complex than traditional tool calling

## Future Enhancements

1. **TypeScript Support**: Add TypeScript compilation and type checking
2. **Module System**: Support for importing/exporting JavaScript modules
3. **Async/Await**: Better support for asynchronous operations
4. **Debugging Tools**: Enhanced debugging and inspection capabilities
5. **Performance Optimization**: Faster JavaScript execution
6. **Security Enhancements**: More granular security controls

## Testing

Run the JavaScript agent tests:

```bash
cargo test --features coding js_agent_test
```

## Example

Here's a complete example of using the JavaScript agent:

```rust
use distri::{
    coding::JsAgent,
    agent::ExecutorContext,
    memory::TaskStep,
    tools::LlmToolsRegistry,
    types::AgentDefinition,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create agent definition
    let definition = AgentDefinition {
        name: "js_agent".to_string(),
        description: "JavaScript coding agent".to_string(),
        agent_type: Some("js_agent".to_string()),
        // ... other fields
    };
    
    // Create tool registry
    let tool_registry = Arc::new(LlmToolsRegistry::new(HashMap::new()));
    
    // Create session store
    let session_store = Arc::new(Box::new(InMemorySessionStore::new()));
    
    // Create context
    let context = Arc::new(ExecutorContext::new(
        "thread_1".to_string(),
        None,
        true,
        None,
        None,
        None,
    ));
    
    // Create JavaScript agent
    let agent = JsAgent::new(definition, tool_registry, session_store, context)?;
    
    // Execute task
    let task = TaskStep {
        task: "Calculate the sum of numbers from 1 to 10".to_string(),
        id: "task_1".to_string(),
    };
    
    let result = agent.invoke(task, None, context, None).await?;
    println!("Result: {}", result);
    
    Ok(())
}
```

This implementation provides a powerful alternative to traditional tool calling, allowing agents to think and work in code while maintaining security and control.