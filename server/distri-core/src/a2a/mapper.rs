//! Agent events → A2A 1.0 stream frames.
//!
//! Two modes, chosen by the request's activated extensions:
//!
//! - **standard** (no `event-stream`): plain A2A only. `statusUpdate` for
//!   state changes (working, input-required with the pending calls,
//!   auth-required, the terminal state with the agent's answer as the status
//!   message), `artifactUpdate` for the agent's streamed text (one artifact
//!   per text message: `append` after the first chunk, `lastChunk` at its
//!   end) and for produced artifacts, and the final `Task`.
//! - **event-stream**: every distri event is a `statusUpdate` whose
//!   `metadata` is the distri event envelope (`type`, `agent_id`,
//!   `parent_task_id`, child keys, `step_id`, `attempt`, and `final: true` on
//!   the root run's end). An event that belongs to an extension the request
//!   did not activate falls back to its standard frames. Artifacts are
//!   `artifactUpdate`s in both modes.

use std::collections::{HashMap, HashSet};

use distri_a2a::{
    Artifact, Message, Metadata, Part, Role, StreamResponse, TaskArtifactUpdateEvent, TaskState,
    TaskStatus, TaskStatusUpdateEvent,
};
use distri_types::a2a_extensions::{
    stamp_created_at, ActiveExtensions, DistriExtension, PART_TYPE_KEY,
};
use distri_types::{AgentEventEnvelope, ToolCall};
use serde_json::Value;

use crate::agent::{AgentEvent, AgentEventType};

/// `RunError.code` for a run that needs the user to connect or authorize
/// something (maps to `auth-required`).
pub use distri_types::RUN_ERROR_AUTH_REQUIRED;

/// Name of the artifact that carries the agent's streamed text.
pub const RESPONSE_ARTIFACT_NAME: &str = "response";

/// Whether the event ends the root run (and the stream).
pub fn is_root_terminal(event: &AgentEvent) -> bool {
    matches!(
        &event.event,
        AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. }
    ) && event.parent_task_id.is_none()
}

/// The A2A state an event leaves its task in.
pub fn state_for(event: &AgentEvent) -> TaskState {
    match &event.event {
        AgentEventType::RunFinished { success, .. } => {
            if *success {
                TaskState::Completed
            } else {
                TaskState::Failed
            }
        }
        AgentEventType::RunError { code, .. } => match code.as_deref() {
            Some("CANCELLED") => TaskState::Canceled,
            Some(c) if c == RUN_ERROR_AUTH_REQUIRED => TaskState::AuthRequired,
            _ => TaskState::Failed,
        },
        AgentEventType::RunParked { .. } => TaskState::InputRequired,
        _ => TaskState::Working,
    }
}

/// The envelope as status-update metadata, with `step_id` (when the event
/// variant does not carry its own) and `final` (root end) added.
pub fn envelope_metadata(event: &AgentEvent, step_id: Option<&str>, is_final: bool) -> Metadata {
    let envelope = AgentEventEnvelope::from_event(event);
    let mut meta = match serde_json::to_value(&envelope) {
        Ok(Value::Object(m)) => m,
        _ => Metadata::new(),
    };
    if let Some(step) = step_id {
        meta.entry("step_id")
            .or_insert_with(|| Value::String(step.to_string()));
    }
    if is_final {
        meta.insert("final".into(), Value::Bool(true));
    }
    meta
}

/// A distri artifact part as an A2A artifact.
pub fn artifact_from_file(file: &distri_types::filesystem::FileMetadata) -> Artifact {
    let name = file.original_filename.clone().or_else(|| {
        file.relative_path
            .rsplit('/')
            .next()
            .map(String::from)
            .filter(|s| !s.is_empty())
    });
    let mut part = Part::data(serde_json::to_value(file).unwrap_or_default())
        .with_metadata(PART_TYPE_KEY, Value::String("artifact".into()));
    if let Some(ct) = &file.content_type {
        part = part.with_media_type(ct.clone());
    }
    if let Some(n) = &name {
        part = part.with_filename(n.clone());
    }
    let mut parts = vec![part];
    if let Some(preview) = file.preview.as_ref().filter(|p| !p.is_empty()) {
        parts.push(Part::text(preview.clone()));
    }
    Artifact {
        artifact_id: file.file_id.clone(),
        name,
        description: None,
        parts,
        metadata: None,
        extensions: vec![],
    }
}

/// Maps one request's events to frames. Stateful: it tracks the current
/// step per task, which text messages already started, the tool calls a
/// park may wait on, and the artifacts produced so far.
pub struct StreamMapper {
    task_id: String,
    context_id: String,
    active: ActiveExtensions,
    steps: HashMap<String, String>,
    text_started: HashSet<String>,
    tool_calls: HashMap<String, ToolCall>,
    artifact_ids: HashSet<String>,
    /// Artifacts produced so far (for the final `Task`).
    pub artifacts: Vec<Artifact>,
}

impl StreamMapper {
    pub fn new(
        task_id: impl Into<String>,
        context_id: impl Into<String>,
        active: ActiveExtensions,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            context_id: context_id.into(),
            active,
            steps: HashMap::new(),
            text_started: HashSet::new(),
            tool_calls: HashMap::new(),
            artifact_ids: HashSet::new(),
            artifacts: Vec::new(),
        }
    }

    /// The `event-stream` extension is active.
    pub fn event_stream(&self) -> bool {
        self.active.is_active(DistriExtension::EventStream)
    }

    pub fn extensions(&self) -> &ActiveExtensions {
        &self.active
    }

    fn status(&self, task_id: &str, state: TaskState, message: Option<Message>) -> StreamResponse {
        StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
            task_id: task_id.to_string(),
            context_id: self.context_id.clone(),
            status: TaskStatus {
                state,
                message,
                timestamp: Some(chrono::Utc::now()),
            },
            metadata: None,
        })
    }

    fn agent_message(&self, parts: Vec<Part>) -> Message {
        let mut message = Message {
            message_id: uuid::Uuid::new_v4().to_string(),
            context_id: Some(self.context_id.clone()),
            task_id: Some(self.task_id.clone()),
            role: Role::Agent,
            parts,
            ..Default::default()
        };
        self.stamp(&mut message, chrono::Utc::now());
        message
    }

    /// With `event-stream` active, set `metadata["distri/created_at"]` on a
    /// message (G1). Standard mode leaves the message as it is.
    pub fn stamp(&self, message: &mut Message, at: chrono::DateTime<chrono::Utc>) {
        if self.event_stream() {
            stamp_created_at(&mut message.metadata, at);
        }
    }

    /// `TaskArtifactUpdateEvent.metadata`: the time, with `event-stream`.
    fn artifact_metadata(&self, at: chrono::DateTime<chrono::Utc>) -> Option<Metadata> {
        let mut meta = None;
        if self.event_stream() {
            stamp_created_at(&mut meta, at);
        }
        meta
    }

    /// Tool calls a later park may wait on that this mapper did not see as
    /// `ToolCalls` events (they were emitted before the request subscribed).
    pub fn remember_calls(&mut self, calls: impl IntoIterator<Item = ToolCall>) {
        for c in calls {
            self.tool_calls.entry(c.tool_call_id.clone()).or_insert(c);
        }
    }

    /// Whether the event goes out as an envelope (else as standard frames).
    pub fn envelope_active(&self, event: &AgentEvent) -> bool {
        let ext = DistriExtension::for_event_type(event.event.wire_type());
        self.event_stream() && ext.is_none_or(|e| self.active.is_active(e))
    }

    /// Frames for one event.
    pub fn map(&mut self, event: &AgentEvent) -> Vec<StreamResponse> {
        // Track what later events need.
        match &event.event {
            AgentEventType::StepStarted { step_id, .. } => {
                self.steps.insert(event.task_id.clone(), step_id.clone());
            }
            AgentEventType::ToolCalls { tool_calls, .. } => {
                for c in tool_calls {
                    self.tool_calls.insert(c.tool_call_id.clone(), c.clone());
                }
            }
            _ => {}
        }

        let mut frames = Vec::new();
        if self.envelope_active(event) {
            let step = self.steps.get(&event.task_id).cloned();
            let is_final = is_root_terminal(event);
            frames.push(StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
                task_id: event.task_id.clone(),
                context_id: event.thread_id.clone(),
                status: TaskStatus {
                    state: state_for(event),
                    message: None,
                    timestamp: Some(event.timestamp),
                },
                metadata: Some(envelope_metadata(event, step.as_deref(), is_final)),
            }));
        } else {
            frames.extend(self.standard_frames(event));
        }
        frames.extend(self.artifact_frames(event));
        frames
    }

    /// Plain-A2A frames for an event (root run only).
    fn standard_frames(&mut self, event: &AgentEvent) -> Vec<StreamResponse> {
        if event.parent_task_id.is_some() {
            return Vec::new();
        }
        let task_id = event.task_id.clone();
        match &event.event {
            AgentEventType::RunStarted {} => vec![self.status(&task_id, TaskState::Working, None)],
            AgentEventType::TextMessageContent {
                message_id, delta, ..
            } => {
                let first = self.text_started.insert(message_id.clone());
                vec![StreamResponse::ArtifactUpdate(TaskArtifactUpdateEvent {
                    task_id,
                    context_id: self.context_id.clone(),
                    artifact: Artifact {
                        artifact_id: message_id.clone(),
                        name: Some(RESPONSE_ARTIFACT_NAME.to_string()),
                        parts: vec![Part::text(delta.clone())],
                        ..Default::default()
                    },
                    append: Some(!first),
                    last_chunk: Some(false),
                    metadata: self.artifact_metadata(event.timestamp),
                })]
            }
            AgentEventType::TextMessageEnd { message_id, .. }
                if self.text_started.contains(message_id) =>
            {
                vec![StreamResponse::ArtifactUpdate(TaskArtifactUpdateEvent {
                    task_id,
                    context_id: self.context_id.clone(),
                    artifact: Artifact {
                        artifact_id: message_id.clone(),
                        name: Some(RESPONSE_ARTIFACT_NAME.to_string()),
                        parts: vec![Part::text("")],
                        ..Default::default()
                    },
                    append: Some(true),
                    last_chunk: Some(true),
                    metadata: self.artifact_metadata(event.timestamp),
                })]
            }
            // A park (durable `RunParked`, or a non-durable step waiting on
            // its client) is the service's to report: only it can tell a
            // live park from a replayed one (`input_required_status`).
            _ => Vec::new(),
        }
    }

    /// The `input-required` status update for a park on `pending`.
    pub fn input_required_status(&self, pending: &[String]) -> StreamResponse {
        let message = self.input_required_message(pending);
        self.status(&self.task_id, TaskState::InputRequired, Some(message))
    }

    /// The status message of an `input-required` state: what the run waits
    /// on, as text, and each pending tool call as a DataPart
    /// `{tool_call_id, tool_name, input}` (`distri/part_type: tool_call`).
    pub fn input_required_message(&self, pending: &[String]) -> Message {
        let calls: Vec<&ToolCall> = pending
            .iter()
            .filter_map(|id| self.tool_calls.get(id))
            .collect();
        let names: Vec<&str> = calls.iter().map(|c| c.tool_name.as_str()).collect();
        let text = if names.is_empty() {
            "Waiting for input.".to_string()
        } else {
            format!("Waiting for input: {}", names.join(", "))
        };
        let mut parts = vec![Part::text(text)];
        for c in calls {
            parts.push(distri_types::a2a_converters::part_to_a2a(
                distri_types::Part::ToolCall(c.clone()),
            ));
        }
        self.agent_message(parts)
    }

    /// `artifactUpdate`s for artifacts an event carries (both modes).
    fn artifact_frames(&mut self, event: &AgentEvent) -> Vec<StreamResponse> {
        let AgentEventType::ToolResults { results, .. } = &event.event else {
            return Vec::new();
        };
        let mut frames = Vec::new();
        for result in results {
            for part in &result.parts {
                let distri_types::Part::Artifact(file) = part else {
                    continue;
                };
                if !self.artifact_ids.insert(file.file_id.clone()) {
                    continue;
                }
                let artifact = artifact_from_file(file);
                self.artifacts.push(artifact.clone());
                frames.push(StreamResponse::ArtifactUpdate(TaskArtifactUpdateEvent {
                    task_id: event.task_id.clone(),
                    context_id: event.thread_id.clone(),
                    artifact,
                    append: Some(false),
                    last_chunk: Some(true),
                    metadata: self.artifact_metadata(event.timestamp),
                }));
            }
        }
        frames
    }

    /// The standard terminal frame for the root run's end: the terminal (or
    /// interrupted) state with the agent's answer (or the error) as the
    /// status message. Standard mode only.
    pub fn terminal_status(
        &self,
        event: &AgentEvent,
        answer: Option<Message>,
    ) -> Option<StreamResponse> {
        if self.event_stream() {
            return None;
        }
        let state = state_for(event);
        let message = match (&event.event, answer) {
            (_, Some(m)) => Some(m),
            (AgentEventType::RunError { message, .. }, None) => {
                Some(self.agent_message(vec![Part::text(message.clone())]))
            }
            _ => None,
        };
        Some(self.status(&event.task_id, state, message))
    }
}

/// `GET /tasks/{id}/events` is `event-stream`'s own method: calling it
/// activates `event-stream`, plus what the request's `A2A-Extensions`
/// header names (G3).
pub fn task_events_activation<'a>(
    header_values: impl IntoIterator<Item = &'a str>,
) -> ActiveExtensions {
    ActiveExtensions::from_request(header_values, &[]).with(DistriExtension::EventStream)
}

/// Whether `GET /tasks/{id}/events` sends an event: the envelopes of
/// `client-tools` (`tool_calls`, `tool_results`, `tool_execution_*`) and
/// `durable-runs` (`run_parked`) only when the request activated them, the
/// same as on a stream (G3). Every other event goes out as before.
pub fn task_events_include(active: &ActiveExtensions, event: &AgentEvent) -> bool {
    match DistriExtension::for_event_type(event.event.wire_type()) {
        Some(e @ (DistriExtension::ClientTools | DistriExtension::DurableRuns)) => {
            active.is_active(e)
        }
        _ => true,
    }
}

/// A status update for a stored event (thread history, replay): the
/// envelope in metadata, ISO timestamp.
pub fn status_update_for_stored(
    event: &distri_types::TaskEvent,
    task: &distri_types::Task,
) -> TaskStatusUpdateEvent {
    crate::a2a::to_a2a_task_update(event, task)
}

/// A synthesized status update with no envelope.
pub fn create_task_status_update(
    task_id: String,
    context_id: String,
    status: TaskState,
    message: Option<Message>,
) -> TaskStatusUpdateEvent {
    TaskStatusUpdateEvent {
        task_id,
        context_id,
        status: TaskStatus {
            state: status,
            message,
            timestamp: Some(chrono::Utc::now()),
        },
        metadata: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::a2a_extensions::ActiveExtensions;

    fn event(e: AgentEventType) -> AgentEvent {
        AgentEvent {
            timestamp: chrono::Utc::now(),
            thread_id: "c".into(),
            run_id: "r".into(),
            event: e,
            task_id: "t".into(),
            parent_task_id: None,
            child: None,
            agent_id: "a".into(),
            user_id: None,
            identifier_id: None,
            workspace_id: None,
            channel_id: None,
        }
    }

    fn file() -> distri_types::filesystem::FileMetadata {
        serde_json::from_value(serde_json::json!({
            "file_id": "f1", "relative_path": "out/report.md", "size": 3,
            "content_type": "text/markdown", "original_filename": null,
            "created_at": "2026-11-20T00:00:00Z", "updated_at": "2026-11-20T00:00:00Z",
            "checksum": null, "stats": null, "preview": "# hi"
        }))
        .unwrap()
    }

    fn results_with_artifact() -> AgentEventType {
        AgentEventType::ToolResults {
            step_id: "s".into(),
            parent_message_id: None,
            results: vec![distri_types::ToolResponse::from_parts(
                "c1".into(),
                "save_artifact".into(),
                vec![distri_types::Part::Artifact(file())],
            )],
        }
    }

    #[test]
    fn artifacts_are_artifact_updates_in_both_modes() {
        for active in [ActiveExtensions::default(), ActiveExtensions::all_legacy()] {
            let mut m = StreamMapper::new("t", "c", active);
            let frames = m.map(&event(results_with_artifact()));
            let a = frames
                .iter()
                .find_map(|f| match f {
                    StreamResponse::ArtifactUpdate(a) => Some(a),
                    _ => None,
                })
                .expect("artifact update");
            assert_eq!(a.artifact.artifact_id, "f1");
            assert_eq!(a.artifact.name.as_deref(), Some("report.md"));
            assert_eq!(a.last_chunk, Some(true));
            assert_eq!(
                a.artifact.parts[0].metadata_value(PART_TYPE_KEY),
                Some(&Value::String("artifact".into()))
            );
            assert_eq!(m.artifacts.len(), 1);
            // Same artifact again: not repeated.
            assert!(m
                .map(&event(results_with_artifact()))
                .iter()
                .all(|f| !matches!(f, StreamResponse::ArtifactUpdate(_))));
        }
    }

    /// G1: with `event-stream`, artifact updates and messages carry
    /// `metadata["distri/created_at"]` (ISO-8601, ms, Z); standard mode
    /// adds nothing.
    #[test]
    fn created_at_only_with_event_stream() {
        let key = distri_types::a2a_extensions::CREATED_AT_KEY;
        let mut std_m = StreamMapper::new("t", "c", ActiveExtensions::default());
        let frames = std_m.map(&event(results_with_artifact()));
        let StreamResponse::ArtifactUpdate(a) = &frames[0] else {
            panic!("artifact")
        };
        assert!(a.metadata.is_none(), "standard: no metadata");
        assert!(std_m.input_required_message(&[]).metadata.is_none());

        let mut es = StreamMapper::new("t", "c", ActiveExtensions::all_legacy());
        let frames = es.map(&event(results_with_artifact()));
        let a = frames
            .iter()
            .find_map(|f| match f {
                StreamResponse::ArtifactUpdate(a) => Some(a),
                _ => None,
            })
            .expect("artifact update");
        let at = a.metadata.as_ref().unwrap()[key].as_str().unwrap();
        assert!(
            at.ends_with('Z') && at.len() == "2026-11-20T12:00:00.000Z".len(),
            "{at}"
        );
        assert!(chrono::DateTime::parse_from_rfc3339(at).is_ok());
        let m = es.input_required_message(&[]);
        assert!(m.metadata.as_ref().unwrap()[key].is_string());
    }

    /// G3: the replay activates event-stream; client-tools and durable-runs
    /// envelopes follow the request's header.
    #[test]
    fn task_events_follow_the_header() {
        let tool_calls = event(AgentEventType::ToolCalls {
            step_id: "s".into(),
            parent_message_id: None,
            tool_calls: vec![],
        });
        let parked = event(AgentEventType::RunParked {
            task_id: "t".into(),
            pending: vec![],
            deadline: None,
            attempt: 0,
        });
        let started = event(AgentEventType::RunStarted {});

        let none = task_events_activation(std::iter::empty());
        assert!(none.is_active(DistriExtension::EventStream));
        assert!(task_events_include(&none, &started));
        assert!(!task_events_include(&none, &tool_calls));
        assert!(!task_events_include(&none, &parked));

        let header = format!(
            "{}, {}",
            DistriExtension::ClientTools.uri(),
            DistriExtension::DurableRuns.uri()
        );
        let both = task_events_activation([header.as_str()]);
        assert!(task_events_include(&both, &tool_calls));
        assert!(task_events_include(&both, &parked));
        assert!(task_events_include(&both, &started));
    }

    #[test]
    fn standard_text_streams_as_one_appended_artifact() {
        let mut m = StreamMapper::new("t", "c", ActiveExtensions::default());
        let chunk = |d: &str| AgentEventType::TextMessageContent {
            message_id: "m1".into(),
            step_id: "s".into(),
            delta: d.into(),
            stripped_content: None,
        };
        let f1 = m.map(&event(chunk("Hel")));
        let f2 = m.map(&event(chunk("lo")));
        let end = m.map(&event(AgentEventType::TextMessageEnd {
            message_id: "m1".into(),
            step_id: "s".into(),
        }));
        let art = |f: &[StreamResponse]| match &f[0] {
            StreamResponse::ArtifactUpdate(a) => a.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(art(&f1).append, Some(false));
        assert_eq!(art(&f1).artifact.parts[0].as_text(), Some("Hel"));
        assert_eq!(art(&f2).append, Some(true));
        assert_eq!(art(&end).last_chunk, Some(true));
        assert_eq!(
            art(&f1).artifact.artifact_id,
            art(&end).artifact.artifact_id
        );
    }

    #[test]
    fn standard_mode_hides_distri_events_and_sub_agents() {
        let mut m = StreamMapper::new("t", "c", ActiveExtensions::default());
        assert!(m
            .map(&event(AgentEventType::StepStarted {
                step_id: "s".into(),
                step_index: 0
            }))
            .is_empty());
        let mut child = event(AgentEventType::RunStarted {});
        child.parent_task_id = Some("t".into());
        assert!(m.map(&child).is_empty());
        let started = m.map(&event(AgentEventType::RunStarted {}));
        assert!(matches!(&started[0], StreamResponse::StatusUpdate(u)
            if u.status.state == TaskState::Working && u.metadata.is_none()));
    }

    #[test]
    fn auth_required_run_error_maps_to_auth_required() {
        let e = event(AgentEventType::RunError {
            message: "connect google".into(),
            code: Some(RUN_ERROR_AUTH_REQUIRED.into()),
            usage: None,
        });
        assert_eq!(state_for(&e), TaskState::AuthRequired);
        let m = StreamMapper::new("t", "c", ActiveExtensions::default());
        match m.terminal_status(&e, None) {
            Some(StreamResponse::StatusUpdate(u)) => {
                assert_eq!(u.status.state, TaskState::AuthRequired);
                assert_eq!(
                    u.status.message.unwrap().parts[0].as_text(),
                    Some("connect google")
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            crate::a2a::agent_run_error_code(&crate::AgentError::AuthRequired("x".into())),
            RUN_ERROR_AUTH_REQUIRED
        );
        let any = anyhow::anyhow!(crate::AgentError::AuthRequired("x".into()));
        assert_eq!(crate::a2a::run_error_code(&any), RUN_ERROR_AUTH_REQUIRED);
    }

    #[test]
    fn event_stream_envelope_gets_step_id_and_final() {
        let mut m = StreamMapper::new("t", "c", ActiveExtensions::all_legacy());
        m.map(&event(AgentEventType::StepStarted {
            step_id: "s9".into(),
            step_index: 0,
        }));
        let frames = m.map(&event(AgentEventType::RunFinished {
            success: true,
            total_steps: 1,
            failed_steps: 0,
            usage: None,
            context_budget: None,
        }));
        match &frames[0] {
            StreamResponse::StatusUpdate(u) => {
                let meta = u.metadata.as_ref().unwrap();
                assert_eq!(meta["type"], "run_finished");
                assert_eq!(meta["step_id"], "s9");
                assert_eq!(meta["final"], true);
                assert_eq!(u.status.state, TaskState::Completed);
            }
            other => panic!("{other:?}"),
        }
    }
}
