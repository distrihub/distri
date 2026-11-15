//! One record per provider call: the [`LlmCallRecorder`] every executor opens
//! before it calls a provider and closes exactly once after.
//!
//! `begin` asks the orchestrator's [`SpendPolicy`](distri_types::SpendPolicy)
//! first; a refusal is itself recorded (status `blocked`) and returned as an
//! error. `finish_ok` / `finish_err` add the provider's usage to the run's
//! totals and emit [`AgentEventType::LlmCall`]. A recorder dropped without
//! either (an early `?`, a stream the caller abandoned) records an `error`
//! call from `Drop`, so no path skips the ledger and none writes twice.

use std::sync::Arc;
use std::time::Instant;

use distri_types::{
    LlmCallStatus, LlmCallUsage, Message, MessageRole, Part, RequestPart, RequestPartKind,
    SpendCheck, ToolDefinition, USAGE_LIMIT_REACHED,
};

use crate::agent::context::ExecutorContext;
use crate::agent::types::AgentEventType;
use crate::AgentError;

/// Token counts from a provider's usage report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProviderUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub cache_creation_tokens: u64,
    pub reasoning_tokens: u64,
}

impl ProviderUsage {
    /// Anthropic reports `input_tokens` without the prompt-cache reads and
    /// writes; the ledger's `input_tokens` is the whole prompt for every
    /// provider, so the three are added here.
    pub fn anthropic(input: u32, output: u32, cache_read: u32, cache_creation: u32) -> Self {
        Self {
            input_tokens: input as u64 + cache_read as u64 + cache_creation as u64,
            output_tokens: output as u64,
            cached_tokens: cache_read as u64,
            cache_creation_tokens: cache_creation as u64,
            reasoning_tokens: 0,
        }
    }
}

/// What the request held, kept until the call ends so parts are counted off
/// the request path.
#[derive(Default)]
struct RequestSnapshot {
    messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
}

pub struct LlmCallRecorder {
    ctx: Arc<ExecutorContext>,
    call: LlmCallUsage,
    start: Instant,
    snapshot: Option<RequestSnapshot>,
    done: bool,
}

impl LlmCallRecorder {
    /// Open a call. Fails (after recording a `blocked` call) when the spend
    /// policy refuses it.
    pub async fn begin(
        ctx: Arc<ExecutorContext>,
        model: &str,
        provider: &str,
        streamed: bool,
        purpose: &str,
        messages: &[Message],
        tools: Vec<ToolDefinition>,
    ) -> Result<Self, AgentError> {
        let call = LlmCallUsage {
            call_id: uuid::Uuid::new_v4().to_string(),
            started_at: chrono::Utc::now(),
            model: model.to_string(),
            provider: provider.to_string(),
            streamed,
            purpose: purpose.to_string(),
            agent_version: ctx.agent_version.clone(),
            parent_run_id: ctx.parent_run_id.clone(),
            caller: ctx.caller.clone(),
            ..Default::default()
        };
        let mut rec = Self {
            ctx,
            call,
            start: Instant::now(),
            snapshot: Some(RequestSnapshot {
                messages: messages.to_vec(),
                tools,
            }),
            done: false,
        };

        let policy = rec
            .ctx
            .orchestrator
            .as_ref()
            .and_then(|o| o.spend_policy.clone());
        if let Some(policy) = policy {
            let refusal = policy
                .check_call(SpendCheck {
                    workspace_id: rec.ctx.workspace_id.as_deref(),
                    user_id: &rec.ctx.user_id,
                    agent_id: &rec.ctx.agent_id,
                    model,
                    caller: rec.ctx.caller.as_ref(),
                })
                .await;
            if let Err(msg) = refusal {
                let msg = if msg.starts_with(USAGE_LIMIT_REACHED) {
                    msg
                } else {
                    format!("{USAGE_LIMIT_REACHED}: {msg}")
                };
                rec.close(LlmCallStatus::Blocked, Some(msg.clone()), None)
                    .await;
                return Err(AgentError::LLMError(msg));
            }
        }
        Ok(rec)
    }

    /// The call id, for logs.
    pub fn call_id(&self) -> &str {
        &self.call.call_id
    }

    pub async fn finish_ok(mut self, usage: Option<ProviderUsage>) {
        self.close(LlmCallStatus::Ok, None, usage).await;
    }

    pub async fn finish_err(mut self, error: impl ToString, usage: Option<ProviderUsage>) {
        self.close(LlmCallStatus::Error, Some(error.to_string()), usage)
            .await;
    }

    async fn close(
        &mut self,
        status: LlmCallStatus,
        error: Option<String>,
        usage: Option<ProviderUsage>,
    ) {
        if self.done {
            return;
        }
        self.done = true;
        let call = self.fill(status, error, usage);
        if let Some(u) = usage {
            self.ctx
                .increment_usage_with_cache(
                    u.input_tokens as u32,
                    u.output_tokens as u32,
                    u.cached_tokens as u32,
                )
                .await;
        }
        let snapshot = self.snapshot.take().unwrap_or_default();
        emit_with_parts(self.ctx.clone(), call, snapshot).await;
    }

    fn fill(
        &self,
        status: LlmCallStatus,
        error: Option<String>,
        usage: Option<ProviderUsage>,
    ) -> LlmCallUsage {
        let mut call = self.call.clone();
        call.status = status;
        call.error = error;
        call.duration_ms = self.start.elapsed().as_millis() as u64;
        if let Some(u) = usage {
            call.provider_reported = true;
            call.input_tokens = u.input_tokens as i64;
            call.output_tokens = u.output_tokens as i64;
            call.cached_tokens = u.cached_tokens as i64;
            call.cache_creation_tokens = u.cache_creation_tokens as i64;
            call.reasoning_tokens = u.reasoning_tokens as i64;
            call.total_tokens = (u.input_tokens + u.output_tokens) as i64;
            call.cost_usd = crate::agent::pricing::exact_cost(
                &call.model,
                u.input_tokens,
                u.output_tokens,
                u.cached_tokens,
            );
        }
        call
    }
}

impl Drop for LlmCallRecorder {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        let call = self.fill(
            LlmCallStatus::Error,
            Some("the call ended before the provider replied".to_string()),
            None,
        );
        let ctx = self.ctx.clone();
        let snapshot = self.snapshot.take().unwrap_or_default();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(emit_with_parts(ctx, call, snapshot));
        }
    }
}

async fn emit_with_parts(ctx: Arc<ExecutorContext>, mut call: LlmCallUsage, snap: RequestSnapshot) {
    let reported = call.provider_reported.then_some(call.input_tokens);
    let counted =
        tokio::task::spawn_blocking(move || request_parts(&snap.messages, &snap.tools, reported))
            .await
            .unwrap_or_default();
    call.parts = counted;
    call.tokenizer = TOKENIZER.to_string();
    ctx.emit(AgentEventType::LlmCall { call }).await;
}

/// The BPE the parts are counted with. Exact for current OpenAI models; an
/// estimate for other providers (their tokenizers are not public).
pub const TOKENIZER: &str = "o200k_base";

fn count(text: &str) -> i64 {
    use std::sync::OnceLock;
    static BPE: OnceLock<Option<tiktoken_rs::CoreBPE>> = OnceLock::new();
    match BPE.get_or_init(|| tiktoken_rs::o200k_base().ok()) {
        Some(bpe) => bpe.encode_ordinary(text).len() as i64,
        // No tokenizer: ~4 characters a token.
        None => (text.len() as i64 + 3) / 4,
    }
}

fn part_text(part: &Part) -> String {
    match part {
        Part::Text(t) => t.clone(),
        Part::ToolCall(c) => serde_json::to_string(c).unwrap_or_default(),
        Part::ToolResult(r) => r.parts.iter().map(part_text).collect::<Vec<_>>().join("\n"),
        Part::Data(v) => v.to_string(),
        // Images and files are billed by the provider in their own way; the
        // overhead part absorbs them.
        _ => String::new(),
    }
}

/// Count a request by part and reconcile it with the provider's input total:
/// `overhead` takes the difference, or, when the counted parts exceed the
/// total, the parts are scaled down so they still sum to it.
pub fn request_parts(
    messages: &[Message],
    tools: &[ToolDefinition],
    provider_input: Option<i64>,
) -> Vec<RequestPart> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<RequestPartKind, (i64, u32)> = BTreeMap::new();
    let mut add = |kind: RequestPartKind, tokens: i64| {
        let e = acc.entry(kind).or_insert((0, 0));
        e.0 += tokens;
        e.1 += 1;
    };

    if !tools.is_empty() {
        for t in tools {
            let schema = serde_json::json!({
                "name": t.name, "description": t.description, "parameters": t.parameters,
            });
            add(RequestPartKind::Tools, count(&schema.to_string()));
        }
    }

    let last_user = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::User));
    for (i, m) in messages.iter().enumerate() {
        let mut plain = String::new();
        for p in &m.parts {
            if let Part::ToolResult(r) = p {
                let kind = if r.tool_name == "load_skill" {
                    RequestPartKind::Skills
                } else {
                    RequestPartKind::ToolResults
                };
                add(kind, count(&part_text(p)));
            } else {
                plain.push_str(&part_text(p));
                plain.push('\n');
            }
        }
        if plain.trim().is_empty() {
            continue;
        }
        let kind = match m.role {
            MessageRole::System | MessageRole::Developer => RequestPartKind::System,
            MessageRole::Tool => RequestPartKind::ToolResults,
            MessageRole::User if Some(i) == last_user => RequestPartKind::UserMessage,
            _ => RequestPartKind::History,
        };
        add(kind, count(&plain));
    }

    let mut parts: Vec<RequestPart> = acc
        .into_iter()
        .map(|(kind, (tokens, items))| RequestPart {
            kind,
            tokens,
            items,
        })
        .collect();

    if let Some(total) = provider_input {
        let counted: i64 = parts.iter().map(|p| p.tokens).sum();
        if counted <= total {
            parts.push(RequestPart {
                kind: RequestPartKind::Overhead,
                tokens: total - counted,
                items: 0,
            });
        } else if counted > 0 {
            // Scale down, then give the rounding remainder to the largest part.
            for p in parts.iter_mut() {
                p.tokens = p.tokens * total / counted;
            }
            let rest = total - parts.iter().map(|p| p.tokens).sum::<i64>();
            if let Some(p) = parts.iter_mut().max_by_key(|p| p.tokens) {
                p.tokens += rest;
            }
        }
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: MessageRole, text: &str) -> Message {
        Message {
            role,
            parts: vec![Part::Text(text.to_string())],
            ..Default::default()
        }
    }

    #[test]
    fn parts_sum_to_the_provider_total() {
        let messages = vec![
            msg(MessageRole::System, "You are a helpful assistant."),
            msg(MessageRole::User, "Earlier question about the weather"),
            msg(MessageRole::Assistant, "It was sunny."),
            msg(MessageRole::User, "And tomorrow?"),
        ];
        let parts = request_parts(&messages, &[], Some(500));
        assert_eq!(parts.iter().map(|p| p.tokens).sum::<i64>(), 500);
        let kinds: Vec<_> = parts.iter().map(|p| p.kind).collect();
        assert!(kinds.contains(&RequestPartKind::System));
        assert!(kinds.contains(&RequestPartKind::History));
        assert!(kinds.contains(&RequestPartKind::UserMessage));
        assert!(kinds.contains(&RequestPartKind::Overhead));
    }

    #[test]
    fn parts_over_the_total_are_scaled_to_it() {
        let messages = vec![msg(MessageRole::User, &"word ".repeat(200))];
        let parts = request_parts(&messages, &[], Some(10));
        assert_eq!(parts.iter().map(|p| p.tokens).sum::<i64>(), 10);
    }

    #[test]
    fn skill_results_and_tool_schemas_are_their_own_parts() {
        let skill = Message {
            role: MessageRole::Tool,
            parts: vec![Part::ToolResult(distri_types::ToolResponse {
                tool_call_id: "c1".into(),
                tool_name: "load_skill".into(),
                parts: vec![Part::Text("# Skill body".into())],
                parts_metadata: None,
            })],
            ..Default::default()
        };
        let tool = ToolDefinition {
            name: "search".into(),
            description: "Search the web".into(),
            parameters: serde_json::json!({"type": "object"}),
            examples: None,
            output_schema: None,
            prompt: None,
        };
        let parts = request_parts(&[skill], &[tool], None);
        let kinds: Vec<_> = parts.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, vec![RequestPartKind::Tools, RequestPartKind::Skills]);
        assert!(parts.iter().all(|p| p.tokens > 0));
    }
}
