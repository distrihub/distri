//! Branded thinking/loading phrases shared between CLI and UI.

use serde::Deserialize;

const PHRASES_JSON: &str = include_str!("thinking_phrases.json");

#[derive(Debug, Clone, Deserialize)]
pub struct Phrase {
    pub text: String,
    pub emoji: String,
    pub icon: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Phrases {
    pub planning: Vec<Phrase>,
    pub replanning: Vec<Phrase>,
    pub thinking: Vec<Phrase>,
}

pub fn phrases() -> &'static Phrases {
    use std::sync::OnceLock;
    static PHRASES: OnceLock<Phrases> = OnceLock::new();
    PHRASES
        .get_or_init(|| serde_json::from_str(PHRASES_JSON).expect("invalid thinking_phrases.json"))
}

/// Pick a phrase from a category using an index (wraps around).
pub fn pick_planning(index: usize) -> &'static Phrase {
    let p = &phrases().planning;
    &p[index % p.len()]
}

pub fn pick_replanning(index: usize) -> &'static Phrase {
    let p = &phrases().replanning;
    &p[index % p.len()]
}

pub fn pick_thinking(index: usize) -> &'static Phrase {
    let p = &phrases().thinking;
    &p[index % p.len()]
}
