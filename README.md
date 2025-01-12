
```bash
curl -X GET http://localhost:8080/agents

curl -X POST http://localhost:8080/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "My Agent",
    "description": "A helpful AI assistant",
    "model": "gpt-4",
    "provider_name": "openai",
    "tools": {"tool1": {"enabled": true}},
    "model_settings": {"temperature": 0.7},
    "prompt": "You are a helpful assistant"
  }'
  ```