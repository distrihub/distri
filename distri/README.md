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

## Session Store API

The Distri client provides a comprehensive session store API for managing thread-scoped key-value storage. Session values can be used to store state, share data between agent iterations, and attach additional content to user messages.

### Basic Session Operations

```rust
use distri::Distri;
use serde_json::json;

let client = Distri::from_env();
let session_id = "thread-123";

// Set a session value
client.set_session_value(
    session_id,
    "user_preference",
    json!({ "theme": "dark", "language": "en" }),
    None, // Optional expiry ISO timestamp
).await?;

// Get a single session value
let value = client.get_session_value(session_id, "user_preference").await?;
println!("User preference: {:?}", value);

// Get all session values as a HashMap
let all_values = client.get_session_values(session_id).await?;
for (key, value) in all_values {
    println!("{}: {:?}", key, value);
}

// Delete a specific key
client.delete_session_value(session_id, "user_preference").await?;

// Clear all values in a session
client.clear_session(session_id).await?;
```

### Prefixed User Parts

For granular control, use the prefixed user parts API. Any session value with the `__user_part_` prefix is automatically included in user messages:

```rust
use distri::Distri;
use distri_types::Part;

let client = Distri::from_env();
let session_id = "thread-123";

// Set a named user part (automatically prefixed with __user_part_)
client.set_user_part(
    session_id,
    "observation", // Name for this part
    Part::Text("The user clicked the submit button".to_string()),
).await?;

// Set a text user part (convenience method)
client.set_user_part_text(
    session_id,
    "screenshot_description",
    "Screenshot shows the login form with validation errors",
).await?;

// Set an image user part (with automatic gzip compression)
client.set_user_part_image(
    session_id,
    "screenshot",
    distri_types::FileType::Bytes {
        bytes: base64_image_string,
        mime_type: "image/png".to_string(),
        name: Some("screenshot.png".to_string()),
    },
).await?;

// Delete a specific user part
client.delete_user_part(session_id, "observation").await?;

// Clear all user parts
client.clear_user_parts(session_id).await?;
```

### Session Value Expiry

Session values can optionally have an expiry time:

```rust
use chrono::Utc;

let expiry = Utc::now() + chrono::Duration::hours(24);
client.set_session_value(
    session_id,
    "temporary_data",
    json!({ "data": "value" }),
    Some(&expiry.to_rfc3339()),
).await?;
```

### Use Cases

- **Browser Automation**: Store screenshots, DOM observations, and user interactions
- **State Management**: Maintain conversation context and user preferences
- **Tool Integration**: Share data between external tools and agent iterations
- **Multi-step Workflows**: Persist intermediate results across agent calls

## Related crates

- `distri-types` for message, tool, and config types
- `distri-a2a` for the A2A protocol primitives
- `distri-filesystem` for tool implementations

## License

MIT
