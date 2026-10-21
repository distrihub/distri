//! Runs: a root task and every task under it, as one tree.
//!
//! A *run* is a root task (`parent_task_id == None`) together with its
//! sub-agents, background forks and workflow steps. Clients list runs with
//! `GET /v1/runs` ([`RunListResponse`]), attach with A2A `tasks/resubscribe`,
//! cancel with A2A `tasks/cancel` and answer a waiting tool with
//! `POST /v1/agents/{agent}/complete-tool`. Spec:
//! `docs/specs/2026-10-22-multi-run.md` in distri-cloud.

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::stores::TaskActivity;
use crate::{Task, TaskStatus};

/// Default page size for [`RunListQuery::limit`].
pub const DEFAULT_RUN_PAGE: u32 = 20;
/// Largest page a caller may ask for.
pub const MAX_RUN_PAGE: u32 = 100;

/// What a run is doing, from the point of view of the person watching it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Working,
    /// Waiting on a person: an external tool, an approval or a question.
    NeedsInput,
    Done,
    Failed,
    Canceled,
}

impl From<&TaskStatus> for RunState {
    fn from(status: &TaskStatus) -> Self {
        match status {
            TaskStatus::Pending | TaskStatus::Running => RunState::Working,
            TaskStatus::InputRequired => RunState::NeedsInput,
            TaskStatus::Completed => RunState::Done,
            TaskStatus::Failed => RunState::Failed,
            TaskStatus::Canceled => RunState::Canceled,
        }
    }
}

impl RunState {
    pub fn is_terminal(self) -> bool {
        matches!(self, RunState::Done | RunState::Failed | RunState::Canceled)
    }

    /// The `tasks.status` values that map to this state.
    pub fn task_statuses(self) -> &'static [&'static str] {
        match self {
            RunState::Working => &["pending", "running"],
            RunState::NeedsInput => &["input_required"],
            RunState::Done => &["completed"],
            RunState::Failed => &["failed"],
            RunState::Canceled => &["canceled"],
        }
    }

    /// Sort rank: waiting on a person first, then working, then finished.
    fn rank(self) -> u8 {
        match self {
            RunState::NeedsInput => 0,
            RunState::Working => 1,
            RunState::Failed => 2,
            RunState::Done | RunState::Canceled => 3,
        }
    }
}

/// One node of the run tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct RunSummary {
    pub task_id: String,
    pub thread_id: String,
    pub parent_task_id: Option<String>,
    /// Root: the thread's agent. Child: the agent its `Invocation` targeted
    /// (`_adhoc_base` for ad-hoc workers).
    pub agent_id: Option<String>,
    pub state: RunState,
    /// First user message: what the run was asked to do.
    pub intent: Option<String>,
    /// Newest contentful event or message, one line.
    pub last_event: Option<String>,
    pub last_event_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Number of direct children.
    pub child_count: usize,
    /// This task or any descendant is waiting on a person.
    pub needs_input: bool,
    #[schema(no_recursion)]
    pub children: Vec<RunSummary>,
}

impl RunSummary {
    /// This node or a descendant with `task_id`.
    pub fn find(&self, task_id: &str) -> Option<&RunSummary> {
        if self.task_id == task_id {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(task_id))
    }

    /// Depth-first, in display order, with each node's depth (root = 0).
    pub fn flatten(&self) -> Vec<(usize, &RunSummary)> {
        let mut out = Vec::new();
        self.flatten_into(0, &mut out);
        out
    }

    fn flatten_into<'a>(&'a self, depth: usize, out: &mut Vec<(usize, &'a RunSummary)>) {
        out.push((depth, self));
        for child in &self.children {
            child.flatten_into(depth + 1, out);
        }
    }

    /// Newest `updated_at` / `last_event_at` anywhere in this subtree.
    pub fn latest_activity(&self) -> i64 {
        let own = self.updated_at.max(self.last_event_at.unwrap_or(0));
        self.children
            .iter()
            .map(RunSummary::latest_activity)
            .fold(own, i64::max)
    }

    /// Whether this node or a descendant is in `state`.
    pub fn contains_state(&self, state: RunState) -> bool {
        self.state == state || self.children.iter().any(|c| c.contains_state(state))
    }
}

/// Query for `GET /v1/runs`.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, ToSchema, IntoParams,
)]
#[into_params(parameter_in = Query)]
pub struct RunListQuery {
    /// Only runs on this thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Only runs whose root task ran this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Only runs where the root or any descendant is in this state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<RunState>,
    /// Roots per page. Default 20, max 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

impl RunListQuery {
    /// Page size after defaulting and clamping.
    pub fn page_size(&self) -> u32 {
        self.limit
            .unwrap_or(DEFAULT_RUN_PAGE)
            .clamp(1, MAX_RUN_PAGE)
    }

    pub fn page_offset(&self) -> u32 {
        self.offset.unwrap_or(0)
    }
}

/// Body of `GET /v1/runs`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct RunListResponse {
    pub runs: Vec<RunSummary>,
    pub has_more: bool,
}

/// A task row plus the agent that runs it. What a `TaskStore` returns for
/// [`build_run_tree`].
#[derive(Debug, Clone, PartialEq)]
pub struct RunTaskRow {
    pub task: Task,
    pub agent_id: Option<String>,
}

/// Build the ordered forest from flat rows. A row whose parent is not in the
/// set becomes a root. Siblings and roots are ordered waiting-on-a-person
/// first, then working, then by newest activity.
pub fn build_run_tree(
    rows: Vec<RunTaskRow>,
    activity: &HashMap<String, TaskActivity>,
) -> Vec<RunSummary> {
    let ids: std::collections::HashSet<String> = rows.iter().map(|r| r.task.id.clone()).collect();
    let mut children_of: HashMap<String, Vec<RunTaskRow>> = HashMap::new();
    let mut roots: Vec<RunTaskRow> = Vec::new();
    for row in rows {
        match row.task.parent_task_id.as_deref() {
            Some(parent) if ids.contains(parent) => {
                children_of.entry(parent.to_string()).or_default().push(row);
            }
            _ => roots.push(row),
        }
    }
    let mut forest: Vec<RunSummary> = roots
        .into_iter()
        .map(|row| build_node(row, &mut children_of, activity))
        .collect();
    sort_siblings(&mut forest);
    forest
}

fn build_node(
    row: RunTaskRow,
    children_of: &mut HashMap<String, Vec<RunTaskRow>>,
    activity: &HashMap<String, TaskActivity>,
) -> RunSummary {
    let child_rows = children_of.remove(&row.task.id).unwrap_or_default();
    let mut children: Vec<RunSummary> = child_rows
        .into_iter()
        .map(|c| build_node(c, children_of, activity))
        .collect();
    sort_siblings(&mut children);

    let state = RunState::from(&row.task.status);
    let needs_input = state == RunState::NeedsInput || children.iter().any(|c| c.needs_input);
    let act = activity.get(&row.task.id);
    RunSummary {
        task_id: row.task.id,
        thread_id: row.task.thread_id,
        parent_task_id: row.task.parent_task_id,
        agent_id: row.agent_id,
        state,
        intent: act.and_then(|a| a.intent.clone()),
        last_event: act.and_then(|a| a.preview.clone()),
        last_event_at: act.and_then(|a| a.last_event_at),
        created_at: row.task.created_at,
        updated_at: row.task.updated_at,
        child_count: children.len(),
        needs_input,
        children,
    }
}

fn sort_siblings(nodes: &mut [RunSummary]) {
    nodes.sort_by(|a, b| {
        let rank = |n: &RunSummary| {
            if n.needs_input { 0 } else { n.state.rank() }
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| b.latest_activity().cmp(&a.latest_activity()))
            .then_with(|| a.task_id.cmp(&b.task_id))
    });
}

/// Agent a task runs: its `Invocation`'s first target, else the thread's
/// agent. Root tasks started by `message/stream` carry an empty invocation,
/// so they fall back to the thread.
pub fn task_agent_id(
    invocation: Option<&crate::invocation::Invocation>,
    thread_agent_id: Option<String>,
) -> Option<String> {
    invocation
        .and_then(|i| i.primary_agent_id())
        .map(str::to_string)
        .or(thread_agent_id)
}

/// Page runs in memory, for stores without a recursive query. `rows` may be
/// every task the caller can see. Groups them into runs, keeps runs that
/// match `query`, orders roots by newest activity anywhere in the run and
/// returns the rows of `limit + 1` runs (the extra one sets `has_more`).
pub fn page_run_rows(rows: Vec<RunTaskRow>, query: &RunListQuery) -> Vec<RunTaskRow> {
    let parent_of: HashMap<String, Option<String>> = rows
        .iter()
        .map(|r| (r.task.id.clone(), r.task.parent_task_id.clone()))
        .collect();
    let root_of = |id: &str| -> String {
        let mut current = id.to_string();
        // Bounded walk; a cycle in bad data must not hang the request.
        for _ in 0..parent_of.len() {
            match parent_of.get(&current).cloned().flatten() {
                Some(p) if parent_of.contains_key(&p) => current = p,
                _ => break,
            }
        }
        current
    };

    struct Group {
        rows: Vec<RunTaskRow>,
        last_at: i64,
        statuses: Vec<RunState>,
    }
    let mut groups: HashMap<String, Group> = HashMap::new();
    for row in rows {
        let root = root_of(&row.task.id);
        let g = groups.entry(root).or_insert(Group {
            rows: Vec::new(),
            last_at: 0,
            statuses: Vec::new(),
        });
        g.last_at = g.last_at.max(row.task.updated_at);
        g.statuses.push(RunState::from(&row.task.status));
        g.rows.push(row);
    }

    let mut kept: Vec<(String, Group)> = groups
        .into_iter()
        .filter(|(root_id, g)| {
            let root = g.rows.iter().find(|r| &r.task.id == root_id);
            let thread_ok = query
                .thread_id
                .as_deref()
                .is_none_or(|t| root.is_some_and(|r| r.task.thread_id == t));
            let agent_ok = query
                .agent_id
                .as_deref()
                .is_none_or(|a| root.is_some_and(|r| r.agent_id.as_deref() == Some(a)));
            let state_ok = query.state.is_none_or(|s| g.statuses.contains(&s));
            thread_ok && agent_ok && state_ok
        })
        .collect();
    kept.sort_by(|a, b| b.1.last_at.cmp(&a.1.last_at).then_with(|| a.0.cmp(&b.0)));
    kept.into_iter()
        .skip(query.page_offset() as usize)
        .take(query.page_size() as usize + 1)
        .flat_map(|(_, g)| g.rows)
        .collect()
}

/// Rows of the subtree rooted at `task_id` (the root included), in memory.
pub fn subtree_rows(rows: Vec<RunTaskRow>, task_id: &str) -> Vec<RunTaskRow> {
    let mut keep: std::collections::HashSet<String> = std::collections::HashSet::new();
    if rows.iter().any(|r| r.task.id == task_id) {
        keep.insert(task_id.to_string());
    }
    loop {
        let before = keep.len();
        for r in &rows {
            if let Some(p) = &r.task.parent_task_id
                && keep.contains(p)
            {
                keep.insert(r.task.id.clone());
            }
        }
        if keep.len() == before {
            break;
        }
    }
    rows.into_iter()
        .filter(|r| keep.contains(&r.task.id))
        .collect()
}

/// Turn a store page from [`TaskStore::list_run_tasks`] into the response:
/// build the forest, keep `limit` roots and say whether there were more.
///
/// [`TaskStore::list_run_tasks`]: crate::stores::TaskStore::list_run_tasks
pub fn into_run_list(
    rows: Vec<RunTaskRow>,
    activity: &HashMap<String, TaskActivity>,
    query: &RunListQuery,
) -> RunListResponse {
    let mut runs = build_run_tree(rows, activity);
    let limit = query.page_size() as usize;
    let has_more = runs.len() > limit;
    if has_more {
        // The store paged by newest activity; drop the extra run by that same
        // order, not by the needs-input-first display order.
        let drop_id = runs
            .iter()
            .min_by_key(|r| (r.latest_activity(), std::cmp::Reverse(r.task_id.clone())))
            .map(|r| r.task_id.clone());
        runs.retain(|r| Some(&r.task_id) != drop_id.as_ref());
    }
    RunListResponse { runs, has_more }
}

/// The shared #170 fixture: a root run that forked two background children,
/// one finished and one waiting on a person. Used by the CLI and distrijs
/// tests; pinned to `distri/testing/fixtures/multi_run/*.json`.
pub mod fixture {
    use chrono::{TimeZone, Utc};
    use serde::{Deserialize, Serialize};

    use super::{RunListResponse, RunState, RunSummary};
    use crate::{AgentEvent, AgentEventType, MessageRole, ToolCall};

    pub const THREAD_ID: &str = "thread-multi-run";
    pub const ROOT_TASK_ID: &str = "task-root";
    pub const WORKER_TASK_ID: &str = "task-worker";
    pub const ASKER_TASK_ID: &str = "task-asker";
    pub const ROOT_AGENT: &str = "multi_run_parent";
    pub const WORKER_AGENT: &str = "multi_run_worker";
    pub const ASKER_AGENT: &str = "multi_run_asker";
    /// External tool the asker calls; no client registers a handler for it,
    /// so a person answers it.
    pub const ASK_TOOL: &str = "ask_human";
    pub const ASK_CALL_ID: &str = "call-ask-1";
    pub const ASK_QUESTION: &str = "Ship the release on Friday?";

    /// Input of the fixture's `ask_human` external tool.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct AskHumanInput {
        pub question: String,
    }

    const T0: i64 = 1_792_000_000_000;

    /// `GET /v1/runs` for the fixture, as the server returns it.
    pub fn multi_run() -> RunListResponse {
        let worker = RunSummary {
            task_id: WORKER_TASK_ID.into(),
            thread_id: THREAD_ID.into(),
            parent_task_id: Some(ROOT_TASK_ID.into()),
            agent_id: Some(WORKER_AGENT.into()),
            state: RunState::Done,
            intent: Some("Count the open issues labelled release".into()),
            last_event: Some("final: 4 open release issues".into()),
            last_event_at: Some(T0 + 30_000),
            created_at: T0 + 2_000,
            updated_at: T0 + 30_000,
            child_count: 0,
            needs_input: false,
            children: vec![],
        };
        let asker = RunSummary {
            task_id: ASKER_TASK_ID.into(),
            thread_id: THREAD_ID.into(),
            parent_task_id: Some(ROOT_TASK_ID.into()),
            agent_id: Some(ASKER_AGENT.into()),
            state: RunState::NeedsInput,
            intent: Some("Ask the release owner whether to ship on Friday".into()),
            last_event: Some(format!("{ASK_TOOL}: {ASK_QUESTION}")),
            last_event_at: Some(T0 + 12_000),
            created_at: T0 + 2_100,
            updated_at: T0 + 12_000,
            child_count: 0,
            needs_input: true,
            children: vec![],
        };
        let root = RunSummary {
            task_id: ROOT_TASK_ID.into(),
            thread_id: THREAD_ID.into(),
            parent_task_id: None,
            agent_id: Some(ROOT_AGENT.into()),
            state: RunState::Done,
            intent: Some("Prepare the Friday release".into()),
            last_event: Some("final: started 2 background agents".into()),
            last_event_at: Some(T0 + 5_000),
            created_at: T0,
            updated_at: T0 + 5_000,
            child_count: 2,
            needs_input: true,
            children: vec![asker, worker],
        };
        RunListResponse {
            runs: vec![root],
            has_more: false,
        }
    }

    fn event(seq: i64, event: AgentEventType) -> AgentEvent {
        AgentEvent {
            timestamp: Utc
                .timestamp_millis_opt(T0 + 2_100 + seq * 100)
                .single()
                .unwrap_or_default(),
            thread_id: THREAD_ID.into(),
            run_id: ASKER_TASK_ID.into(),
            event,
            task_id: ASKER_TASK_ID.into(),
            parent_task_id: Some(ROOT_TASK_ID.into()),
            agent_id: ASKER_AGENT.into(),
            user_id: None,
            identifier_id: None,
            workspace_id: None,
            channel_id: None,
        }
    }

    /// The pending `ask_human` call in the fixture.
    pub fn ask_call() -> ToolCall {
        ToolCall {
            tool_call_id: ASK_CALL_ID.into(),
            tool_name: ASK_TOOL.into(),
            input: serde_json::to_value(AskHumanInput {
                question: ASK_QUESTION.into(),
            })
            .unwrap_or_default(),
        }
    }

    /// What `tasks/resubscribe` replays for the asker: it starts, says what
    /// it is doing, and calls `ask_human`, which is still pending.
    pub fn pending_ask_events() -> Vec<AgentEvent> {
        let call = ask_call();
        vec![
            event(0, AgentEventType::RunStarted {}),
            event(
                1,
                AgentEventType::StepStarted {
                    step_id: "step-1".into(),
                    step_index: 0,
                },
            ),
            event(
                2,
                AgentEventType::TextMessageStart {
                    message_id: "msg-1".into(),
                    step_id: "step-1".into(),
                    role: MessageRole::Assistant,
                },
            ),
            event(
                3,
                AgentEventType::TextMessageContent {
                    message_id: "msg-1".into(),
                    step_id: "step-1".into(),
                    delta: "I need the release owner to decide.".into(),
                    stripped_content: None,
                },
            ),
            event(
                4,
                AgentEventType::TextMessageEnd {
                    message_id: "msg-1".into(),
                    step_id: "step-1".into(),
                },
            ),
            event(
                5,
                AgentEventType::ToolCalls {
                    step_id: "step-1".into(),
                    parent_message_id: Some("msg-1".into()),
                    tool_calls: vec![call.clone()],
                },
            ),
            event(
                6,
                AgentEventType::ToolExecutionStart {
                    step_id: "step-1".into(),
                    tool_call_id: call.tool_call_id.clone(),
                    tool_call_name: call.tool_name.clone(),
                    input: call.input.clone(),
                },
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, parent: Option<&str>, status: TaskStatus, updated: i64) -> RunTaskRow {
        RunTaskRow {
            task: Task {
                id: id.into(),
                thread_id: "th".into(),
                parent_task_id: parent.map(Into::into),
                status,
                created_at: updated - 10,
                updated_at: updated,
            },
            agent_id: Some(format!("agent-{id}")),
        }
    }

    #[test]
    fn state_mapping_covers_every_status() {
        assert_eq!(RunState::from(&TaskStatus::Pending), RunState::Working);
        assert_eq!(RunState::from(&TaskStatus::Running), RunState::Working);
        assert_eq!(
            RunState::from(&TaskStatus::InputRequired),
            RunState::NeedsInput
        );
        assert_eq!(RunState::from(&TaskStatus::Completed), RunState::Done);
        assert_eq!(RunState::from(&TaskStatus::Failed), RunState::Failed);
        assert_eq!(RunState::from(&TaskStatus::Canceled), RunState::Canceled);
    }

    #[test]
    fn tree_nests_children_and_rolls_up_needs_input() {
        let rows = vec![
            task("root", None, TaskStatus::Completed, 100),
            task("worker", Some("root"), TaskStatus::Completed, 300),
            task("asker", Some("root"), TaskStatus::InputRequired, 120),
        ];
        let forest = build_run_tree(rows, &HashMap::new());
        assert_eq!(forest.len(), 1);
        let root = &forest[0];
        assert_eq!(root.child_count, 2);
        assert!(root.needs_input);
        assert_eq!(root.state, RunState::Done);
        // Waiting child sorts before the newer finished one.
        assert_eq!(root.children[0].task_id, "asker");
        assert_eq!(root.children[1].task_id, "worker");
        assert_eq!(root.latest_activity(), 300);
    }

    #[test]
    fn needs_input_roots_sort_first_then_by_activity() {
        let rows = vec![
            task("new", None, TaskStatus::Running, 900),
            task("old", None, TaskStatus::Completed, 100),
            task("old-child", Some("old"), TaskStatus::InputRequired, 150),
            task("failed", None, TaskStatus::Failed, 950),
        ];
        let forest = build_run_tree(rows, &HashMap::new());
        let order: Vec<&str> = forest.iter().map(|r| r.task_id.as_str()).collect();
        assert_eq!(order, vec!["old", "new", "failed"]);
    }

    #[test]
    fn orphan_rows_become_roots_and_activity_fills_fields() {
        let rows = vec![task("child", Some("missing"), TaskStatus::Running, 10)];
        let mut activity = HashMap::new();
        activity.insert(
            "child".to_string(),
            TaskActivity {
                preview: Some("Bash: ls".into()),
                last_event_at: Some(42),
                intent: Some("list files".into()),
                context_used_tokens: None,
                context_window_tokens: None,
            },
        );
        let forest = build_run_tree(rows, &activity);
        assert_eq!(forest.len(), 1);
        assert_eq!(forest[0].parent_task_id.as_deref(), Some("missing"));
        assert_eq!(forest[0].last_event.as_deref(), Some("Bash: ls"));
        assert_eq!(forest[0].intent.as_deref(), Some("list files"));
        assert_eq!(forest[0].latest_activity(), 42);
    }

    #[test]
    fn find_and_flatten_walk_in_display_order() {
        let fx = fixture::multi_run();
        let root = &fx.runs[0];
        let flat: Vec<(usize, &str)> = root
            .flatten()
            .into_iter()
            .map(|(d, n)| (d, n.task_id.as_str()))
            .collect();
        assert_eq!(
            flat,
            vec![
                (0, fixture::ROOT_TASK_ID),
                (1, fixture::ASKER_TASK_ID),
                (1, fixture::WORKER_TASK_ID)
            ]
        );
        assert!(root.find(fixture::ASKER_TASK_ID).is_some());
        assert!(root.find("nope").is_none());
        assert!(root.contains_state(RunState::NeedsInput));
    }

    #[test]
    fn fixture_matches_what_build_run_tree_produces() {
        let fx = fixture::multi_run();
        let rows: Vec<RunTaskRow> = fx.runs[0]
            .flatten()
            .into_iter()
            .map(|(_, n)| RunTaskRow {
                task: Task {
                    id: n.task_id.clone(),
                    thread_id: n.thread_id.clone(),
                    parent_task_id: n.parent_task_id.clone(),
                    status: match n.state {
                        RunState::Working => TaskStatus::Running,
                        RunState::NeedsInput => TaskStatus::InputRequired,
                        RunState::Done => TaskStatus::Completed,
                        RunState::Failed => TaskStatus::Failed,
                        RunState::Canceled => TaskStatus::Canceled,
                    },
                    created_at: n.created_at,
                    updated_at: n.updated_at,
                },
                agent_id: n.agent_id.clone(),
            })
            .collect();
        let activity: HashMap<String, TaskActivity> = fx.runs[0]
            .flatten()
            .into_iter()
            .map(|(_, n)| {
                (
                    n.task_id.clone(),
                    TaskActivity {
                        preview: n.last_event.clone(),
                        last_event_at: n.last_event_at,
                        intent: n.intent.clone(),
                        context_used_tokens: None,
                        context_window_tokens: None,
                    },
                )
            })
            .collect();
        assert_eq!(build_run_tree(rows, &activity), fx.runs);
    }

    /// The JSON files distrijs imports must equal the typed fixture.
    #[test]
    fn fixture_json_files_are_pinned() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../testing/fixtures/multi_run");
        // `DISTRI_WRITE_FIXTURES=1 cargo test -p distri-types runs::` rewrites them.
        if std::env::var("DISTRI_WRITE_FIXTURES").as_deref() == Ok("1") {
            std::fs::create_dir_all(&dir).expect("fixture dir");
            std::fs::write(
                dir.join("runs.json"),
                serde_json::to_string_pretty(&fixture::multi_run()).unwrap() + "\n",
            )
            .expect("write runs.json");
            std::fs::write(
                dir.join("asker_events.json"),
                serde_json::to_string_pretty(&fixture::pending_ask_events()).unwrap() + "\n",
            )
            .expect("write asker_events.json");
        }
        let runs: RunListResponse = serde_json::from_str(
            &std::fs::read_to_string(dir.join("runs.json")).expect("runs.json"),
        )
        .expect("runs.json parses as RunListResponse");
        assert_eq!(runs, fixture::multi_run());
        let events: Vec<crate::AgentEvent> = serde_json::from_str(
            &std::fs::read_to_string(dir.join("asker_events.json")).expect("asker_events.json"),
        )
        .expect("asker_events.json parses as Vec<AgentEvent>");
        let expected = fixture::pending_ask_events();
        assert_eq!(events.len(), expected.len());
        for (a, b) in events.iter().zip(expected.iter()) {
            assert_eq!(
                serde_json::to_value(a).unwrap(),
                serde_json::to_value(b).unwrap()
            );
        }
    }

    #[test]
    fn page_run_rows_pages_whole_runs_by_newest_activity() {
        let rows = vec![
            task("a", None, TaskStatus::Completed, 100),
            task("a1", Some("a"), TaskStatus::InputRequired, 500),
            task("b", None, TaskStatus::Running, 300),
            task("c", None, TaskStatus::Completed, 200),
        ];
        let q = RunListQuery {
            limit: Some(1),
            ..Default::default()
        };
        let page = page_run_rows(rows.clone(), &q);
        let ids: Vec<&str> = page.iter().map(|r| r.task.id.as_str()).collect();
        // Run "a" (activity 500) and the extra run "b" (300).
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&"a") && ids.contains(&"a1") && ids.contains(&"b"));

        let resp = into_run_list(page, &HashMap::new(), &q);
        assert!(resp.has_more);
        assert_eq!(resp.runs.len(), 1);
        assert_eq!(resp.runs[0].task_id, "a");

        let q = RunListQuery {
            state: Some(RunState::NeedsInput),
            ..Default::default()
        };
        let page = page_run_rows(rows.clone(), &q);
        assert!(page.iter().all(|r| r.task.id.starts_with('a')));

        let q = RunListQuery {
            agent_id: Some("agent-b".into()),
            ..Default::default()
        };
        let page = page_run_rows(rows.clone(), &q);
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].task.id, "b");

        let sub = subtree_rows(rows, "a1");
        assert_eq!(sub.len(), 1);
    }

    #[test]
    fn agent_id_prefers_invocation_target() {
        use crate::invocation::{AgentRef, Invocation, Target};
        let inv = Invocation::single(Target {
            agent: AgentRef::named("worker"),
            message: crate::Message::user("go".into(), None),
            executor: None,
        });
        assert_eq!(
            task_agent_id(Some(&inv), Some("parent".into())).as_deref(),
            Some("worker")
        );
        assert_eq!(
            task_agent_id(None, Some("parent".into())).as_deref(),
            Some("parent")
        );
    }

    #[test]
    fn query_page_size_is_clamped() {
        assert_eq!(RunListQuery::default().page_size(), DEFAULT_RUN_PAGE);
        let q = RunListQuery {
            limit: Some(10_000),
            ..Default::default()
        };
        assert_eq!(q.page_size(), MAX_RUN_PAGE);
        let q = RunListQuery {
            limit: Some(0),
            ..Default::default()
        };
        assert_eq!(q.page_size(), 1);
    }

    #[test]
    fn run_state_wire_names() {
        assert_eq!(
            serde_json::to_string(&RunState::NeedsInput).unwrap(),
            "\"needs_input\""
        );
    }
}
