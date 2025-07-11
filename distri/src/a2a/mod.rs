mod handler;
mod stream;
use distri_a2a::{JsonRpcError, Message, Part};
pub use handler::A2AHandler;
use serde::{Deserialize, Serialize};
pub mod mapper;

fn unimplemented_error(method: &str) -> JsonRpcError {
    JsonRpcError {
        code: -32601,
        message: format!("Method not implemented: {}", method),
        data: None,
    }
}

fn extract_text_from_message(message: &Message) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(text_part) => Some(text_part.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SseMessage {
    pub event: Option<String>,
    pub data: String,
}

impl SseMessage {
    pub fn new(event: Option<String>, data: String) -> Self {
        Self { event, data }
    }

    pub fn data(&self) -> Self {
        Self {
            event: self.event.clone(),
            data: serde_json::to_string(&self).unwrap(),
        }
    }
}
