//! `distri::run` — shared entry point used by `distri-cli` (Commands::Run) and
//! server-side runners (e.g. cloud's `LocalProcessRemoteRunner`). Both go
//! through the same `MessageSendParams` construction so anything tested via
//! one is effectively tested via the other.
//!
//! The shape is split into two halves so the CLI can slot in its
//! `inject_external_tools` step between building and streaming:
//!
//! - [`build_run_params`] — async; does the connections fetch + params build.
//! - [`stream_run`]       — drives the SSE stream via `AgentStreamClient`.
//!
//! Callers that don't need to inject anything between the two halves can use
//! the convenience [`run_agent`] that chains them.
//!
//! ```ignore
//! // CLI usage (needs inject_external_tools between build + stream):
//! let mut params = build_run_params(&platform_client, &opts).await;
//! app.inject_external_tools(&mut params)?;
//! stream_run(&stream_client, &agent_name, params, on_event).await?;
//!
//! // Server-side runner usage (no injection):
//! run_agent(&platform_client, &stream_client, opts, on_event).await?;
//! ```

use std::collections::HashMap;

use distri_a2a::MessageSendParams;

use crate::client_stream::StreamError;
use crate::message::{build_connections_context, build_message_params_full};
use crate::{AgentStreamClient, Distri, StreamItem};

/// Default agent invoked when callers don't specify one.
pub const DEFAULT_RUN_AGENT: &str = "distri_runner";

/// Parameters that match what `distri run …` parses from its CLI args.
/// Both the CLI and server-side in-process runners build one of these and
/// hand it to the shared `run_agent` / `build_run_params` entry points.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Agent name. Defaults to [`DEFAULT_RUN_AGENT`] when None.
    pub agent: Option<String>,
    /// Task prompt text.
    pub task: String,
    /// Explicit task_id for this execution. Falls back to `DISTRI_TASK_ID`
    /// env when None.
    pub task_id: Option<String>,
    /// Explicit thread_id (a.k.a. context_id). Falls back to
    /// `DISTRI_THREAD_ID` env when None.
    pub thread_id: Option<String>,
    /// True when the call should run remotely (server forks to sandbox /
    /// in-process runner via runtime-constraint dispatch).
    pub remote: bool,
    /// Model override. Shows up in `definition_overrides.model`.
    pub model: Option<String>,
    /// Env vars to pass through the `ExecutorContext`. Merged with secrets
    /// when the caller builds them.
    pub env_vars: Option<HashMap<String, String>>,
    /// When true, skip listing connections (saves an extra HTTP round-trip).
    /// The server-side runner sets this because the server already knows
    /// about workspace connections — shipping them back up the wire is
    /// pointless.
    pub skip_connections_context: bool,
    /// Arbitrary tags forwarded to the server in `ExecutorContextMetadata.tags`.
    /// Recorded on the agent span and merged into the thread's attributes.
    pub tags: Option<HashMap<String, String>>,
    /// Inbound distributed-trace context forwarded in
    /// `ExecutorContextMetadata.trace_context` for remote-parent propagation.
    pub trace_context: Option<distri_types::TraceContext>,
}

/// Resolve the agent name for a run, defaulting to [`DEFAULT_RUN_AGENT`].
pub fn resolve_agent_name(opts: &RunOptions) -> String {
    opts.agent
        .clone()
        .unwrap_or_else(|| DEFAULT_RUN_AGENT.to_string())
}

/// Async half of `run_agent`: fetch optional connections context and build
/// the `MessageSendParams`. The CLI calls this, injects its external tool
/// schemas into `params.metadata.external_tools`, and then calls
/// [`stream_run`]. Non-CLI callers (server-side runners) typically call
/// [`run_agent`] which chains both halves.
///
/// Thread/task IDs fall back to `DISTRI_THREAD_ID` / `DISTRI_TASK_ID` env
/// vars when absent from `opts` — matching the CLI's behavior.
pub async fn build_run_params(platform_client: &Distri, opts: &RunOptions) -> MessageSendParams {
    let connections_context = if opts.skip_connections_context {
        None
    } else {
        build_connections_context(platform_client).await
    };

    let effective_thread = opts
        .thread_id
        .clone()
        .or_else(|| std::env::var("DISTRI_THREAD_ID").ok());
    let effective_task = opts
        .task_id
        .clone()
        .or_else(|| std::env::var("DISTRI_TASK_ID").ok());

    build_message_params_full(
        opts.task.clone(),
        effective_thread.as_deref(),
        effective_task.as_deref(),
        opts.model.as_deref(),
        opts.remote,
        connections_context,
        opts.env_vars.clone(),
        opts.tags.clone(),
        opts.trace_context.clone(),
    )
}

/// Stream an agent run using the supplied already-built params. Thin wrapper
/// over [`AgentStreamClient::stream_agent`] — provided for symmetry with
/// [`build_run_params`] so callers can compose the two halves explicitly.
pub async fn stream_run<F, Fut>(
    stream_client: &AgentStreamClient,
    agent_name: &str,
    params: MessageSendParams,
    on_event: F,
) -> Result<(), StreamError>
where
    F: FnMut(StreamItem) -> Fut,
    Fut: std::future::Future<Output = ()> + Send,
{
    stream_client
        .stream_agent(agent_name, params, on_event)
        .await
}

/// Convenience: build + stream in one call. Used by server-side in-process
/// runners and any non-CLI caller that doesn't need to inject external tool
/// schemas between the two halves.
///
/// The caller is responsible for registering any external tools on the
/// stream client beforehand (the CLI registers local shell/fs tools;
/// server-side runners register none — the server already has the tools).
pub async fn run_agent<F, Fut>(
    platform_client: &Distri,
    stream_client: &AgentStreamClient,
    opts: RunOptions,
    on_event: F,
) -> Result<(), StreamError>
where
    F: FnMut(StreamItem) -> Fut,
    Fut: std::future::Future<Output = ()> + Send,
{
    let agent_name = resolve_agent_name(&opts);
    let params = build_run_params(platform_client, &opts).await;
    stream_run(stream_client, &agent_name, params, on_event).await
}
