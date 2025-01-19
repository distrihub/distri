```json
{
  "mcpServers": {
    "twitter-mcp": {
      "command": "/Users/vivek/projects/distri/distri-agents/target/debug/examples/server",
      "args": [],
      "env": {}
    }
  }
}
```

### Example
```bash
cat << 'EOF' | cargo run --bin twitter
{"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "get_timeline", "arguments": {"session_string": "your_session_here", "count": 1}}, "id": 1}
EOF
```
