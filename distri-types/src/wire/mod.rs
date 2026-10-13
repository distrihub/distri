//! LLM provider wire formats — the request/response shapes spoken on the
//! network, independent of which direction they travel.
//!
//! These types started life as outbound-only client structs in
//! `llm-gateway::claude_client` (request `Serialize`, response `Deserialize`).
//! Distri Cloud's gateway ingress reads the *same* shapes in the other
//! direction — a client POSTs a Messages request to us and we write a Messages
//! response back — so every type here derives both `Serialize` and
//! `Deserialize` and neither side owns the definition.
//!
//! Two dialects live here:
//!
//! - [`anthropic`] — the Anthropic Messages API (`POST /v1/messages`), also
//!   spoken by Anthropic-compatible endpoints such as Z.ai's coding plan.
//! - [`openai`] — the OpenAI Chat Completions API
//!   (`POST /v1/chat/completions`), the Responses API (`POST /v1/responses`),
//!   and the model-listing envelope (`GET /v1/models`) that every
//!   OpenAI-compatible client expects.

pub mod anthropic;
pub mod openai;
