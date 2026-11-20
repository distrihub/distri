//! Diesel (sqlite / postgres) [`RunQueue`] + [`PendingCallStore`].
//!
//! Queue columns live on `tasks` (see `schema::task_queue`), waits in
//! `pending_calls`. Every state change is a compare-and-set `UPDATE ...
//! WHERE` (lease id, status = 'pending', lease expired), so it is safe on
//! both backends without `SKIP LOCKED`. `park` and `complete` each run in one
//! transaction that touches the task row, so a completion that lands while a
//! segment parks is never lost (B4).

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::query_builder::QueryFragment;
use diesel_async::methods::ExecuteDsl;
use diesel_async::{AsyncConnection, AsyncConnectionCore, RunQueryDsl};
use distri_types::durable::{
    CallResult, Claim, ClientClaimOutcome, CompleteOutcome, NewPendingCall, ParkOutcome,
    PendingCall, PendingCallKind, PendingCallStore, PendingStatus, QueueEntry, RunInputs, RunQueue,
    RunState, WaitingOn,
};
use uuid::Uuid;

use crate::diesel_store::{DieselBackendConnection, DieselConn, DieselStorePool};
use crate::schema::{pending_calls, task_queue};

fn ms(dt: DateTime<Utc>) -> i64 {
    dt.timestamp_millis()
}
fn from_ms(v: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(v).unwrap_or_default()
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = pending_calls)]
struct PendingCallRow {
    id: String,
    workspace_id: Option<String>,
    task_id: String,
    thread_id: String,
    kind: String,
    tool_name: Option<String>,
    request: String,
    status: String,
    result: Option<String>,
    completed_by: Option<String>,
    deadline: Option<i64>,
    created_at: i64,
    completed_at: Option<i64>,
    claimed_by: Option<String>,
    claimed_until: Option<i64>,
}

impl From<PendingCallRow> for PendingCall {
    fn from(r: PendingCallRow) -> Self {
        PendingCall {
            id: r.id,
            workspace_id: r.workspace_id,
            task_id: r.task_id,
            thread_id: r.thread_id,
            kind: PendingCallKind::parse(&r.kind),
            tool_name: r.tool_name,
            request: serde_json::from_str(&r.request).unwrap_or(serde_json::Value::Null),
            status: PendingStatus::parse(&r.status),
            result: r
                .result
                .and_then(|s| serde_json::from_str(&s).ok())
                .map(CallResult),
            completed_by: r.completed_by,
            deadline: r.deadline.map(from_ms),
            created_at: from_ms(r.created_at),
            completed_at: r.completed_at.map(from_ms),
            claimed_by: r.claimed_by,
            claimed_until: r.claimed_until.map(from_ms),
        }
    }
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = task_queue)]
struct QueueRow {
    id: String,
    ready_at: Option<i64>,
    lease_owner: Option<String>,
    lease_until: Option<i64>,
    lease_id: Option<String>,
    waiting_on: Option<String>,
    attempts: i32,
    run_inputs: Option<String>,
    run_state: Option<String>,
}

fn workspace_of(run_inputs: &Option<String>) -> Option<String> {
    run_inputs
        .as_deref()
        .and_then(|s| serde_json::from_str::<RunInputs>(s).ok())
        .and_then(|i| i.principal.workspace_id)
}

/// Whether a database error is a transient lock conflict worth retrying
/// (sqlite `database is locked` / shared-cache `table is locked`).
fn is_lock_conflict(err: &anyhow::Error) -> bool {
    let s = format!("{err:#}").to_lowercase();
    s.contains("locked") || s.contains("busy") || s.contains("could not serialize")
}

/// Diesel run queue + pending-call store over one pool.
pub struct DieselDurableStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pool: DieselStorePool<Conn>,
}

impl<Conn> Clone for DieselDurableStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
        }
    }
}

/// Outcome of closing a call inside a transaction.
enum CloseResult {
    Done(CompleteOutcome),
}

impl<Conn> DieselDurableStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pub fn new(pool: DieselStorePool<Conn>) -> Self {
        Self { pool }
    }

    async fn conn(&self) -> Result<DieselConn<'_, Conn>> {
        self.pool
            .get()
            .await
            .context("failed to acquire diesel connection")
    }

    async fn read_queue(&self, task_id: &str) -> Result<Option<QueueRow>> {
        let mut c = self.conn().await?;
        task_queue::table
            .filter(task_queue::id.eq(task_id))
            .select(QueueRow::as_select())
            .first(&mut c)
            .await
            .optional()
            .context("failed to read task queue row")
    }

    async fn try_claim(
        &self,
        task_id: &str,
        worker: &str,
        lease: Duration,
    ) -> Result<Option<Claim>> {
        let now = ms(Utc::now());
        let lease_id = Uuid::new_v4();
        let until = now + lease.as_millis() as i64;
        let mut c = self.conn().await?;
        let n = diesel::update(
            task_queue::table
                .filter(task_queue::id.eq(task_id))
                .filter(task_queue::ready_at.le(now))
                .filter(
                    task_queue::lease_id
                        .is_null()
                        .or(task_queue::lease_until.lt(now)),
                ),
        )
        .set((
            task_queue::ready_at.eq(None::<i64>),
            task_queue::lease_owner.eq(Some(worker)),
            task_queue::lease_until.eq(Some(until)),
            task_queue::lease_id.eq(Some(lease_id.to_string())),
        ))
        .execute(&mut c)
        .await
        .context("failed to claim task")?;
        drop(c);
        if n != 1 {
            return Ok(None);
        }
        let row = self
            .read_queue(task_id)
            .await?
            .ok_or_else(|| anyhow!("claimed task {task_id} vanished"))?;
        Ok(Some(Claim {
            task_id: task_id.to_string(),
            workspace_id: workspace_of(&row.run_inputs),
            lease_id,
            attempt: row.attempts,
            worker: worker.to_string(),
            lease_ms: lease.as_millis() as i64,
        }))
    }

    /// Close call `id` with `status` (first writer wins); wakes the task when
    /// nothing else is pending (except for `Canceled`). One transaction.
    async fn close_call(
        &self,
        id: &str,
        status: PendingStatus,
        result: CallResult,
        by: Option<&str>,
    ) -> Result<CompleteOutcome> {
        for attempt in 0..20 {
            match self.close_call_once(id, status, result.clone(), by).await {
                Ok(CloseResult::Done(o)) => return Ok(o),
                Err(e) if attempt < 19 && is_lock_conflict(&e) => {
                    tokio::time::sleep(Duration::from_millis(5 * (attempt + 1))).await;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

    async fn close_call_once(
        &self,
        id: &str,
        status: PendingStatus,
        result: CallResult,
        by: Option<&str>,
    ) -> Result<CloseResult> {
        let mut c = self.conn().await?;
        let id = id.to_string();
        let by = by.map(str::to_string);
        let out = c
            .transaction::<_, diesel::result::Error, _>(|conn| {
                Box::pin(async move {
                    let now = ms(Utc::now());
                    let row: Option<PendingCallRow> = pending_calls::table
                        .filter(pending_calls::id.eq(&id))
                        .select(PendingCallRow::as_select())
                        .first(conn)
                        .await
                        .optional()?;
                    let Some(row) = row else {
                        return Ok(CompleteOutcome::NotFound);
                    };
                    let current = PendingStatus::parse(&row.status);
                    if current != PendingStatus::Pending {
                        return Ok(settled_outcome(current, &row.result, status, &result));
                    }
                    let (status, result) = match row.deadline {
                        Some(d) if status == PendingStatus::Completed && d < now => {
                            (PendingStatus::Expired, CallResult::timed_out())
                        }
                        _ => (status, result),
                    };
                    let payload = serde_json::to_string(&result.0).unwrap_or_default();
                    let n = diesel::update(
                        pending_calls::table
                            .filter(pending_calls::id.eq(&id))
                            .filter(pending_calls::status.eq("pending")),
                    )
                    .set((
                        pending_calls::status.eq(status.as_str()),
                        pending_calls::result.eq(Some(payload)),
                        pending_calls::completed_by.eq(by.clone()),
                        pending_calls::completed_at.eq(Some(now)),
                    ))
                    .execute(conn)
                    .await?;
                    if n != 1 {
                        // Lost the race: report what the winner wrote.
                        let row: PendingCallRow = pending_calls::table
                            .filter(pending_calls::id.eq(&id))
                            .select(PendingCallRow::as_select())
                            .first(conn)
                            .await?;
                        return Ok(settled_outcome(
                            PendingStatus::parse(&row.status),
                            &row.result,
                            status,
                            &result,
                        ));
                    }
                    if status != PendingStatus::Canceled {
                        let left: i64 = pending_calls::table
                            .filter(pending_calls::task_id.eq(&row.task_id))
                            .filter(pending_calls::status.eq("pending"))
                            .count()
                            .get_result(conn)
                            .await?;
                        if left == 0 {
                            diesel::update(
                                task_queue::table
                                    .filter(task_queue::id.eq(&row.task_id))
                                    .filter(
                                        task_queue::ready_at
                                            .is_null()
                                            .or(task_queue::ready_at.gt(now)),
                                    ),
                            )
                            .set((
                                task_queue::ready_at.eq(Some(now)),
                                task_queue::waiting_on.eq(None::<String>),
                            ))
                            .execute(conn)
                            .await?;
                        }
                    }
                    Ok(match status {
                        PendingStatus::Expired => CompleteOutcome::Expired,
                        _ => CompleteOutcome::Completed,
                    })
                })
            })
            .await
            .map_err(|e| anyhow!("close pending call: {e}"))?;
        Ok(CloseResult::Done(out))
    }
}

/// Outcome for a write against a call already in `current`.
fn settled_outcome(
    current: PendingStatus,
    stored: &Option<String>,
    wanted: PendingStatus,
    result: &CallResult,
) -> CompleteOutcome {
    match current {
        PendingStatus::Completed => {
            let stored = stored
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .map(CallResult)
                .unwrap_or(CallResult(serde_json::Value::Null));
            if wanted == PendingStatus::Completed && stored.same_as(result) {
                CompleteOutcome::Duplicate(stored)
            } else {
                CompleteOutcome::Conflict
            }
        }
        PendingStatus::Expired => CompleteOutcome::Expired,
        PendingStatus::Pending => CompleteOutcome::Conflict,
        PendingStatus::Canceled | PendingStatus::Interrupted => CompleteOutcome::Closed,
    }
}

#[async_trait]
impl<Conn> RunQueue for DieselDurableStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    async fn wake(&self, task_id: &str, at: Option<DateTime<Utc>>) -> Result<()> {
        let at = ms(at.unwrap_or_else(Utc::now));
        let mut c = self.conn().await?;
        diesel::update(
            task_queue::table.filter(task_queue::id.eq(task_id)).filter(
                task_queue::ready_at
                    .is_null()
                    .or(task_queue::ready_at.gt(at)),
            ),
        )
        .set((
            task_queue::ready_at.eq(Some(at)),
            task_queue::waiting_on.eq(None::<String>),
        ))
        .execute(&mut c)
        .await
        .context("failed to wake task")?;
        Ok(())
    }

    async fn claim(&self, worker: &str, lease: Duration) -> Result<Option<Claim>> {
        let now = ms(Utc::now());
        let candidates: Vec<String> = {
            let mut c = self.conn().await?;
            task_queue::table
                .filter(task_queue::ready_at.le(now))
                .filter(
                    task_queue::lease_id
                        .is_null()
                        .or(task_queue::lease_until.lt(now)),
                )
                .order(task_queue::ready_at.asc())
                .select(task_queue::id)
                .limit(8)
                .load(&mut c)
                .await
                .context("failed to list ready tasks")?
        };
        for id in candidates {
            if let Some(claim) = self.try_claim(&id, worker, lease).await? {
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
    ) -> Result<Option<Claim>> {
        self.try_claim(task_id, worker, lease).await
    }

    async fn renew(&self, claim: &Claim) -> Result<bool> {
        let until = ms(Utc::now()) + claim.lease_ms;
        let mut c = self.conn().await?;
        let n = diesel::update(
            task_queue::table
                .filter(task_queue::id.eq(&claim.task_id))
                .filter(task_queue::lease_id.eq(claim.lease_id.to_string())),
        )
        .set(task_queue::lease_until.eq(Some(until)))
        .execute(&mut c)
        .await
        .context("failed to renew lease")?;
        Ok(n == 1)
    }

    async fn park(&self, claim: &Claim, waiting_on: WaitingOn) -> Result<ParkOutcome> {
        let mut last_err = None;
        for attempt in 0..20u64 {
            let mut c = self.conn().await?;
            let task_id = claim.task_id.clone();
            let lease_id = claim.lease_id.to_string();
            let worker = claim.worker.clone();
            let waiting = waiting_on.clone();
            let waiting_json = serde_json::to_string(&waiting_on)?;
            let res = c
                .transaction::<_, diesel::result::Error, _>(|conn| {
                    Box::pin(async move {
                        // Lock the task row first (fenced by the lease).
                        let n = diesel::update(
                            task_queue::table
                                .filter(task_queue::id.eq(&task_id))
                                .filter(task_queue::lease_id.eq(&lease_id)),
                        )
                        .set(task_queue::lease_owner.eq(Some(worker)))
                        .execute(conn)
                        .await?;
                        if n != 1 {
                            return Ok(None);
                        }
                        let open: i64 = if waiting.pending.is_empty() {
                            0
                        } else {
                            pending_calls::table
                                .filter(pending_calls::id.eq_any(&waiting.pending))
                                .filter(pending_calls::status.eq("pending"))
                                .count()
                                .get_result(conn)
                                .await?
                        };
                        if open == 0 {
                            diesel::update(task_queue::table.filter(task_queue::id.eq(&task_id)))
                                .set((
                                    task_queue::waiting_on.eq(None::<String>),
                                    task_queue::ready_at.eq(None::<i64>),
                                ))
                                .execute(conn)
                                .await?;
                            return Ok(Some(ParkOutcome::Woken));
                        }
                        diesel::update(task_queue::table.filter(task_queue::id.eq(&task_id)))
                            .set((
                                task_queue::waiting_on.eq(Some(waiting_json)),
                                task_queue::ready_at.eq(None::<i64>),
                                task_queue::lease_owner.eq(None::<String>),
                                task_queue::lease_until.eq(None::<i64>),
                                task_queue::lease_id.eq(None::<String>),
                            ))
                            .execute(conn)
                            .await?;
                        Ok(Some(ParkOutcome::Parked))
                    })
                })
                .await;
            match res {
                Ok(Some(o)) => return Ok(o),
                Ok(None) => return Err(anyhow!("lease lost for task {}", claim.task_id)),
                Err(e) => {
                    let e = anyhow!("park: {e}");
                    if attempt < 19 && is_lock_conflict(&e) {
                        tokio::time::sleep(Duration::from_millis(5 * (attempt + 1))).await;
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("park failed")))
    }

    async fn finish(&self, claim: &Claim) -> Result<bool> {
        let mut c = self.conn().await?;
        let n = diesel::update(
            task_queue::table
                .filter(task_queue::id.eq(&claim.task_id))
                .filter(task_queue::lease_id.eq(claim.lease_id.to_string())),
        )
        .set((
            task_queue::ready_at.eq(None::<i64>),
            task_queue::waiting_on.eq(None::<String>),
            task_queue::lease_owner.eq(None::<String>),
            task_queue::lease_until.eq(None::<i64>),
            task_queue::lease_id.eq(None::<String>),
        ))
        .execute(&mut c)
        .await
        .context("failed to finish task")?;
        Ok(n == 1)
    }

    async fn release_for_retry(&self, claim: &Claim, at: DateTime<Utc>) -> Result<()> {
        let mut c = self.conn().await?;
        diesel::update(
            task_queue::table
                .filter(task_queue::id.eq(&claim.task_id))
                .filter(task_queue::lease_id.eq(claim.lease_id.to_string())),
        )
        .set((
            task_queue::ready_at.eq(Some(ms(at))),
            task_queue::lease_owner.eq(None::<String>),
            task_queue::lease_until.eq(None::<i64>),
            task_queue::lease_id.eq(None::<String>),
        ))
        .execute(&mut c)
        .await
        .context("failed to release task")?;
        Ok(())
    }

    async fn reap(&self, now: DateTime<Utc>) -> Result<Vec<String>> {
        let now = ms(now);
        let mut c = self.conn().await?;
        let expired: Vec<String> = task_queue::table
            .filter(task_queue::lease_id.is_not_null())
            .filter(task_queue::lease_until.lt(now))
            .select(task_queue::id)
            .load(&mut c)
            .await
            .context("failed to list expired leases")?;
        let mut reaped = Vec::new();
        for id in expired {
            let n = diesel::update(
                task_queue::table
                    .filter(task_queue::id.eq(&id))
                    .filter(task_queue::lease_id.is_not_null())
                    .filter(task_queue::lease_until.lt(now)),
            )
            .set((
                task_queue::ready_at.eq(Some(now)),
                task_queue::attempts.eq(task_queue::attempts + 1),
                task_queue::lease_owner.eq(None::<String>),
                task_queue::lease_until.eq(None::<i64>),
                task_queue::lease_id.eq(None::<String>),
            ))
            .execute(&mut c)
            .await
            .context("failed to reap task")?;
            if n == 1 {
                reaped.push(id);
            }
        }
        Ok(reaped)
    }

    async fn cancel(&self, task_id: &str) -> Result<()> {
        let mut c = self.conn().await?;
        diesel::update(task_queue::table.filter(task_queue::id.eq(task_id)))
            .set((
                task_queue::ready_at.eq(None::<i64>),
                task_queue::waiting_on.eq(None::<String>),
                task_queue::lease_owner.eq(None::<String>),
                task_queue::lease_until.eq(None::<i64>),
                task_queue::lease_id.eq(None::<String>),
            ))
            .execute(&mut c)
            .await
            .context("failed to cancel queue entry")?;
        Ok(())
    }

    async fn put_inputs(&self, task_id: &str, inputs: &RunInputs) -> Result<()> {
        let json = serde_json::to_string(inputs)?;
        let mut c = self.conn().await?;
        let n = diesel::update(task_queue::table.filter(task_queue::id.eq(task_id)))
            .set(task_queue::run_inputs.eq(Some(json)))
            .execute(&mut c)
            .await
            .context("failed to store run inputs")?;
        if n != 1 {
            return Err(anyhow!("task {task_id} has no row to store run inputs on"));
        }
        Ok(())
    }

    async fn get_inputs(&self, task_id: &str) -> Result<Option<RunInputs>> {
        Ok(self
            .read_queue(task_id)
            .await?
            .and_then(|r| r.run_inputs)
            .and_then(|s| serde_json::from_str(&s).ok()))
    }

    async fn save_state(&self, claim: &Claim, state: &RunState) -> Result<bool> {
        let json = serde_json::to_string(state)?;
        let mut c = self.conn().await?;
        let n = diesel::update(
            task_queue::table
                .filter(task_queue::id.eq(&claim.task_id))
                .filter(task_queue::lease_id.eq(claim.lease_id.to_string())),
        )
        .set(task_queue::run_state.eq(Some(json)))
        .execute(&mut c)
        .await
        .context("failed to save run state")?;
        Ok(n == 1)
    }

    async fn get_state(&self, task_id: &str) -> Result<Option<RunState>> {
        Ok(self
            .read_queue(task_id)
            .await?
            .and_then(|r| r.run_state)
            .and_then(|s| serde_json::from_str(&s).ok()))
    }

    async fn get_entry(&self, task_id: &str) -> Result<Option<QueueEntry>> {
        Ok(self.read_queue(task_id).await?.map(|r| QueueEntry {
            workspace_id: workspace_of(&r.run_inputs),
            task_id: r.id,
            ready_at: r.ready_at.map(from_ms),
            lease_owner: r.lease_owner,
            lease_until: r.lease_until.map(from_ms),
            lease_id: r.lease_id.and_then(|s| Uuid::parse_str(&s).ok()),
            waiting_on: r.waiting_on.and_then(|s| serde_json::from_str(&s).ok()),
            attempts: r.attempts,
        }))
    }
}

#[async_trait]
impl<Conn> PendingCallStore for DieselDurableStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    async fn create(&self, call: NewPendingCall) -> Result<()> {
        let mut c = self.conn().await?;
        // Idempotent for the same task; a tool call id reused by another
        // task (a provider with non-unique ids) is refused instead of
        // silently waiting on the other task's row.
        let owner: Option<String> = pending_calls::table
            .filter(pending_calls::id.eq(&call.id))
            .select(pending_calls::task_id)
            .first(&mut c)
            .await
            .optional()?;
        if let Some(owner) = owner {
            if owner != call.task_id {
                return Err(anyhow!(
                    "pending call id {} already belongs to task {owner}",
                    call.id
                ));
            }
            return Ok(());
        }
        let res = diesel::insert_into(pending_calls::table)
            .values((
                pending_calls::id.eq(&call.id),
                pending_calls::workspace_id.eq(&call.workspace_id),
                pending_calls::task_id.eq(&call.task_id),
                pending_calls::thread_id.eq(&call.thread_id),
                pending_calls::kind.eq(call.kind.as_str()),
                pending_calls::tool_name.eq(&call.tool_name),
                pending_calls::request.eq(serde_json::to_string(&call.request)?),
                pending_calls::status.eq("pending"),
                pending_calls::deadline.eq(call.deadline.map(ms)),
                pending_calls::created_at.eq(ms(Utc::now())),
            ))
            .execute(&mut c)
            .await;
        match res {
            Ok(_) => Ok(()),
            // Raced with another create of the same id: idempotent.
            Err(diesel::result::Error::DatabaseError(
                diesel::result::DatabaseErrorKind::UniqueViolation,
                _,
            )) => Ok(()),
            Err(e) => Err(anyhow!("failed to create pending call: {e}")),
        }
    }

    async fn complete(&self, id: &str, result: CallResult, by: &str) -> Result<CompleteOutcome> {
        self.close_call(id, PendingStatus::Completed, result, Some(by))
            .await
    }

    async fn close(
        &self,
        id: &str,
        status: PendingStatus,
        result: CallResult,
    ) -> Result<CompleteOutcome> {
        self.close_call(id, status, result, None).await
    }

    async fn get(&self, id: &str) -> Result<Option<PendingCall>> {
        let mut c = self.conn().await?;
        Ok(pending_calls::table
            .filter(pending_calls::id.eq(id))
            .select(PendingCallRow::as_select())
            .first(&mut c)
            .await
            .optional()?
            .map(Into::into))
    }

    async fn list_pending(&self, task_id: &str) -> Result<Vec<PendingCall>> {
        let mut c = self.conn().await?;
        let rows: Vec<PendingCallRow> = pending_calls::table
            .filter(pending_calls::task_id.eq(task_id))
            .filter(pending_calls::status.eq("pending"))
            .order((pending_calls::created_at.asc(), pending_calls::id.asc()))
            .select(PendingCallRow::as_select())
            .load(&mut c)
            .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn list_for_task(&self, task_id: &str) -> Result<Vec<PendingCall>> {
        let mut c = self.conn().await?;
        let rows: Vec<PendingCallRow> = pending_calls::table
            .filter(pending_calls::task_id.eq(task_id))
            .order((pending_calls::created_at.asc(), pending_calls::id.asc()))
            .select(PendingCallRow::as_select())
            .load(&mut c)
            .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn list_pending_for_thread(&self, thread_id: &str) -> Result<Vec<PendingCall>> {
        let mut c = self.conn().await?;
        let rows: Vec<PendingCallRow> = pending_calls::table
            .filter(pending_calls::thread_id.eq(thread_id))
            .filter(pending_calls::status.eq("pending"))
            .order((pending_calls::created_at.asc(), pending_calls::id.asc()))
            .select(PendingCallRow::as_select())
            .load(&mut c)
            .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn expire_due(&self, now: DateTime<Utc>) -> Result<Vec<PendingCall>> {
        let due: Vec<String> = {
            let mut c = self.conn().await?;
            pending_calls::table
                .filter(pending_calls::status.eq("pending"))
                .filter(pending_calls::deadline.lt(ms(now)))
                .select(pending_calls::id)
                .load(&mut c)
                .await?
        };
        let mut expired = Vec::new();
        for id in due {
            let outcome = self
                .close_call(&id, PendingStatus::Expired, CallResult::timed_out(), None)
                .await?;
            if outcome == CompleteOutcome::Expired
                && let Some(call) = self.get(&id).await?
            {
                expired.push(call);
            }
        }
        Ok(expired)
    }

    async fn cancel_for_task(&self, task_id: &str) -> Result<usize> {
        let mut c = self.conn().await?;
        let payload = serde_json::to_string(&CallResult::canceled("canceled").0)?;
        let n = diesel::update(
            pending_calls::table
                .filter(pending_calls::task_id.eq(task_id))
                .filter(pending_calls::status.eq("pending")),
        )
        .set((
            pending_calls::status.eq("canceled"),
            pending_calls::result.eq(Some(payload)),
            pending_calls::completed_at.eq(Some(ms(Utc::now()))),
        ))
        .execute(&mut c)
        .await?;
        Ok(n)
    }

    async fn claim_client(
        &self,
        id: &str,
        instance: &str,
        lease: Duration,
    ) -> Result<ClientClaimOutcome> {
        let now = ms(Utc::now());
        let until = now + lease.as_millis() as i64;
        let mut c = self.conn().await?;
        let n = diesel::update(
            pending_calls::table
                .filter(pending_calls::id.eq(id))
                .filter(pending_calls::status.eq("pending"))
                .filter(
                    pending_calls::claimed_by
                        .is_null()
                        .or(pending_calls::claimed_by.eq(instance))
                        .or(pending_calls::claimed_until.lt(now)),
                ),
        )
        .set((
            pending_calls::claimed_by.eq(Some(instance)),
            pending_calls::claimed_until.eq(Some(until)),
        ))
        .execute(&mut c)
        .await?;
        drop(c);
        if n == 1 {
            return Ok(ClientClaimOutcome::Claimed {
                until: from_ms(until),
            });
        }
        Ok(match self.get(id).await? {
            None => ClientClaimOutcome::NotFound,
            Some(call) if call.status != PendingStatus::Pending => {
                ClientClaimOutcome::Closed(call.status)
            }
            Some(call) => ClientClaimOutcome::Held {
                by: call.claimed_by.unwrap_or_default(),
                until: call.claimed_until.unwrap_or_else(Utc::now),
            },
        })
    }
}
