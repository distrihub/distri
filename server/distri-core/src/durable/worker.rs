//! The run worker: claims a task, renews its lease in its own task, runs one
//! segment, then parks / finishes it; reaps expired leases and expires
//! deadlines on a tick. Replaces the bare `tokio::spawn` of a run when durable
//! mode is on.

use std::sync::{Arc, Weak};
use std::time::Duration;

use chrono::{DateTime, Utc};
use distri_types::configuration::DefinitionOverrides;
use distri_types::durable::{Claim, ParkOutcome, RunState, MAX_SEGMENT_ATTEMPTS};
use distri_types::{AgentEvent, AgentEventType, Message, MessageRole, TaskStatus};
use futures::StreamExt;

use super::{context_from_inputs, restore_state, DurableRun, DurableRuntime, SegmentKind};
use crate::a2a::stream::{run_until_cancelled, RunEnd};
use crate::agent::ExecutorContext;
use crate::AgentOrchestrator;

/// How a claimed segment starts.
#[allow(clippy::large_enum_variant)]
pub enum SegmentStart {
    /// The request that holds the claim: its context is already registered
    /// (cancel signal, mailbox) and its event relay runs.
    Request {
        message: Message,
        context: Arc<ExecutorContext>,
        overrides: Option<DefinitionOverrides>,
        kind: SegmentKind,
    },
    /// A wake (completed call, deadline, reaped lease): rebuild everything
    /// from the stored inputs.
    Resume,
}

/// What one tick did.
#[derive(Debug, Default, Clone)]
pub struct TickReport {
    /// Pending call ids that expired.
    pub expired: Vec<String>,
    /// Tasks whose lease expired and were made ready again.
    pub reaped: Vec<String>,
}

/// Claims and runs segments for one orchestrator.
pub struct RunWorker {
    orchestrator: Weak<AgentOrchestrator>,
    runtime: Arc<DurableRuntime>,
}

impl RunWorker {
    /// `None` when the orchestrator is not in durable mode.
    pub fn new(orchestrator: &Arc<AgentOrchestrator>) -> Option<Arc<Self>> {
        let runtime = orchestrator.durable.clone()?;
        Some(Arc::new(Self {
            orchestrator: Arc::downgrade(orchestrator),
            runtime,
        }))
    }

    pub fn runtime(&self) -> &Arc<DurableRuntime> {
        &self.runtime
    }

    /// Start the background loops: claim on every wake notification (the
    /// fast path), poll the queue every `poll_every` (the fallback), and run
    /// [`tick`](Self::tick) every `tick_every` (`None`: the host's scheduler
    /// calls `tick` itself).
    pub fn spawn_loops(
        self: &Arc<Self>,
        poll_every: Duration,
        tick_every: Option<Duration>,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        let mut handles = Vec::new();
        let me = self.clone();
        handles.push(tokio::spawn(async move {
            let Ok(mut wakes) = me.runtime.notifier.subscribe().await else {
                tracing::warn!("durable worker: wake subscription failed");
                return;
            };
            while let Some(task_id) = wakes.next().await {
                if me.orchestrator.upgrade().is_none() {
                    break;
                }
                me.claim_and_run(&task_id).await;
            }
        }));
        let me = self.clone();
        handles.push(tokio::spawn(async move {
            loop {
                tokio::time::sleep(poll_every).await;
                if me.orchestrator.upgrade().is_none() {
                    break;
                }
                while me.poll_once().await {}
            }
        }));
        if let Some(every) = tick_every {
            let me = self.clone();
            handles.push(tokio::spawn(async move {
                loop {
                    tokio::time::sleep(every).await;
                    if me.orchestrator.upgrade().is_none() {
                        break;
                    }
                    if let Err(e) = me.tick(Utc::now()).await {
                        tracing::warn!(error = %e, "durable worker tick failed");
                    }
                }
            }));
        }
        handles
    }

    /// Expire due pending calls and reap expired leases; wake their tasks.
    pub async fn tick(&self, now: DateTime<Utc>) -> anyhow::Result<TickReport> {
        let expired = self.runtime.pending_calls.expire_due(now).await?;
        let reaped = self.runtime.run_queue.reap(now).await?;
        let mut woken: Vec<String> = expired.iter().map(|c| c.task_id.clone()).collect();
        woken.extend(reaped.iter().cloned());
        woken.sort();
        woken.dedup();
        for task_id in &woken {
            let _ = self.runtime.notifier.publish(task_id).await;
        }
        Ok(TickReport {
            expired: expired.into_iter().map(|c| c.id).collect(),
            reaped,
        })
    }

    /// Claim the next ready task, if any, and run it. `true` if one was
    /// claimed.
    pub async fn poll_once(self: &Arc<Self>) -> bool {
        match self
            .runtime
            .run_queue
            .claim(&self.runtime.worker_id, self.runtime.lease)
            .await
        {
            Ok(Some(claim)) => {
                self.clone().run_claimed(claim, SegmentStart::Resume);
                true
            }
            Ok(None) => false,
            Err(e) => {
                tracing::warn!(error = %e, "durable worker: claim failed");
                false
            }
        }
    }

    /// Claim `task_id` now if it is ready and free, and run it.
    pub async fn claim_and_run(self: &Arc<Self>, task_id: &str) -> bool {
        match self
            .runtime
            .run_queue
            .claim_task(task_id, &self.runtime.worker_id, self.runtime.lease)
            .await
        {
            Ok(Some(claim)) => {
                self.clone().run_claimed(claim, SegmentStart::Resume);
                true
            }
            _ => false,
        }
    }

    /// Run the claimed task's segment in the background.
    pub fn run_claimed(
        self: Arc<Self>,
        claim: Claim,
        start: SegmentStart,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move { self.run_segments(claim, start).await })
    }

    async fn run_segments(self: Arc<Self>, claim: Claim, mut start: SegmentStart) {
        loop {
            let Some(orch) = self.orchestrator.upgrade() else {
                return;
            };
            match self.run_segment(orch, &claim, start).await {
                Some(next) => start = next,
                None => return,
            }
        }
    }

    /// Run one segment. `Some(next)` when the park found every call already
    /// answered (B4): continue at once under the same claim.
    async fn run_segment(
        &self,
        orch: Arc<AgentOrchestrator>,
        claim: &Claim,
        start: SegmentStart,
    ) -> Option<SegmentStart> {
        let rq = self.runtime.run_queue.clone();
        let task_id = claim.task_id.clone();

        if claim.attempt > MAX_SEGMENT_ATTEMPTS {
            fail_task(
                &orch,
                &task_id,
                &format!("run stopped after {} crashed attempts", claim.attempt),
            )
            .await;
            let _ = rq.finish(claim).await;
            return None;
        }

        let inputs = match rq.get_inputs(&task_id).await {
            Ok(Some(i)) => i,
            Ok(None) => {
                fail_task(
                    &orch,
                    &task_id,
                    "the run has no stored inputs to resume from",
                )
                .await;
                let _ = rq.finish(claim).await;
                return None;
            }
            Err(e) => {
                tracing::warn!(task_id, error = %e, "durable: cannot read run inputs; retrying");
                let _ = rq
                    .release_for_retry(claim, Utc::now() + chrono::Duration::seconds(5))
                    .await;
                return None;
            }
        };
        let stored_state = rq
            .get_state(&task_id)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();

        let user_id = inputs.principal.user_id.clone();
        let ws_uuid = inputs
            .principal
            .workspace_id
            .as_deref()
            .and_then(|w| uuid::Uuid::parse_str(w).ok());

        let runtime = self.runtime.clone();
        let claim_c = claim.clone();
        let inputs_c = inputs.clone();
        let orch_c = orch.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let segment = async move {
            let next = distri_auth::context::with_user_and_workspace(
                user_id,
                ws_uuid,
                run_segment_inner(orch_c, runtime, claim_c, inputs_c, stored_state, start),
            )
            .await;
            let _ = tx.send(next);
        };
        self.runtime
            .resume_hook
            .scope(&inputs, Box::pin(segment))
            .await;
        rx.await.ok().flatten()
    }
}

async fn run_segment_inner(
    orch: Arc<AgentOrchestrator>,
    runtime: Arc<DurableRuntime>,
    claim: Claim,
    inputs: distri_types::RunInputs,
    stored_state: RunState,
    start: SegmentStart,
) -> Option<SegmentStart> {
    let rq = runtime.run_queue.clone();
    let task_id = claim.task_id.clone();

    let (ctx, message, overrides, kind) = match start {
        SegmentStart::Request {
            message,
            context,
            overrides,
            kind,
        } => (context, message, overrides, kind),
        SegmentStart::Resume => {
            let mut ctx = context_from_inputs(orch.clone(), &task_id, &inputs);
            if let Err(e) = runtime
                .resume_hook
                .before_resume(&task_id, &inputs, &mut ctx)
                .await
            {
                fail_task(&orch, &task_id, &e.to_string()).await;
                let _ = rq.finish(&claim).await;
                return None;
            }
            if inputs.caller_env_supplied && ctx.env_vars.read().await.is_empty() {
                fail_task(
                    &orch,
                    &task_id,
                    "this run was started with caller-supplied env vars, which are not stored; \
                     it cannot continue after waiting",
                )
                .await;
                let _ = rq.finish(&claim).await;
                return None;
            }
            let thread_id = ctx.thread_id.clone();
            let (ctx, event_rx) = match orch.register_task(&task_id, &thread_id, ctx).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(task_id, error = %e, "durable: register_task failed; retrying");
                    let _ = rq
                        .release_for_retry(&claim, Utc::now() + chrono::Duration::seconds(5))
                        .await;
                    return None;
                }
            };
            orch.spawn_task_relay(task_id.clone(), event_rx);
            let message = triggering_user_message(&orch, &thread_id, &task_id).await;
            (
                ctx,
                message,
                inputs.definition_overrides.clone(),
                SegmentKind::Resume,
            )
        }
    };

    // A fresh turn starts its counters over; a resume continues them.
    let state = match kind {
        SegmentKind::Fresh => RunState {
            pending_step: None,
            segments: stored_state.segments + 1,
            ..Default::default()
        },
        _ => RunState {
            segments: stored_state.segments + 1,
            ..stored_state.clone()
        },
    };
    let run = Arc::new(DurableRun::new(
        claim.clone(),
        runtime.clone(),
        kind,
        state.clone(),
    ));
    let ctx = {
        let mut c = (*ctx).clone();
        c.durable = Some(run.clone());
        c.run_id = distri_types::run_id_for_attempt(&c.run_id, claim.attempt);
        Arc::new(c)
    };
    if kind != SegmentKind::Fresh {
        restore_state(&ctx, &state).await;
    }
    run.save().await;

    // Lease renewal in its own task: a failed renewal stops the segment (S6).
    let renewer = {
        let run = run.clone();
        let rq = rq.clone();
        let every = runtime.renew_every;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                match rq.renew(&run.claim).await {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::warn!(task_id = %run.claim.task_id, "lease lost; stopping segment");
                        run.mark_lost();
                        break;
                    }
                    Err(e) => tracing::warn!(error = %e, "lease renewal failed"),
                }
            }
        })
    };

    let agent_id = ctx.agent_id.clone();
    let exec = orch.execute_stream(&agent_id, message, ctx.clone(), overrides);
    let end = tokio::select! {
        _ = run.lease_lost_token.cancelled() => None,
        end = run_until_cancelled(exec, &ctx) => Some(end),
    };
    renewer.abort();

    let Some(end) = end else {
        // Zombie: another worker owns the task now. Write nothing.
        return None;
    };
    if run.lost() {
        return None;
    }
    match end {
        RunEnd::Canceled => {
            let _ = runtime.pending_calls.cancel_for_task(&task_id).await;
            let _ = rq.finish(&claim).await;
            None
        }
        RunEnd::Failed(e) => {
            tracing::error!(task_id, error = %e, "durable segment failed");
            if ctx.get_status().await != Some(TaskStatus::Failed) {
                ctx.update_status(TaskStatus::Failed).await;
                ctx.emit(AgentEventType::RunError {
                    message: e.to_string(),
                    code: Some("EXECUTION_ERROR".to_string()),
                    usage: Some(ctx.get_step_usage().await),
                })
                .await;
            }
            let _ = rq.finish(&claim).await;
            None
        }
        RunEnd::Finished(_) => {
            let parked = run.parked.lock().await.take();
            run.sync_from_context(&ctx).await;
            match parked {
                Some(waiting) => match rq.park(&claim, waiting).await {
                    Ok(ParkOutcome::Parked) => None,
                    Ok(ParkOutcome::Woken) => Some(SegmentStart::Resume),
                    Err(e) => {
                        tracing::warn!(task_id, error = %e, "park refused (lease lost)");
                        None
                    }
                },
                None => {
                    let _ = rq.finish(&claim).await;
                    let state = run.state.lock().await.clone();
                    runtime
                        .resume_hook
                        .on_finished(&task_id, &inputs, &state, &ctx)
                        .await;
                    None
                }
            }
        }
    }
}

/// The user message that started the task's current turn (S2): a resume
/// plans against it without saving it again.
pub(crate) async fn triggering_user_message(
    orch: &AgentOrchestrator,
    thread_id: &str,
    task_id: &str,
) -> Message {
    if let Ok(history) = orch.stores.task_store.get_history(thread_id, None).await {
        let found = history
            .into_iter()
            .filter(|(row, _)| row.id == task_id)
            .flat_map(|(_, msgs)| msgs)
            .filter_map(|m| match m {
                distri_types::TaskMessage::Message(m) if m.role == MessageRole::User => Some(m),
                _ => None,
            })
            .next_back();
        if let Some(m) = found {
            return m;
        }
    }
    Message::user("Continue.".to_string(), None)
}

/// Fail a task that has no live segment: status Failed and one terminal
/// `RunError` on its topic and in its log.
pub(crate) async fn fail_task(orch: &AgentOrchestrator, task_id: &str, message: &str) {
    let _ = orch
        .stores
        .task_store
        .update_task_status(task_id, TaskStatus::Failed)
        .await;
    publish_terminal(
        orch,
        task_id,
        AgentEventType::RunError {
            message: message.to_string(),
            code: Some("EXECUTION_ERROR".to_string()),
            usage: None,
        },
    )
    .await;
}

/// Publish and persist a terminal event for a task without a live segment.
pub(crate) async fn publish_terminal(
    orch: &AgentOrchestrator,
    task_id: &str,
    event: AgentEventType,
) {
    let task = orch
        .stores
        .task_store
        .get_task(task_id)
        .await
        .ok()
        .flatten();
    let event = AgentEvent {
        timestamp: Utc::now(),
        thread_id: task
            .as_ref()
            .map(|t| t.thread_id.clone())
            .unwrap_or_default(),
        run_id: String::new(),
        event,
        task_id: task_id.to_string(),
        parent_task_id: task.as_ref().and_then(|t| t.parent_task_id.clone()),
        child: None,
        agent_id: String::new(),
        user_id: None,
        identifier_id: None,
        workspace_id: None,
        channel_id: None,
    };
    let _ = orch
        .stores
        .task_store
        .add_event_to_task(task_id, event.clone())
        .await;
    let _ = orch.broadcaster().publish(task_id, event).await;
    let _ = orch.coordinator().complete_task(task_id).await;
}
