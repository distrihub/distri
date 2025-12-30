# Reflection System

The Distri reflection system provides automated post-execution analysis and retry capabilities for agents, enabling self-improvement and quality assurance.

## Overview

The reflection system allows agents to automatically analyze their own execution history and determine if their responses need improvement. When issues are identified, the system can trigger a retry with specific feedback for improvement. Reflection artifacts (feedback transcripts, retry metadata) are persisted through the shared filesystem rooted at `CURRENT_WORKING_DIR`, so running `distri-server` against `examples/` means all review data lives inside `examples/filesystem/` alongside editable agents/files.

## Architecture

### Components

1. **Reflection Agent** (`distri/src/agent/reflection/reflection_agent.md`)
   - Specialized subagent for analyzing execution history
   - Evaluates response quality, completeness, and accuracy
   - Provides structured feedback and retry recommendations

2. **Agent Loop Integration** (`distri/src/agent/agent_loop.rs`)
   - Triggers reflection after task completion
   - Parses reflection recommendations
   - Implements retry mechanism when needed

3. **Configuration Flag** (`distri-types/src/agent.rs`)
   - `enable_reflection: Option<bool>` in `StandardDefinition`
   - Helper method `is_reflection_enabled()` (defaults to false)

## How It Works

### Execution Flow

1. **Normal Execution**: Agent executes task and provides response
2. **Reflection Trigger**: If `enable_reflection = true`, reflection agent launches
3. **Analysis**: Reflection agent analyzes:
   - Execution history and scratchpad
   - Final result quality and completeness
   - Tool usage patterns and efficiency
   - Error patterns and missed opportunities
4. **Decision**: Reflection agent determines:
   - "Should Continue: YES" → Triggers retry
   - "Should Continue: NO" → Execution complete
5. **Retry (if needed)**: System clears final result and restarts execution

### Retry Mechanism

When reflection recommends retry:
- Final result is cleared from context
- Execution restarts with reflection feedback in history
- Loop prevention: Reflection only runs once per execution
- Recursive execution handled with boxed futures

## Configuration

### Enable Reflection

Add to any agent's TOML frontmatter:

```toml
---
name = "my_agent"
description = "Agent with reflection enabled"
enable_reflection = true
# ... other config
---
```

### Reflection Agent Settings

The reflection agent uses:
- **Model**: `gpt-4.1-mini` (focused analysis)
- **Temperature**: 0.2 (consistent evaluation)
- **Max Tokens**: 1500 (detailed analysis)
- **Max Iterations**: 5 (quick analysis)

## Analysis Criteria

The reflection agent evaluates responses across multiple dimensions:

### Quality Assessment
- **Accuracy**: Factual correctness
- **Completeness**: Coverage of requirements
- **Clarity**: Clear communication
- **Relevance**: Addressing the actual question

### Execution Analysis
- **Tool Usage**: Appropriate tool selection and usage
- **Step Efficiency**: Unnecessary or redundant steps
- **Error Patterns**: Recurring failures or issues
- **Alternative Approaches**: Better strategies available

## Output Format

The reflection agent provides structured analysis:

```
**Key Insights:**
- [Specific observations about execution]

**Final Result Assessment:**
- Quality: [Excellent/Good/Fair/Poor]
- Completeness: [Complete/Partial/Incomplete]  
- Issues Found: [List specific problems]

**Improvement Recommendations:**
- [Actionable suggestions]

**Retry Recommendation:**
- Should Continue: [YES/NO]
- Reason: [Why retry is needed or not]
- Specific Improvements Needed: [Focus areas for retry]

**Overall Assessment:**
[Summary of execution quality]
```

## Use Cases

### Quality Assurance
- Automatically catch incomplete or inaccurate responses
- Ensure responses meet complexity requirements
- Verify tool usage is appropriate

### Self-Improvement
- Learn from execution patterns
- Identify recurring issues
- Optimize response strategies

### Complex Tasks
- Multi-step workflows that need validation
- Research tasks requiring comprehensive coverage
- Creative tasks needing quality verification

## Implementation Details

### Loop Prevention
- Checks execution history for existing reflection results
- Prevents infinite reflection loops
- Single reflection per execution cycle

### Context Isolation
- Reflection runs in separate context
- Original final results preserved
- Reflection analysis stored separately

### Error Handling
- Both success and failure cases handled
- Reflection failures don't break main execution
- Graceful degradation when orchestrator unavailable

## Testing

### Basic Reflection Test
```bash
# Test with reflection enabled
cargo run --bin distri run test_reflection --task "What is 2+2?" --verbose
```

### Complex Task (May Trigger Retry)
```bash  
# Test with demanding task
cargo run --bin distri run test_reflection --task "Write a comprehensive analysis of quantum computing with detailed technical explanations" --verbose
```

### Monitor Reflection Events
```bash
# Watch for reflection triggers
export RUST_LOG=info
cargo run --bin distri run test_reflection --task "Tell me about Paris" --verbose 2>&1 | grep -E "(🤔|🔄|Should Continue)"
```

## Configuration Examples

### Research Agent with Reflection
```markdown
---
name = "research_agent"
description = "Research agent with quality assurance"
enable_reflection = true
max_iterations = 10

[model_settings]
model = "gpt-4.1-mini"
temperature = 0.3

[tools]
builtin = ["final", "search", "web_fetch"]
---

# Research Agent

You are a research agent that provides comprehensive analysis...
[Reflection will automatically verify research quality]
```

### Creative Agent with Reflection  
```markdown
---
name = "creative_writer"
description = "Creative writing agent with self-review"
enable_reflection = true

[strategy.execution_mode]
type = "tools"
---

# Creative Writer

You create engaging content...
[Reflection will assess creativity and completeness]
```

## Performance Considerations

- **Overhead**: Adds ~1-3 seconds per execution for analysis
- **Token Usage**: Additional ~500-1500 tokens for reflection
- **Retry Cost**: Doubles execution time/tokens when retry triggered
- **Accuracy**: Significantly improves response quality and completeness

## Best Practices

1. **Enable Selectively**: Use for quality-critical tasks
2. **Clear Instructions**: Provide detailed task requirements
3. **Monitor Logs**: Watch for reflection triggers and decisions
4. **Tune Thresholds**: Adjust reflection agent criteria as needed
5. **Test Scenarios**: Verify reflection works for your use cases

## Troubleshooting

### Reflection Not Triggering
- Verify `enable_reflection = true` in agent config
- Check agent logs for reflection status messages
- Ensure orchestrator is available in context

### Infinite Loops
- System prevents multiple reflections per execution
- Check for reflection results in execution history
- Monitor step count and iteration limits

### Quality Issues
- Adjust reflection agent criteria
- Review reflection analysis output
- Tune model temperature and max tokens

## Future Enhancements

- Configurable reflection criteria
- Multiple reflection strategies
- Learning from reflection history
- Integration with agent training pipelines
