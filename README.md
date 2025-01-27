# API Examples

## Configuration
Copy configuration to 
```bash
cp config/config.example.yaml config/config.yaml
```

## Authentication

First, login to get your access token:

```bash
# Login and get bearer token
curl -X POST http://localhost:8080/auth/login \
  -H "Content-Type: application/json" \
  -d '{
    "username": "user@example.com", 
    "password": "your_password"
  }'

# Sample response:
# {
#   "token": "eyJhbGciOiJIUzI1NiIs..."
# }
```

## Working with Agents 

Use the bearer token in subsequent requests:

```bash
# Get profile
curl -X GET http://localhost:8080/api/profile \
  -H "Authorization: Bearer 24550843-675d-4b41-90a9-d89d4209d16e"

# Get list of agents
curl -X GET http://localhost:8080/api/agents \
  -H "Authorization: Bearer 24550843-675d-4b41-90a9-d89d4209d16e"

# Create a new agent
curl -X POST http://localhost:8080/api/agents \
  -H "Authorization: Bearer YOUR_TOKEN_HERE" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "My Agent",
    "description": "A helpful AI assistant",
    "model": "gpt-4o-mini",
    "provider_name": "openai",
    "tools": {"tool1": {"enabled": true}},
    "model_settings": {"temperature": 0.7},
    "prompt": "You are a helpful assistant"
  }'

# Get a specific agent
curl -X GET http://localhost:8080/agents/123 \
  -H "Authorization: Bearer YOUR_TOKEN_HERE"
```

## File System Tools

```bash
cat << 'EOF' |  npx -y @modelcontextprotocol/server-filesystem .
{"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_directory", "arguments": {"path": "."}}, "id": 1}
EOF
```

### Workflow

- Analyse Profile
- Get Trends 
- Question/Answer
- Activity
  - Frequency