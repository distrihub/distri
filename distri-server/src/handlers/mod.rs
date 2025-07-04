mod message_stream;

use distri_a2a::{Message, Part};
pub use message_stream::*;

pub fn extract_text_from_message(message: &Message) -> String {
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
