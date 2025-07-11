# Agent Handover Demonstration

This document demonstrates the agent handover mechanism in the Distri platform, showing how multiple agents can work together by transferring control to each other.

## Overview

The agent handover feature allows agents to transfer control to other agents when they encounter tasks that require different capabilities or expertise. This creates a collaborative multi-agent system where agents can work together to solve complex problems.

## Key Features

### 1. **Sub-Agents Configuration**
- Each agent can specify a list of `sub_agents` that they're allowed to transfer control to
- This provides security and prevents unauthorized handovers
- Agents can only transfer to agents in their `sub_agents` list

### 2. **Transfer Tool**
- Built-in `transfer_to_agent` tool available to all agents
- Allows agents to transfer control with a reason
- Emits `AgentHandover` events for monitoring

### 3. **Event Tracking**
- Handover events are emitted when transfers occur
- Events include `from_agent`, `to_agent`, and `reason` fields
- Enables monitoring and debugging of agent collaboration

## Configuration Example

```yaml
agents:
  - name: "twitter-bot"
    description: "Social media analysis agent"
    system_prompt: |
      You are TwitterBot. When you need web research beyond Twitter,
      transfer to the "distri-search" agent using transfer_to_agent.
    mcp_servers:
      - name: "twitter"
    sub_agents:
      - "distri-search"  # Can transfer to search agent

  - name: "distri-search"
    description: "Web research agent"
    system_prompt: |
      You are DistriSearch. After completing research, transfer back
      to "twitter-bot" for final social media analysis.
    mcp_servers:
      - name: "search"
      - name: "scrape"
    sub_agents:
      - "twitter-bot"  # Can transfer back to twitter agent
```

## How It Works

### 1. **Agent Registration**
```rust
// Register agents with their sub_agents configuration
executor.register_default_agent(twitter_agent_def).await?;
executor.register_default_agent(search_agent_def).await?;
```

### 2. **Transfer Execution**
When an agent calls `transfer_to_agent`:
```rust
// Agent uses the transfer tool
transfer_to_agent({
    "agent_name": "distri-search",
    "reason": "Need comprehensive web research"
})
```

### 3. **Handover Process**
1. Transfer tool validates the target agent exists
2. Checks if target agent is in the `sub_agents` list
3. Sends `CoordinatorMessage::HandoverAgent` message
4. Coordinator emits `AgentHandover` event
5. Control transfers to the target agent

## Testing the Handover

### Running the Test
```bash
# Run the handover test
cargo test test_agent_handover_back_and_forth -- --nocapture

# Run all handover tests
cargo test handover_test -- --nocapture
```

### Test Scenarios

#### 1. **Back-and-Forth Handover**
- Start with twitter-bot
- Task requires both Twitter analysis and web research
- Twitter-bot transfers to distri-search for research
- Distri-search transfers back to twitter-bot for final analysis
- Verifies multiple handovers work correctly

#### 2. **Sub-Agents Validation**
- Tests that agents can only transfer to agents in their `sub_agents` list
- Verifies proper configuration is enforced

#### 3. **Error Handling**
- Tests handover to non-existent agents
- Verifies system robustness and error handling

## Sample Task Flow

### Complex Task Example
```
Task: "Analyze recent OpenAI buzz on Twitter and provide comprehensive analysis"

1. twitter-bot starts
   - Checks Twitter for OpenAI mentions
   - Realizes it needs comprehensive background research
   - Transfers to distri-search with reason: "Need background research on OpenAI"

2. distri-search takes over
   - Searches web for latest OpenAI news
   - Scrapes detailed articles
   - Gathers comprehensive information
   - Transfers back to twitter-bot with reason: "Research complete, need social media analysis"

3. twitter-bot receives control again
   - Combines Twitter data with research findings
   - Provides final comprehensive analysis
   - Task completes with both social media and web research insights
```

## Implementation Details

### Agent Configuration
```rust
let twitter_agent_def = AgentDefinition {
    name: "twitter-bot".to_string(),
    system_prompt: Some(twitter_prompt),
    mcp_servers: vec![
        McpDefinition {
            name: "twitter".to_string(),
            r#type: McpServerType::Tool,
        }
    ],
    sub_agents: vec!["distri-search".to_string()], // Key configuration
    ..Default::default()
};
```

### Event Handling
```rust
// Capture handover events
let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(100);

// Listen for handover events
while let Some(event) = event_rx.recv().await {
    match &event.event {
        AgentEventType::AgentHandover { from_agent, to_agent, reason } => {
            println!("🔄 HANDOVER: {} -> {}, reason: {:?}", from_agent, to_agent, reason);
        }
        _ => {}
    }
}
```

### Transfer Tool Usage
```rust
// In agent system prompt
"When you need different capabilities, use transfer_to_agent:
{
    \"agent_name\": \"target-agent\",
    \"reason\": \"Why the transfer is needed\"
}"
```

## Best Practices

### 1. **Clear Transfer Criteria**
- Define specific scenarios when to transfer
- Include clear examples in system prompts
- Explain the capabilities of each agent

### 2. **Proper Sub-Agent Configuration**
- Only include agents that make sense for handover
- Consider security implications
- Document the allowed transfer patterns

### 3. **Meaningful Transfer Reasons**
- Provide context for why the transfer is happening
- Help with debugging and monitoring
- Improve system transparency

### 4. **Circular Handover Prevention**
- Design agent capabilities to complement each other
- Avoid infinite transfer loops
- Set appropriate max_iterations limits

## Monitoring and Debugging

### Event Tracking
```rust
// Track all handover events
let handover_events = Arc::new(Mutex::new(Vec::new()));

// Process events
match event.event {
    AgentEventType::AgentHandover { from_agent, to_agent, reason } => {
        println!("Handover: {} -> {} ({})", from_agent, to_agent, reason.unwrap_or("No reason"));
    }
    _ => {}
}
```

### Logging
```rust
// Enable detailed logging
init_logging("info");

// Logs will show:
// - Agent handover requests
// - Transfer tool executions
// - Event emissions
// - Task completions
```

## Example Output

```
🚀 Starting complex task that requires agent handover...
🔄 HANDOVER: twitter-bot -> distri-search, reason: Some("Need comprehensive web research about OpenAI")
🔄 HANDOVER: distri-search -> twitter-bot, reason: Some("Research complete, need social media analysis")
🏁 Task completed
🎉 Agent handover test completed successfully!
   - 2 handover events captured
   - Result length: 1847 characters
✅ First handover verified: twitter-bot -> distri-search
✅ Second handover verified: distri-search -> twitter-bot
```

## Configuration File

Use the provided `handover-test-config.yaml` file to see a complete working configuration with:
- Two agents configured for handover
- Proper sub_agents setup
- Clear system prompts explaining when to transfer
- Mock MCP servers for testing

## Benefits

1. **Specialization**: Each agent can focus on their expertise
2. **Collaboration**: Agents work together on complex tasks
3. **Flexibility**: Dynamic task routing based on requirements
4. **Modularity**: Easy to add new agents with specific capabilities
5. **Monitoring**: Full visibility into agent interactions

The handover mechanism enables building sophisticated multi-agent systems where agents collaborate seamlessly to solve complex problems that require multiple types of expertise.