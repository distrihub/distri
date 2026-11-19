//! In-memory [`RunQueue`] + [`PendingCallStore`] (one lock, so `park` and
//! `complete` see each other atomically) and an in-process [`WakeNotifier`].
//! Used by the CLI, the OSS server without a database, and tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use distri_types::durable::{
    CallResult, Claim, ClientClaimOutcome, CompleteOutcome, NewPendingCall, ParkOutcome,
    PendingCall, PendingCallStore, PendingStatus, QueueEntry, RunInputs, RunQueue, RunState,
    WaitingOn, WakeNotifier,
};
use futures::stream::BoxStream;
use uuid::Uuid;

#[derive(Default, Clone)]
struct Entry {
    ready_at: Option<DateTime<Utc>>,
    lease_owner: Option<String>,
    lease_until: Option<DateTime<Utc>>,
    lease_id: Option<Uuid>,
    waiting_on: Option<WaitingOn>,
    attempts: i32,
    inputs: Option<RunInputs>,
    state: Option<RunState>,
}

impl Entry {
    fn workspace_id(&self) -> Option<String> {
        self.inputs
            .as_ref()
            .and_then(|i| i.principal.workspace_id.clone())
    }
    fn leased(&self, now: DateTime<Utc>) -> bool {
        self.lease_id.is_some() && self.lease_until.is_some_and(|u| u >= now)
    }
    fn holds(&self, claim: &Claim) -> bool {
        self.lease_id == Some(claim.lease_id)
    }
    fn release(&mut self) {
        self.lease_owner = None;
        self.lease_until = None;
        self.lease_id = None;
    }
}

#[derive(Default)]
struct Inner {
    entries: HashMap<String, Entry>,
    calls: HashMap<String, PendingCall>,
    /// Creation order of calls (for stable listing).
    order: Vec<String>,
}

impl Inner {
    fn wake(&mut self, task_id: &str, at: DateTime<Utc>) {
        let entry = self.entries.entry(task_id.to_string()).or_default();
        entry.ready_at = Some(match entry.ready_at {
            Some(existing) if existing < at => existing,
            _ => at,
        });
        entry.waiting_on = None;
    }

    fn task_has_pending(&self, task_id: &str) -> bool {
        self.calls
            .values()
            .any(|c| c.task_id == task_id && c.status == PendingStatus::Pending)
    }

    fn claim_entry(
        &mut self,
        task_id: &str,
        worker: &str,
        lease: Duration,
        now: DateTime<Utc>,
    ) -> Option<Claim> {
        let entry = self.entries.get_mut(task_id)?;
        if entry.ready_at.is_none_or(|r| r > now) || entry.leased(now) {
            return None;
        }
        let lease_id = Uuid::new_v4();
        entry.ready_at = None;
        entry.lease_owner = Some(worker.to_string());
        entry.lease_until = Some(now + chrono::Duration::from_std(lease).unwrap_or_default());
        entry.lease_id = Some(lease_id);
        Some(Claim {
            task_id: task_id.to_string(),
            workspace_id: entry.workspace_id(),
            lease_id,
            attempt: entry.attempts,
            worker: worker.to_string(),
            lease_ms: lease.as_millis() as i64,
        })
    }

    /// Close call `id` with `status`. First writer wins.
    fn close_call(
        &mut self,
        id: &str,
        status: PendingStatus,
        result: CallResult,
        by: Option<&str>,
        now: DateTime<Utc>,
    ) -> CompleteOutcome {
        let Some(call) = self.calls.get_mut(id) else {
            return CompleteOutcome::NotFound;
        };
        match call.status {
            PendingStatus::Pending => {}
            PendingStatus::Completed => {
                let stored = call
                    .result
                    .clone()
                    .unwrap_or(CallResult(serde_json::Value::Null));
                return if status == PendingStatus::Completed && stored.same_as(&result) {
                    CompleteOutcome::Duplicate(stored)
                } else {
                    CompleteOutcome::Conflict
                };
            }
            PendingStatus::Expired => return CompleteOutcome::Expired,
            PendingStatus::Canceled | PendingStatus::Interrupted => {
                return CompleteOutcome::Closed;
            }
        }
        // A completion after the deadline is refused: the deadline wins.
        let (status, result) = match call.deadline {
            Some(d) if status == PendingStatus::Completed && d < now => {
                (PendingStatus::Expired, CallResult::timed_out())
            }
            _ => (status, result),
        };
        call.status = status;
        call.result = Some(result);
        call.completed_by = by.map(str::to_string);
        call.completed_at = Some(now);
        let task_id = call.task_id.clone();
        if status != PendingStatus::Canceled && !self.task_has_pending(&task_id) {
            self.wake(&task_id, now);
        }
        match status {
            PendingStatus::Expired => CompleteOutcome::Expired,
            _ => CompleteOutcome::Completed,
        }
    }
}

/// In-memory run queue and pending-call store.
#[derive(Clone, Default)]
pub struct InMemoryDurableStore {
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for InMemoryDurableStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryDurableStore").finish()
    }
}

impl InMemoryDurableStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Test helper: force a task's lease to have expired at `at`.
    pub fn expire_lease_for_test(&self, task_id: &str, at: DateTime<Utc>) {
        if let Some(e) = self.lock().entries.get_mut(task_id) {
            e.lease_until = Some(at);
        }
    }
}

#[async_trait]
impl RunQueue for InMemoryDurableStore {
    async fn wake(&self, task_id: &str, at: Option<DateTime<Utc>>) -> anyhow::Result<()> {
        self.lock().wake(task_id, at.unwrap_or_else(Utc::now));
        Ok(())
    }

    async fn claim(&self, worker: &str, lease: Duration) -> anyhow::Result<Option<Claim>> {
        let now = Utc::now();
        let mut inner = self.lock();
        let mut ready: Vec<(DateTime<Utc>, String)> = inner
            .entries
            .iter()
            .filter(|(_, e)| e.ready_at.is_some_and(|r| r <= now) && !e.leased(now))
            .map(|(id, e)| (e.ready_at.unwrap_or(now), id.clone()))
            .collect();
        ready.sort();
        for (_, id) in ready {
            if let Some(claim) = inner.claim_entry(&id, worker, lease, now) {
                return Ok(Some(claim));
            }
        }
        Ok(None)
    }

    async fn claim_task(
        &self,
        task_id: &str,
        worker: &str,
        lease: Duration,
    ) -> anyhow::Result<Option<Claim>> {
        Ok(self.lock().claim_entry(task_id, worker, lease, Utc::now()))
    }

    async fn renew(&self, claim: &Claim) -> anyhow::Result<bool> {
        let now = Utc::now();
        let mut inner = self.lock();
        match inner.entries.get_mut(&claim.task_id) {
            Some(e) if e.holds(claim) => {
                e.lease_until = Some(now + chrono::Duration::milliseconds(claim.lease_ms));
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn park(&self, claim: &Claim, waiting_on: WaitingOn) -> anyhow::Result<ParkOutcome> {
        let mut inner = self.lock();
        let all_closed = waiting_on
            .pending
            .iter()
            .all(|id| inner.calls.get(id).is_none_or(|c| c.status.is_closed()));
        let entry = inner
            .entries
            .get_mut(&claim.task_id)
            .filter(|e| e.holds(claim))
            .ok_or_else(|| anyhow::anyhow!("lease lost for task {}", claim.task_id))?;
        if all_closed {
            entry.waiting_on = None;
            entry.ready_at = None;
            return Ok(ParkOutcome::Woken);
        }
        entry.release();
        entry.ready_at = None;
        entry.waiting_on = Some(waiting_on);
        Ok(ParkOutcome::Parked)
    }

    async fn finish(&self, claim: &Claim) -> anyhow::Result<bool> {
        let mut inner = self.lock();
        match inner.entries.get_mut(&claim.task_id) {
            Some(e) if e.holds(claim) => {
                e.release();
                e.ready_at = None;
                e.waiting_on = None;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn release_for_retry(&self, claim: &Claim, at: DateTime<Utc>) -> anyhow::Result<()> {
        let mut inner = self.lock();
        if let Some(e) = inner
            .entries
            .get_mut(&claim.task_id)
            .filter(|e| e.holds(claim))
        {
            e.release();
            e.ready_at = Some(at);
        }
        Ok(())
    }

    async fn reap(&self, now: DateTime<Utc>) -> anyhow::Result<Vec<String>> {
        let mut inner = self.lock();
        let mut reaped = Vec::new();
        for (id, e) in inner.entries.iter_mut() {
            if e.lease_id.is_some() && e.lease_until.is_some_and(|u| u < now) {
                e.release();
                e.attempts += 1;
                e.ready_at = Some(now);
                reaped.push(id.clone());
            }
        }
        Ok(reaped)
    }

    async fn cancel(&self, task_id: &str) -> anyhow::Result<()> {
        if let Some(e) = self.lock().entries.get_mut(task_id) {
            e.release();
            e.ready_at = None;
            e.waiting_on = None;
        }
        Ok(())
    }

    async fn put_inputs(&self, task_id: &str, inputs: &RunInputs) -> anyhow::Result<()> {
        self.lock()
            .entries
            .entry(task_id.to_string())
            .or_default()
            .inputs = Some(inputs.clone());
        Ok(())
    }

    async fn get_inputs(&self, task_id: &str) -> anyhow::Result<Option<RunInputs>> {
        Ok(self
            .lock()
            .entries
            .get(task_id)
            .and_then(|e| e.inputs.clone()))
    }

    async fn save_state(&self, claim: &Claim, state: &RunState) -> anyhow::Result<bool> {
        let mut inner = self.lock();
        match inner.entries.get_mut(&claim.task_id) {
            Some(e) if e.holds(claim) => {
                e.state = Some(state.clone());
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn get_state(&self, task_id: &str) -> anyhow::Result<Option<RunState>> {
        Ok(self
            .lock()
            .entries
            .get(task_id)
            .and_then(|e| e.state.clone()))
    }

    async fn get_entry(&self, task_id: &str) -> anyhow::Result<Option<QueueEntry>> {
        Ok(self.lock().entries.get(task_id).map(|e| QueueEntry {
            task_id: task_id.to_string(),
            workspace_id: e.workspace_id(),
            ready_at: e.ready_at,
            lease_owner: e.lease_owner.clone(),
            lease_until: e.lease_until,
            lease_id: e.lease_id,
            waiting_on: e.waiting_on.clone(),
            attempts: e.attempts,
        }))
    }
}

#[async_trait]
impl PendingCallStore for InMemoryDurableStore {
    async fn create(&self, call: NewPendingCall) -> anyhow::Result<()> {
        let mut inner = self.lock();
        if inner.calls.contains_key(&call.id) {
            return Ok(());
        }
        inner.order.push(call.id.clone());
        inner.calls.insert(
            call.id.clone(),
            PendingCall {
                id: call.id,
                workspace_id: call.workspace_id,
                task_id: call.task_id,
                thread_id: call.thread_id,
                kind: call.kind,
                tool_name: call.tool_name,
                request: call.request,
                status: PendingStatus::Pending,
                result: None,
                completed_by: None,
                deadline: call.deadline,
                created_at: Utc::now(),
                completed_at: None,
                claimed_by: None,
                claimed_until: None,
            },
        );
        Ok(())
    }

    async fn complete(
        &self,
        id: &str,
        result: CallResult,
        by: &str,
    ) -> anyhow::Result<CompleteOutcome> {
        Ok(self
            .lock()
            .close_call(id, PendingStatus::Completed, result, Some(by), Utc::now()))
    }

    async fn close(
        &self,
        id: &str,
        status: PendingStatus,
        result: CallResult,
    ) -> anyhow::Result<CompleteOutcome> {
        Ok(self.lock().close_call(id, status, result, None, Utc::now()))
    }

    async fn get(&self, id: &str) -> anyhow::Result<Option<PendingCall>> {
        Ok(self.lock().calls.get(id).cloned())
    }

    async fn list_pending(&self, task_id: &str) -> anyhow::Result<Vec<PendingCall>> {
        let inner = self.lock();
        Ok(inner
            .order
            .iter()
            .filter_map(|id| inner.calls.get(id))
            .filter(|c| c.task_id == task_id && c.status == PendingStatus::Pending)
            .cloned()
            .collect())
    }

    async fn list_for_task(&self, task_id: &str) -> anyhow::Result<Vec<PendingCall>> {
        let inner = self.lock();
        Ok(inner
            .order
            .iter()
            .filter_map(|id| inner.calls.get(id))
            .filter(|c| c.task_id == task_id)
            .cloned()
            .collect())
    }

    async fn list_pending_for_thread(&self, thread_id: &str) -> anyhow::Result<Vec<PendingCall>> {
        let inner = self.lock();
        Ok(inner
            .order
            .iter()
            .filter_map(|id| inner.calls.get(id))
            .filter(|c| c.thread_id == thread_id && c.status == PendingStatus::Pending)
            .cloned()
            .collect())
    }

    async fn expire_due(&self, now: DateTime<Utc>) -> anyhow::Result<Vec<PendingCall>> {
        let mut inner = self.lock();
        let due: Vec<String> = inner
            .calls
            .values()
            .filter(|c| c.status == PendingStatus::Pending && c.deadline.is_some_and(|d| d < now))
            .map(|c| c.id.clone())
            .collect();
        let mut expired = Vec::new();
        for id in due {
            inner.close_call(
                &id,
                PendingStatus::Expired,
                CallResult::timed_out(),
                None,
                now,
            );
            if let Some(c) = inner.calls.get(&id) {
                expired.push(c.clone());
            }
        }
        Ok(expired)
    }

    async fn cancel_for_task(&self, task_id: &str) -> anyhow::Result<usize> {
        let now = Utc::now();
        let mut inner = self.lock();
        let mut n = 0;
        for c in inner.calls.values_mut() {
            if c.task_id == task_id && c.status == PendingStatus::Pending {
                c.status = PendingStatus::Canceled;
                c.result = Some(CallResult::canceled("canceled"));
                c.completed_at = Some(now);
                n += 1;
            }
        }
        Ok(n)
    }

    async fn claim_client(
        &self,
        id: &str,
        instance: &str,
        lease: Duration,
    ) -> anyhow::Result<ClientClaimOutcome> {
        let now = Utc::now();
        let mut inner = self.lock();
        let Some(call) = inner.calls.get_mut(id) else {
            return Ok(ClientClaimOutcome::NotFound);
        };
        if call.status != PendingStatus::Pending {
            return Ok(ClientClaimOutcome::Closed(call.status));
        }
        if let (Some(by), Some(until)) = (&call.claimed_by, call.claimed_until) {
            if by != instance && until > now {
                return Ok(ClientClaimOutcome::Held {
                    by: by.clone(),
                    until,
                });
            }
        }
        let until = now + chrono::Duration::from_std(lease).unwrap_or_default();
        call.claimed_by = Some(instance.to_string());
        call.claimed_until = Some(until);
        Ok(ClientClaimOutcome::Claimed { until })
    }
}

/// In-process wake notifications (a broadcast channel).
pub struct InProcessWakeNotifier {
    tx: tokio::sync::broadcast::Sender<String>,
}

impl Default for InProcessWakeNotifier {
    fn default() -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(1024);
        Self { tx }
    }
}

impl InProcessWakeNotifier {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl WakeNotifier for InProcessWakeNotifier {
    async fn publish(&self, task_id: &str) -> anyhow::Result<()> {
        let _ = self.tx.send(task_id.to_string());
        Ok(())
    }

    async fn subscribe(&self) -> anyhow::Result<BoxStream<'static, String>> {
        use futures::StreamExt;
        let rx = self.tx.subscribe();
        Ok(futures::stream::unfold(rx, |mut rx| async move {
            loop {
                match rx.recv().await {
                    Ok(id) => return Some((id, rx)),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => return None,
                }
            }
        })
        .boxed())
    }
}
