# distri

Rust client for the Distri A2A agent platform. Use it to invoke agents,
stream responses over SSE, and handle tool calls, connect MCPs and much more. 
Check out https://distri.dev/ for further information.

## Install

```toml
[dependencies]
distri = "0.2.4"
```

## Quick start

```rust
use distri::Distri;
use distri_types::Message;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Uses DISTRI_BASE_URL and DISTRI_API_KEY if set.
    let client = Distri::from_env();

    let messages = vec![Message::user("Write a short haiku about Rust.".into(), None)];
    let replies = client.invoke("my-agent", &messages).await?;

    for reply in replies {
        if let Some(text) = reply.as_text() {
            println!("{text}");
        }
    }

    Ok(())
}
```

## Streaming responses

```rust
use distri::Distri;
use distri_types::Message;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Distri::new();
    let messages = vec![Message::user("Stream the response.".into(), None)];

    client
        .invoke_stream("my-agent", &messages, |item| async move {
            if let Some(message) = item.message {
                if let Some(text) = message.as_text() {
                    println!("{text}");
                }
            }
        })
        .await?;

    Ok(())
}
```

## Handle tool calls locally

```rust
use distri::{AgentStreamClient, Distri, DistriConfig, ExternalToolRegistry};
use distri_types::{AgentEvent, ToolCall, ToolResponse};
use serde_json::json;

let registry = ExternalToolRegistry::new();
registry.register("my-agent", "echo", |call: ToolCall, _event: AgentEvent| async move {
    Ok(ToolResponse::direct(
        call.tool_call_id,
        call.tool_name,
        json!({ "echo": call.input }),
    ))
});

let stream = AgentStreamClient::from_config(DistriConfig::default()).with_tool_registry(registry);
let client = Distri::new().with_stream_client(stream);
```

## Configuration

`Distri::from_env()` and `DistriConfig::from_env()` read:

- `DISTRI_BASE_URL` (defaults to `https://api.distri.dev`)
- `DISTRI_API_KEY` (optional)

You can also create a `~/.distri/config` file:

```toml
base_url = "https://api.distri.dev"
api_key = "your-api-key"
```

## Related crates

- `distri-types` for message, tool, and config types
- `distri-a2a` for the A2A protocol primitives
- `distri-filesystem` for tool implementations

## License

MIT
