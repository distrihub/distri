use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use distri_types::{AgentEvent, AgentEventType};
use futures_util::stream::BoxStream;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use super::mailbox::{AgentMessage, MailboxSender};
use super::pool::{AgentJob, WorkerPool};

const CHANNEL_CAPACITY: usize = 512;
const MAILBOX_CAPACITY: usize = 64;

/// Per-task state managed by the worker pool.
struct TaskEntry {
    /// Cancellation token to signal abort to the running agent loop.
    cancel_token: CancellationToken,
    /// Broadcast sender for live event streaming.
    event_tx: broadcast::Sender<AgentEvent>,
    /// Mailbox sender for inter-agent message delivery.
    mailbox_tx: MailboxSender,
    /// Event log for replay by late subscribers.
    event_log: Vec<AgentEvent>,
    /// Parent task_id, if this was spawned as a child.
    parent_task_id: Option<String>,
    /// Whether the task has reached a terminal state.
    completed: bool,
}

/// In-memory worker pool using DashMap + tokio broadcast channels.
///
/// Each submitted job gets:
/// - A `CancellationToken` for abort signaling
/// - A `broadcast::Sender` for live event streaming
/// - A `Mailbox` for inter-agent message delivery
/// - An event log for replay by late subscribers
///
/// This implementation is suitable for single-process deployments (OSS/CLI).
/// For multi-process cloud deployments, use `RedisWorkerPool` instead.
pub struct InMemoryWorkerPool {
    /// Per-task state.
    tasks: DashMap<String, TaskEntry>,
    /// Name → task_id mapping for SendMessage routing.
    names: DashMap<String, String>,
}

impl InMemoryWorkerPool {
    pub fn new() -> Self {
        Self {
            tasks: DashMap::new(),
            names: DashMap::new(),
        }
    }

    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// Internal: register a task and return all resources including broadcast sender.
    /// Used by tests that need direct access to the broadcast::Sender.
    pub fn register_task_full(
        &self,
        job: &AgentJob,
    ) -> (CancellationToken, broadcast::Sender<AgentEvent>, super::mailbox::Mailbox) {
        let cancel_token = CancellationToken::new();
        let (event_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let mailbox = super::mailbox::Mailbox::new(MAILBOX_CAPACITY);
        let mailbox_sender = mailbox.sender();

        self.tasks.insert(
            job.task_id.clone(),
            TaskEntry {
                cancel_token: cancel_token.clone(),
                event_tx: event_tx.clone(),
                mailbox_tx: mailbox_sender,
                event_log: Vec::new(),
                parent_task_id: job.parent_task_id.clone(),
                completed: false,
            },
        );

        if let Some(ref name) = job.agent_name {
            self.names.insert(name.clone(), job.task_id.clone());
        }

        (cancel_token, event_tx, mailbox)
    }

    /// Publish an event for a task (synchronous version for internal use).
    /// Appends to the event log and broadcasts to live subscribers.
    pub fn publish_event_sync(&self, task_id: &str, event: AgentEvent) {
        if let Some(mut entry) = self.tasks.get_mut(task_id) {
            // Check if this is a terminal event
            let is_terminal = matches!(
                &event.event,
                AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. }
            );

            entry.event_log.push(event.clone());
            // Broadcast to live subscribers (ignore if no receivers)
            let _ = entry.event_tx.send(event.clone());

            if is_terminal {
                entry.completed = true;

                // If this task has a parent, notify it
                if let Some(ref parent_id) = entry.parent_task_id.clone() {
                    // Drop the mutable borrow before accessing the parent
                    drop(entry);
                    if let Some(parent_entry) = self.tasks.get(parent_id) {
                        let _ = parent_entry.event_tx.send(event);
                    }
                }
            }
        }
    }
}

impl Default for InMemoryWorkerPool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WorkerPool for InMemoryWorkerPool {
    async fn register_task(
        &self,
        job: &AgentJob,
    ) -> anyhow::Result<(CancellationToken, super::mailbox::Mailbox)> {
        let (token, _tx, mailbox) = self.register_task_full(job);
        Ok((token, mailbox))
    }

    async fn submit(&self, job: AgentJob) -> anyhow::Result<String> {
        let task_id = job.task_id.clone();

        // Ensure task entry exists (it may have been pre-registered via register_task)
        if !self.tasks.contains_key(&task_id) {
            let cancel_token = CancellationToken::new();
            let (event_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
            let mailbox = super::mailbox::Mailbox::new(MAILBOX_CAPACITY);
            let mailbox_sender = mailbox.sender();

            self.tasks.insert(
                task_id.clone(),
                TaskEntry {
                    cancel_token,
                    event_tx,
                    mailbox_tx: mailbox_sender,
                    event_log: Vec::new(),
                    parent_task_id: job.parent_task_id.clone(),
                    completed: false,
                },
            );

            if let Some(ref name) = job.agent_name {
                self.names.insert(name.clone(), task_id.clone());
            }
        }

        Ok(task_id)
    }

    async fn cancel(&self, task_id: &str) -> anyhow::Result<()> {
        if let Some(entry) = self.tasks.get(task_id) {
            entry.cancel_token.cancel();
            Ok(())
        } else {
            // Task not found — could have already completed
            Ok(())
        }
    }

    async fn subscribe(&self, task_id: &str) -> anyhow::Result<BoxStream<'static, AgentEvent>> {
        let entry = self
            .tasks
            .get(task_id)
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", task_id))?;

        // Collect replay events
        let replay: Vec<AgentEvent> = entry.event_log.clone();
        let completed = entry.completed;

        // Subscribe to live events
        let mut rx = entry.event_tx.subscribe();
        drop(entry);

        let stream = async_stream::stream! {
            // Replay buffered events
            for event in replay {
                yield event;
            }

            // If already completed, don't wait for more
            if completed {
                return;
            }

            // Stream live events
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let is_terminal = matches!(
                            &event.event,
                            AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. }
                        );
                        yield event;
                        if is_terminal {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("WorkerPool subscriber lagged by {} events", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        };

        Ok(Box::pin(stream))
    }

    async fn deliver_message(&self, task_id: &str, msg: AgentMessage) -> anyhow::Result<()> {
        let entry = self
            .tasks
            .get(task_id)
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", task_id))?;

        if entry.completed {
            return Err(anyhow::anyhow!("Task {} has already completed", task_id));
        }

        entry
            .mailbox_tx
            .send(msg)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to deliver message: {}", e))?;

        Ok(())
    }

    async fn resolve_name(&self, name: &str) -> Option<String> {
        // 1. Check name registry
        if let Some(task_id) = self.names.get(name) {
            return Some(task_id.clone());
        }
        // 2. Check if `name` is a direct task_id
        if self.tasks.contains_key(name) {
            return Some(name.to_string());
        }
        None
    }

    async fn register_name(&self, name: &str, task_id: &str) -> anyhow::Result<()> {
        self.names.insert(name.to_string(), task_id.to_string());
        Ok(())
    }

    async fn publish_event(&self, task_id: &str, event: AgentEvent) {
        self.publish_event_sync(task_id, event);
    }

    async fn is_running(&self, task_id: &str) -> bool {
        self.tasks
            .get(task_id)
            .map(|e| !e.completed)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    fn make_event(task_id: &str, event_type: AgentEventType) -> AgentEvent {
        AgentEvent {
            timestamp: chrono::Utc::now(),
            thread_id: "test-thread".to_string(),
            run_id: "test-run".to_string(),
            event: event_type,
            task_id: task_id.to_string(),
            agent_id: "test-agent".to_string(),
            user_id: None,
            identifier_id: None,
            workspace_id: None,
            channel_id: None,
        }
    }

    #[tokio::test]
    async fn test_submit_and_subscribe() {
        let pool = InMemoryWorkerPool::new();

        let job = AgentJob {
            task_id: "task-1".to_string(),
            thread_id: "thread-1".to_string(),
            agent_id: "agent-1".to_string(),
            message: distri_types::Message::user("hello".to_string(), None),
            workspace_id: None,
            user_id: "user-1".to_string(),
            parent_task_id: None,
            agent_name: None,
        };

        pool.submit(job).await.unwrap();

        // Publish events
        pool.publish_event_sync(
            "task-1",
            make_event("task-1", AgentEventType::RunStarted {}),
        );
        pool.publish_event_sync(
            "task-1",
            make_event(
                "task-1",
                AgentEventType::RunFinished {
                    success: true,
                    total_steps: 1,
                    failed_steps: 0,
                    usage: None,
                    context_budget: None,
                },
            ),
        );

        // Subscribe replays all events
        let mut stream = pool.subscribe("task-1").await.unwrap();

        let event1 = stream.next().await.unwrap();
        assert!(matches!(event1.event, AgentEventType::RunStarted {}));

        let event2 = stream.next().await.unwrap();
        assert!(matches!(event2.event, AgentEventType::RunFinished { .. }));

        // Stream should end after terminal event
        let event3 = stream.next().await;
        assert!(event3.is_none());
    }

    #[tokio::test]
    async fn test_cancel_signals_token() {
        let pool = InMemoryWorkerPool::new();

        let job = AgentJob {
            task_id: "task-cancel".to_string(),
            thread_id: "thread-1".to_string(),
            agent_id: "agent-1".to_string(),
            message: distri_types::Message::user("hello".to_string(), None),
            workspace_id: None,
            user_id: "user-1".to_string(),
            parent_task_id: None,
            agent_name: None,
        };

        let (cancel_token, _, _) = pool.register_task_full(&job);
        assert!(!cancel_token.is_cancelled());

        pool.cancel("task-cancel").await.unwrap();
        assert!(cancel_token.is_cancelled());
    }

    #[tokio::test]
    async fn test_name_resolution() {
        let pool = InMemoryWorkerPool::new();

        let job = AgentJob {
            task_id: "task-named".to_string(),
            thread_id: "thread-1".to_string(),
            agent_id: "agent-1".to_string(),
            message: distri_types::Message::user("hello".to_string(), None),
            workspace_id: None,
            user_id: "user-1".to_string(),
            parent_task_id: None,
            agent_name: Some("researcher".to_string()),
        };

        pool.submit(job).await.unwrap();

        // Resolve by name
        assert_eq!(
            pool.resolve_name("researcher").await,
            Some("task-named".to_string())
        );

        // Resolve by task_id directly
        assert_eq!(
            pool.resolve_name("task-named").await,
            Some("task-named".to_string())
        );

        // Unknown name
        assert_eq!(pool.resolve_name("unknown").await, None);
    }

    #[tokio::test]
    async fn test_mailbox_delivery() {
        let pool = InMemoryWorkerPool::new();

        let job = AgentJob {
            task_id: "task-mail".to_string(),
            thread_id: "thread-1".to_string(),
            agent_id: "agent-1".to_string(),
            message: distri_types::Message::user("hello".to_string(), None),
            workspace_id: None,
            user_id: "user-1".to_string(),
            parent_task_id: None,
            agent_name: None,
        };

        let (_, _, mut mailbox) = pool.register_task_full(&job);

        // Deliver message via pool
        pool.deliver_message(
            "task-mail",
            AgentMessage {
                from: "agent-a".to_string(),
                content: "hi there".to_string(),
            },
        )
        .await
        .unwrap();

        // Agent loop picks it up
        let msgs = mailbox.try_recv_all();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hi there");
    }

    #[tokio::test]
    async fn test_parent_notification_on_child_completion() {
        let pool = InMemoryWorkerPool::new();

        // Register parent task
        let parent_job = AgentJob {
            task_id: "parent-task".to_string(),
            thread_id: "thread-1".to_string(),
            agent_id: "parent-agent".to_string(),
            message: distri_types::Message::user("parent work".to_string(), None),
            workspace_id: None,
            user_id: "user-1".to_string(),
            parent_task_id: None,
            agent_name: None,
        };
        pool.submit(parent_job).await.unwrap();

        // Register child task with parent_task_id
        let child_job = AgentJob {
            task_id: "child-task".to_string(),
            thread_id: "thread-1".to_string(),
            agent_id: "child-agent".to_string(),
            message: distri_types::Message::user("child work".to_string(), None),
            workspace_id: None,
            user_id: "user-1".to_string(),
            parent_task_id: Some("parent-task".to_string()),
            agent_name: None,
        };
        pool.submit(child_job).await.unwrap();

        // Subscribe to parent events
        let parent_entry = pool.tasks.get("parent-task").unwrap();
        let mut parent_rx = parent_entry.event_tx.subscribe();
        drop(parent_entry);

        // Child completes — parent should get notified
        let finish_event = make_event(
            "child-task",
            AgentEventType::RunFinished {
                success: true,
                total_steps: 1,
                failed_steps: 0,
                usage: None,
                context_budget: None,
            },
        );
        pool.publish_event_sync("child-task", finish_event);

        // Parent receives the child's completion event
        let parent_event = parent_rx.recv().await.unwrap();
        assert!(matches!(
            parent_event.event,
            AgentEventType::RunFinished { .. }
        ));
        assert_eq!(parent_event.task_id, "child-task");
    }

    #[tokio::test]
    async fn test_is_running() {
        let pool = InMemoryWorkerPool::new();

        let job = AgentJob {
            task_id: "task-run".to_string(),
            thread_id: "thread-1".to_string(),
            agent_id: "agent-1".to_string(),
            message: distri_types::Message::user("hello".to_string(), None),
            workspace_id: None,
            user_id: "user-1".to_string(),
            parent_task_id: None,
            agent_name: None,
        };

        pool.submit(job).await.unwrap();
        assert!(pool.is_running("task-run").await);

        pool.publish_event_sync(
            "task-run",
            make_event(
                "task-run",
                AgentEventType::RunFinished {
                    success: true,
                    total_steps: 1,
                    failed_steps: 0,
                    usage: None,
                    context_budget: None,
                },
            ),
        );
        assert!(!pool.is_running("task-run").await);
    }

    #[tokio::test]
    async fn test_live_subscribe_receives_events() {
        let pool = Arc::new(InMemoryWorkerPool::new());

        let job = AgentJob {
            task_id: "task-live".to_string(),
            thread_id: "thread-1".to_string(),
            agent_id: "agent-1".to_string(),
            message: distri_types::Message::user("hello".to_string(), None),
            workspace_id: None,
            user_id: "user-1".to_string(),
            parent_task_id: None,
            agent_name: None,
        };
        pool.submit(job).await.unwrap();

        // Subscribe first (no events yet)
        let pool_clone = pool.clone();
        let handle = tokio::spawn(async move {
            let mut stream = pool_clone.subscribe("task-live").await.unwrap();
            let mut events = Vec::new();
            while let Some(event) = stream.next().await {
                let is_terminal = matches!(
                    &event.event,
                    AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. }
                );
                events.push(event);
                if is_terminal {
                    break;
                }
            }
            events
        });

        // Small delay to ensure subscriber is ready
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Publish events
        pool.publish_event_sync(
            "task-live",
            make_event("task-live", AgentEventType::RunStarted {}),
        );
        pool.publish_event_sync(
            "task-live",
            make_event(
                "task-live",
                AgentEventType::RunFinished {
                    success: true,
                    total_steps: 1,
                    failed_steps: 0,
                    usage: None,
                    context_budget: None,
                },
            ),
        );

        let events = handle.await.unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0].event, AgentEventType::RunStarted {}));
        assert!(matches!(events[1].event, AgentEventType::RunFinished { .. }));
    }
}
