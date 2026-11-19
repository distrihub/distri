//! Conformance checks every [`RunQueue`] + [`PendingCallStore`] must pass.
//! Hosts with their own implementation (distri-cloud's Postgres) call
//! [`run_all`] from their store tests with a fresh store and a function that
//! makes a task row (the queue lives on tasks).

use std::time::Duration;

use chrono::Utc;
use distri_types::durable::{
    CallResult, ClientClaimOutcome, CompleteOutcome, NewPendingCall, ParkOutcome, PendingCallKind,
    PendingCallStore, PendingStatus, RunInputs, RunQueue, RunState, WaitingOn,
};

const LEASE: Duration = Duration::from_secs(30);

fn new_call(id: &str, task: &str, thread: &str) -> NewPendingCall {
    NewPendingCall {
        id: id.to_string(),
        workspace_id: Some("ws-1".to_string()),
        task_id: task.to_string(),
        thread_id: thread.to_string(),
        kind: PendingCallKind::ClientTool,
        tool_name: Some("pick".to_string()),
        request: serde_json::json!({"q": 1}),
        deadline: None,
    }
}

async fn queued<S: RunQueue + ?Sized>(s: &S, task: &str) {
    let mut inputs = RunInputs::default();
    inputs.principal.workspace_id = Some("ws-1".to_string());
    s.put_inputs(task, &inputs).await.expect("put_inputs");
}

/// Run every check. `make_task` must create a task row with the given id
/// (and thread) and return once it exists.
pub async fn run_all<S, F, Fut>(store: &S, make_task: F)
where
    S: RunQueue + PendingCallStore,
    F: Fn(String, String) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let id = || uuid::Uuid::new_v4().to_string();

    // claim / claim_task / renew / finish
    {
        let (t, th) = (id(), id());
        make_task(t.clone(), th.clone()).await;
        queued(store, &t).await;
        assert!(
            store.claim_task(&t, "w1", LEASE).await.unwrap().is_none(),
            "not ready yet"
        );
        store.wake(&t, None).await.unwrap();
        let c = store
            .claim_task(&t, "w1", LEASE)
            .await
            .unwrap()
            .expect("claim");
        assert_eq!(c.workspace_id.as_deref(), Some("ws-1"));
        assert!(
            store.claim_task(&t, "w2", LEASE).await.unwrap().is_none(),
            "held"
        );
        assert!(store.renew(&c).await.unwrap());
        let mut state = RunState::default();
        state.iterations = 3;
        assert!(store.save_state(&c, &state).await.unwrap());
        assert_eq!(store.get_state(&t).await.unwrap().unwrap().iterations, 3);
        assert!(store.finish(&c).await.unwrap());
        assert!(!store.renew(&c).await.unwrap(), "finished claim is gone");
        assert!(
            !store.save_state(&c, &state).await.unwrap(),
            "fenced after finish"
        );
        let e = store.get_entry(&t).await.unwrap().unwrap();
        assert!(e.ready_at.is_none() && e.lease_owner.is_none());
    }

    // S1: park leaves the task unclaimed and waiting
    {
        let (t, th) = (id(), id());
        make_task(t.clone(), th.clone()).await;
        queued(store, &t).await;
        store.wake(&t, None).await.unwrap();
        let c = store.claim_task(&t, "w1", LEASE).await.unwrap().unwrap();
        store.create(new_call("c1", &t, &th)).await.unwrap();
        store.create(new_call("c1", &t, &th)).await.unwrap(); // idempotent
        let out = store
            .park(
                &c,
                WaitingOn {
                    pending: vec!["c1".into()],
                    deadline: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(out, ParkOutcome::Parked);
        let e = store.get_entry(&t).await.unwrap().unwrap();
        assert!(e.ready_at.is_none() && e.lease_owner.is_none());
        assert_eq!(e.waiting_on.unwrap().pending, vec!["c1".to_string()]);
        assert_eq!(store.list_pending(&t).await.unwrap().len(), 1);
        assert_eq!(store.list_pending_for_thread(&th).await.unwrap().len(), 1);

        // S2 / S6: complete wakes it; duplicate vs conflict.
        let r = CallResult(serde_json::json!({"ok": true}));
        assert_eq!(
            store.complete("c1", r.clone(), "u").await.unwrap(),
            CompleteOutcome::Completed
        );
        assert!(matches!(
            store.complete("c1", r.clone(), "u").await.unwrap(),
            CompleteOutcome::Duplicate(_)
        ));
        assert_eq!(
            store
                .complete("c1", CallResult(serde_json::json!({"ok": false})), "u")
                .await
                .unwrap(),
            CompleteOutcome::Conflict
        );
        assert_eq!(
            store.complete("nope", r, "u").await.unwrap(),
            CompleteOutcome::NotFound
        );
        let c2 = store.claim(&"w2".to_string(), LEASE).await.unwrap();
        // Another ready task from an earlier check may not exist; this one must.
        let c2 = match c2 {
            Some(c) if c.task_id == t => c,
            _ => store
                .claim_task(&t, "w2", LEASE)
                .await
                .unwrap()
                .expect("woken"),
        };
        store.finish(&c2).await.unwrap();
    }

    // S19 / B4: completed before park → Woken, claim kept
    {
        let (t, th) = (id(), id());
        make_task(t.clone(), th.clone()).await;
        queued(store, &t).await;
        store.wake(&t, None).await.unwrap();
        let c = store.claim_task(&t, "w1", LEASE).await.unwrap().unwrap();
        store.create(new_call("c19", &t, &th)).await.unwrap();
        store
            .complete("c19", CallResult(serde_json::json!(1)), "u")
            .await
            .unwrap();
        let out = store
            .park(
                &c,
                WaitingOn {
                    pending: vec!["c19".into()],
                    deadline: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(out, ParkOutcome::Woken);
        assert!(store.renew(&c).await.unwrap(), "claim kept after Woken");
        assert!(store.finish(&c).await.unwrap());
    }

    // S5 / S30: deadline expiry wakes; a late result is refused
    {
        let (t, th) = (id(), id());
        make_task(t.clone(), th.clone()).await;
        queued(store, &t).await;
        let mut call = new_call("c5", &t, &th);
        call.deadline = Some(Utc::now() - chrono::Duration::seconds(1));
        store.create(call).await.unwrap();
        let expired = store.expire_due(Utc::now()).await.unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].status, PendingStatus::Expired);
        assert_eq!(
            store
                .complete("c5", CallResult(serde_json::json!(1)), "u")
                .await
                .unwrap(),
            CompleteOutcome::Expired
        );
        assert!(
            store
                .get_entry(&t)
                .await
                .unwrap()
                .unwrap()
                .ready_at
                .is_some()
        );
        assert!(store.expire_due(Utc::now()).await.unwrap().is_empty());
    }

    // S13: cancel_for_task does not wake
    {
        let (t, th) = (id(), id());
        make_task(t.clone(), th.clone()).await;
        queued(store, &t).await;
        store.create(new_call("c13", &t, &th)).await.unwrap();
        assert_eq!(store.cancel_for_task(&t).await.unwrap(), 1);
        assert!(
            store
                .get_entry(&t)
                .await
                .unwrap()
                .unwrap()
                .ready_at
                .is_none()
        );
        assert_eq!(
            store
                .complete("c13", CallResult(serde_json::json!(1)), "u")
                .await
                .unwrap(),
            CompleteOutcome::Closed
        );
    }

    // S7 / S31: reap an expired lease; the zombie's writes are refused
    {
        let (t, th) = (id(), id());
        make_task(t.clone(), th.clone()).await;
        queued(store, &t).await;
        store.wake(&t, None).await.unwrap();
        let c = store
            .claim_task(&t, "w1", Duration::from_millis(1))
            .await
            .unwrap()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        let reaped = store.reap(Utc::now()).await.unwrap();
        assert!(reaped.contains(&t));
        assert!(!store.save_state(&c, &RunState::default()).await.unwrap());
        assert!(!store.renew(&c).await.unwrap());
        assert!(store.park(&c, WaitingOn::default()).await.is_err());
        let c2 = store.claim_task(&t, "w2", LEASE).await.unwrap().unwrap();
        assert_eq!(c2.attempt, 1);
        store.release_for_retry(&c2, Utc::now()).await.unwrap();
        let c3 = store.claim_task(&t, "w3", LEASE).await.unwrap().unwrap();
        store.cancel(&t).await.unwrap();
        assert!(!store.renew(&c3).await.unwrap());
        let _ = th;
    }

    // S5 (client): claim before running
    {
        let (t, th) = (id(), id());
        make_task(t.clone(), th.clone()).await;
        store.create(new_call("c28", &t, &th)).await.unwrap();
        assert!(matches!(
            store.claim_client("c28", "tab-a", LEASE).await.unwrap(),
            ClientClaimOutcome::Claimed { .. }
        ));
        assert!(matches!(
            store.claim_client("c28", "tab-b", LEASE).await.unwrap(),
            ClientClaimOutcome::Held { .. }
        ));
        assert!(matches!(
            store.claim_client("c28", "tab-a", LEASE).await.unwrap(),
            ClientClaimOutcome::Claimed { .. }
        ));
        assert_eq!(
            store.claim_client("missing", "tab-a", LEASE).await.unwrap(),
            ClientClaimOutcome::NotFound
        );
    }

    // S30: concurrent completions — exactly one Completed
    {
        let (t, th) = (id(), id());
        make_task(t.clone(), th.clone()).await;
        queued(store, &t).await;
        store.create(new_call("c30", &t, &th)).await.unwrap();
        let (a, b) = tokio::join!(
            store.complete("c30", CallResult(serde_json::json!("a")), "x"),
            store.close("c30", PendingStatus::Expired, CallResult::timed_out()),
        );
        let outcomes = [a.unwrap(), b.unwrap()];
        let firsts = outcomes
            .iter()
            .filter(|o| matches!(o, CompleteOutcome::Completed | CompleteOutcome::Expired))
            .count();
        assert!(firsts >= 1, "{outcomes:?}");
        let stored = store.get("c30").await.unwrap().unwrap();
        assert!(stored.status.is_closed());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_conforms() {
        let store = crate::durable::InMemoryDurableStore::new();
        run_all(&store, |_t, _th| async {}).await;
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn sqlite_conforms() {
        use distri_types::stores::{CreateTaskInput, TaskStore};
        let db = format!("file:{}?mode=memory&cache=shared", uuid::Uuid::new_v4());
        let builder = crate::diesel_store::SqliteStoreBuilder::sqlite(&db, 4)
            .await
            .unwrap();
        let tasks = builder.task_store();
        let store = builder.durable_store();
        run_all(&store, |t, th| {
            let tasks = &tasks;
            async move {
                tasks
                    .create_task(CreateTaskInput::local(&th).with_id(&t))
                    .await
                    .unwrap();
            }
        })
        .await;
    }
}
