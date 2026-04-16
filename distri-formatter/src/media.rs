//! Utilities for decoding media payloads from agent events.
//!
//! `BrowserScreenshot.image` and `MediaGenerated.data` are both base64-encoded
//! strings (possibly with a `data:…;base64,` prefix). Channel formatters need
//! raw bytes before handing them to platform APIs like Telegram's `sendPhoto`.

use base64::{Engine as _, engine::general_purpose};

/// Decode a base64-encoded media payload to raw bytes.
///
/// Handles:
/// - Plain base64 (`"iVBORw0KGgo…"`)
/// - Data-URI prefixed (`"data:image/png;base64,iVBORw0KGgo…"`)
/// - Whitespace-padded payloads
///
/// Returns `None` if decoding fails (caller should log and skip).
pub fn decode_base64_media(encoded: &str) -> Option<Vec<u8>> {
    let trimmed = encoded.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Strip data-URI prefix if present.
    let payload = if let Some(idx) = trimmed.find("base64,") {
        &trimmed[idx + "base64,".len()..]
    } else if let Some(idx) = trimmed.find(',') {
        &trimmed[idx + 1..]
    } else {
        trimmed
    };

    // Remove any whitespace (line breaks in long payloads).
    let sanitized: String = payload.split_whitespace().collect();

    general_purpose::STANDARD.decode(sanitized.as_bytes()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_plain_base64() {
        // "hello" in base64
        let result = decode_base64_media("aGVsbG8=").unwrap();
        assert_eq!(result, b"hello");
    }

    #[test]
    fn decode_data_uri() {
        let result = decode_base64_media("data:image/png;base64,aGVsbG8=").unwrap();
        assert_eq!(result, b"hello");
    }

    #[test]
    fn decode_with_whitespace() {
        let result = decode_base64_media("aGVs\nbG8=").unwrap();
        assert_eq!(result, b"hello");
    }

    #[test]
    fn empty_returns_none() {
        assert!(decode_base64_media("").is_none());
        assert!(decode_base64_media("   ").is_none());
    }
}
