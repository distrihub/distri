//! The `distri sessions` state machine. No terminal, no network: it takes
//! [`Input`]s (keys, loaded runs, stream items, results of effects) and
//! returns [`Effect`]s for the event loop in `mod.rs` to carry out. The
//! renderer in `ui.rs` only reads it. That split is what lets the tests at
//! the bottom drive list → attach → answer → detach without a TTY.

use std::collections::HashSet;
use std::sync::Arc;

use distri::{EventPrinter, StreamItem};
use distri_types::{
    AgentEvent, AgentEventType, RunListQuery, RunListResponse, RunState, RunSummary, ToolCall,
};

use super::transcript::Transcript;

/// The approval tool's name. Its CLI handler reads stdin, which the TUI owns,
/// so the switcher always asks for the answer as text.
const APPROVAL_TOOL: &str = "approval_request";

/// Keys the state machine understands, independent of the terminal library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    PageUp,
    PageDown,
    Enter,
    Esc,
    Backspace,
    Char(char),
    CtrlC,
}

#[derive(Debug, Clone)]
pub enum Input {
    Key(Key),
    /// Poll timer: time to refresh the list.
    Tick,
    RunsLoaded(RunListResponse),
    LoadFailed(String),
    /// One item from the attached run's `tasks/resubscribe` stream.
    Stream {
        task_id: String,
        item: Box<StreamItem>,
    },
    /// The attached stream closed (the run finished, or the connection dropped).
    StreamEnded {
        task_id: String,
        error: Option<String>,
    },
    AnswerDone {
        tool_call_id: String,
        error: Option<String>,
    },
    CancelDone {
        task_id: String,
        error: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub enum Effect {
    LoadRuns(RunListQuery),
    /// Open `tasks/resubscribe` for `task_id` (history replay, then live).
    Attach {
        agent_id: String,
        task_id: String,
    },
    /// Drop the stream. The run keeps going on the server.
    Detach {
        task_id: String,
    },
    /// Run a pending external call through the local `ExternalToolRegistry`.
    RunTool {
        agent_id: String,
        event: Box<AgentEvent>,
        call: ToolCall,
    },
    /// Post what the person typed as the tool result.
    SendText {
        agent_id: String,
        call: ToolCall,
        text: String,
    },
    /// A2A `tasks/cancel` (cascades to the run's children).
    Cancel {
        agent_id: String,
        task_id: String,
    },
    Quit,
}

/// `RunTool` compares by agent and call; the carried event is context for
/// the handler, not identity.
impl PartialEq for Effect {
    fn eq(&self, other: &Self) -> bool {
        use Effect::*;
        match (self, other) {
            (LoadRuns(a), LoadRuns(b)) => a == b,
            (
                Attach {
                    agent_id: a1,
                    task_id: t1,
                },
                Attach {
                    agent_id: a2,
                    task_id: t2,
                },
            ) => a1 == a2 && t1 == t2,
            (Detach { task_id: a }, Detach { task_id: b }) => a == b,
            (
                RunTool {
                    agent_id: a1,
                    call: c1,
                    ..
                },
                RunTool {
                    agent_id: a2,
                    call: c2,
                    ..
                },
            ) => a1 == a2 && c1 == c2,
            (
                SendText {
                    agent_id: a1,
                    call: c1,
                    text: x1,
                },
                SendText {
                    agent_id: a2,
                    call: c2,
                    text: x2,
                },
            ) => a1 == a2 && c1 == c2 && x1 == x2,
            (
                Cancel {
                    agent_id: a1,
                    task_id: t1,
                },
                Cancel {
                    agent_id: a2,
                    task_id: t2,
                },
            ) => a1 == a2 && t1 == t2,
            (Quit, Quit) => true,
            _ => false,
        }
    }
}

/// Which runs the list shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    All,
    Thread(String),
}

/// What keys currently do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Browse,
    /// Typing an answer to `tool_call_id`.
    Answer {
        tool_call_id: String,
        text: String,
    },
    /// `x` pressed; `y` confirms cancelling `task_id`.
    ConfirmCancel {
        task_id: String,
    },
}

/// A tool call on the attached run that has not finished yet.
#[derive(Debug, Clone)]
pub struct PendingCall {
    pub call: ToolCall,
    /// The event that carried the call (handlers get it as context).
    pub event: AgentEvent,
    /// The local registry has a handler for it.
    pub runnable: bool,
    /// We already sent an answer and are waiting for the run to move on.
    pub answered: bool,
}

pub struct Attached {
    pub task_id: String,
    pub agent_id: String,
    pub transcript: Transcript,
    printer: EventPrinter,
    /// Calls seen in `ToolCalls` that have no `ToolExecutionEnd`/`ToolResults` yet.
    pub pending: Vec<PendingCall>,
    pub streaming: bool,
    /// Lines scrolled up from the bottom (0 = follow).
    pub scroll: u16,
}

impl Attached {
    fn new(task_id: String, agent_id: String) -> Self {
        let transcript = Transcript::default();
        let printer = EventPrinter::new()
            .with_agent_name(agent_id.clone())
            .with_output(Box::new(transcript.writer()));
        Self {
            task_id,
            agent_id,
            transcript,
            printer,
            pending: Vec::new(),
            streaming: true,
            scroll: 0,
        }
    }

    /// The first call a person can answer now.
    pub fn answerable(&self) -> Option<&PendingCall> {
        self.pending.iter().find(|p| !p.answered)
    }
}

/// One line of the run list: a node of the tree at `depth`.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub depth: usize,
    pub task_id: String,
}

pub struct SessionsState {
    pub runs: Vec<RunSummary>,
    pub has_more: bool,
    pub rows: Vec<Row>,
    pub selected: usize,
    pub scope: Scope,
    /// The thread the picker was opened from, so `t` can toggle back to it.
    pub home_thread: Option<String>,
    pub attached: Option<Attached>,
    pub mode: Mode,
    pub status: Option<String>,
    pub loaded: bool,
    can_run: Arc<dyn Fn(&str) -> bool + Send + Sync>,
}

impl SessionsState {
    /// `can_run(tool)` says whether the local `ExternalToolRegistry` has a
    /// handler for `tool`.
    pub fn new(scope: Scope, can_run: Arc<dyn Fn(&str) -> bool + Send + Sync>) -> Self {
        let home_thread = match &scope {
            Scope::Thread(t) => Some(t.clone()),
            Scope::All => None,
        };
        Self {
            runs: Vec::new(),
            has_more: false,
            rows: Vec::new(),
            selected: 0,
            scope,
            home_thread,
            attached: None,
            mode: Mode::Browse,
            status: None,
            loaded: false,
            can_run,
        }
    }

    /// Effects to run when the switcher opens.
    pub fn start(&self) -> Vec<Effect> {
        vec![Effect::LoadRuns(self.query())]
    }

    pub fn query(&self) -> RunListQuery {
        RunListQuery {
            thread_id: match &self.scope {
                Scope::Thread(t) => Some(t.clone()),
                Scope::All => None,
            },
            limit: Some(50),
            ..Default::default()
        }
    }

    pub fn selected_run(&self) -> Option<&RunSummary> {
        let row = self.rows.get(self.selected)?;
        self.find(&row.task_id)
    }

    pub fn find(&self, task_id: &str) -> Option<&RunSummary> {
        self.runs.iter().find_map(|r| r.find(task_id))
    }

    /// State of the attached run, from the latest list poll.
    pub fn attached_state(&self) -> Option<RunState> {
        let a = self.attached.as_ref()?;
        self.find(&a.task_id).map(|r| r.state)
    }

    pub async fn handle(&mut self, input: Input) -> Vec<Effect> {
        match input {
            Input::Key(key) => self.on_key(key),
            Input::Tick => vec![Effect::LoadRuns(self.query())],
            Input::RunsLoaded(resp) => {
                self.set_runs(resp);
                vec![]
            }
            Input::LoadFailed(err) => {
                self.status = Some(format!("could not load runs: {err}"));
                vec![]
            }
            Input::Stream { task_id, item } => {
                self.on_stream(&task_id, item).await;
                vec![]
            }
            Input::StreamEnded { task_id, error } => {
                if let Some(a) = self.attached.as_mut().filter(|a| a.task_id == task_id) {
                    a.streaming = false;
                    a.pending.clear();
                    self.status = Some(match error {
                        Some(e) => format!("stream closed: {e}"),
                        None => "run finished".to_string(),
                    });
                }
                vec![Effect::LoadRuns(self.query())]
            }
            Input::AnswerDone {
                tool_call_id,
                error,
            } => {
                if let Some(a) = self.attached.as_mut() {
                    if let Some(p) = a
                        .pending
                        .iter_mut()
                        .find(|p| p.call.tool_call_id == tool_call_id)
                    {
                        p.answered = error.is_none();
                    }
                }
                self.status = Some(match error {
                    Some(e) => format!("answer failed: {e}"),
                    None => "answer sent".to_string(),
                });
                vec![Effect::LoadRuns(self.query())]
            }
            Input::CancelDone { task_id, error } => {
                self.status = Some(match error {
                    Some(e) => format!("cancel failed: {e}"),
                    None => format!("canceled {}", short(&task_id)),
                });
                vec![Effect::LoadRuns(self.query())]
            }
        }
    }

    fn set_runs(&mut self, resp: RunListResponse) {
        let keep = self.rows.get(self.selected).map(|r| r.task_id.clone());
        self.runs = resp.runs;
        self.has_more = resp.has_more;
        self.loaded = true;
        self.rows = self
            .runs
            .iter()
            .flat_map(|r| {
                r.flatten().into_iter().map(|(depth, n)| Row {
                    depth,
                    task_id: n.task_id.clone(),
                })
            })
            .collect();
        // Keep the cursor on the same run across refreshes.
        self.selected = keep
            .and_then(|id| self.rows.iter().position(|r| r.task_id == id))
            .unwrap_or(0)
            .min(self.rows.len().saturating_sub(1));
    }

    async fn on_stream(&mut self, task_id: &str, item: Box<StreamItem>) {
        let can_run = self.can_run.clone();
        let Some(a) = self.attached.as_mut().filter(|a| a.task_id == task_id) else {
            return; // a late item from a stream we already left
        };
        if let Some(event) = &item.agent_event {
            match &event.event {
                AgentEventType::ToolCalls { tool_calls, .. } => {
                    for call in tool_calls {
                        if a.pending
                            .iter()
                            .any(|p| p.call.tool_call_id == call.tool_call_id)
                        {
                            continue;
                        }
                        a.pending.push(PendingCall {
                            call: call.clone(),
                            event: event.clone(),
                            runnable: call.tool_name != APPROVAL_TOOL && can_run(&call.tool_name),
                            answered: false,
                        });
                    }
                }
                AgentEventType::ToolExecutionEnd { tool_call_id, .. } => {
                    a.pending.retain(|p| &p.call.tool_call_id != tool_call_id);
                }
                AgentEventType::ToolResults { results, .. } => {
                    let done: HashSet<&str> =
                        results.iter().map(|r| r.tool_call_id.as_str()).collect();
                    a.pending
                        .retain(|p| !done.contains(p.call.tool_call_id.as_str()));
                }
                AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. }
                    if event.task_id == a.task_id =>
                {
                    a.pending.clear();
                }
                _ => {}
            }
            a.printer.handle_event(event).await;
        }
        if let Some(msg) = &item.message {
            a.printer.handle_final_message(msg);
        }
    }

    fn on_key(&mut self, key: Key) -> Vec<Effect> {
        if key == Key::CtrlC {
            return self.quit();
        }
        match self.mode.clone() {
            Mode::Answer { tool_call_id, text } => self.on_answer_key(key, tool_call_id, text),
            Mode::ConfirmCancel { task_id } => {
                self.mode = Mode::Browse;
                if key == Key::Char('y') {
                    if let Some(agent_id) = self.agent_of(&task_id) {
                        self.status = Some(format!("canceling {}…", short(&task_id)));
                        return vec![Effect::Cancel { agent_id, task_id }];
                    }
                }
                self.status = Some("cancel aborted".to_string());
                vec![]
            }
            Mode::Browse => self.on_browse_key(key),
        }
    }

    fn on_browse_key(&mut self, key: Key) -> Vec<Effect> {
        match key {
            Key::Up | Key::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                vec![]
            }
            Key::Down | Key::Char('j') => {
                if self.selected + 1 < self.rows.len() {
                    self.selected += 1;
                }
                vec![]
            }
            Key::PageUp => {
                if let Some(a) = self.attached.as_mut() {
                    a.scroll = a.scroll.saturating_add(10);
                }
                vec![]
            }
            Key::PageDown => {
                if let Some(a) = self.attached.as_mut() {
                    a.scroll = a.scroll.saturating_sub(10);
                }
                vec![]
            }
            Key::Enter => self.attach_selected(),
            Key::Esc => match self.attached.take() {
                Some(a) => {
                    self.status = Some(format!(
                        "detached from {} — it keeps running",
                        short(&a.task_id)
                    ));
                    vec![Effect::Detach { task_id: a.task_id }]
                }
                None => self.quit(),
            },
            Key::Char('q') => self.quit(),
            Key::Char('r') => vec![Effect::LoadRuns(self.query())],
            Key::Char('t') => {
                self.scope = match (&self.scope, &self.home_thread) {
                    (Scope::All, Some(t)) => Scope::Thread(t.clone()),
                    _ => Scope::All,
                };
                vec![Effect::LoadRuns(self.query())]
            }
            Key::Char('x') => {
                let target = self
                    .attached
                    .as_ref()
                    .map(|a| a.task_id.clone())
                    .or_else(|| self.rows.get(self.selected).map(|r| r.task_id.clone()));
                match target {
                    Some(task_id)
                        if self.find(&task_id).is_some_and(|r| !r.state.is_terminal()) =>
                    {
                        self.status =
                            Some(format!("cancel {} and its children? y/n", short(&task_id)));
                        self.mode = Mode::ConfirmCancel { task_id };
                    }
                    Some(_) => self.status = Some("that run has already finished".to_string()),
                    None => {}
                }
                vec![]
            }
            Key::Char('a') => self.answer(),
            _ => vec![],
        }
    }

    fn on_answer_key(&mut self, key: Key, tool_call_id: String, mut text: String) -> Vec<Effect> {
        match key {
            Key::Esc => {
                self.mode = Mode::Browse;
                vec![]
            }
            Key::Backspace => {
                text.pop();
                self.mode = Mode::Answer { tool_call_id, text };
                vec![]
            }
            Key::Char(c) => {
                text.push(c);
                self.mode = Mode::Answer { tool_call_id, text };
                vec![]
            }
            Key::Enter if !text.trim().is_empty() => {
                self.mode = Mode::Browse;
                let Some(a) = self.attached.as_ref() else {
                    return vec![];
                };
                let Some(p) = a
                    .pending
                    .iter()
                    .find(|p| p.call.tool_call_id == tool_call_id)
                else {
                    self.status = Some("that call is no longer waiting".to_string());
                    return vec![];
                };
                self.status = Some(format!("sending answer to {}…", p.call.tool_name));
                vec![Effect::SendText {
                    agent_id: answer_agent(a, p),
                    call: p.call.clone(),
                    text,
                }]
            }
            _ => {
                self.mode = Mode::Answer { tool_call_id, text };
                vec![]
            }
        }
    }

    /// `a`: answer the first waiting call on the attached run — run it
    /// through the registry when we have a handler, else ask for text.
    fn answer(&mut self) -> Vec<Effect> {
        let Some(a) = self.attached.as_ref() else {
            self.status = Some("attach to a run first (enter)".to_string());
            return vec![];
        };
        let Some(p) = a.answerable() else {
            self.status = Some("nothing is waiting on you here".to_string());
            return vec![];
        };
        if p.runnable {
            self.status = Some(format!("running {} locally…", p.call.tool_name));
            vec![Effect::RunTool {
                agent_id: answer_agent(a, p),
                event: Box::new(p.event.clone()),
                call: p.call.clone(),
            }]
        } else {
            self.mode = Mode::Answer {
                tool_call_id: p.call.tool_call_id.clone(),
                text: String::new(),
            };
            vec![]
        }
    }

    fn attach_selected(&mut self) -> Vec<Effect> {
        let Some(run) = self.selected_run() else {
            return vec![];
        };
        let task_id = run.task_id.clone();
        let Some(agent_id) = run.agent_id.clone() else {
            self.status = Some("this run has no agent to attach through".to_string());
            return vec![];
        };
        let mut effects = Vec::new();
        if let Some(prev) = self.attached.take() {
            if prev.task_id == task_id {
                self.attached = Some(prev);
                return effects;
            }
            effects.push(Effect::Detach {
                task_id: prev.task_id,
            });
        }
        self.status = Some(format!("attached to {} ({agent_id})", short(&task_id)));
        self.attached = Some(Attached::new(task_id.clone(), agent_id.clone()));
        effects.push(Effect::Attach { agent_id, task_id });
        effects
    }

    fn agent_of(&self, task_id: &str) -> Option<String> {
        self.find(task_id)
            .and_then(|r| r.agent_id.clone())
            .or_else(|| {
                self.attached
                    .as_ref()
                    .filter(|a| a.task_id == task_id)
                    .map(|a| a.agent_id.clone())
            })
    }

    fn quit(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        if let Some(a) = self.attached.take() {
            effects.push(Effect::Detach { task_id: a.task_id });
        }
        effects.push(Effect::Quit);
        effects
    }
}

/// The agent whose `complete-tool` route takes the answer: the one that
/// made the call, else the attached run's agent.
fn answer_agent(a: &Attached, p: &PendingCall) -> String {
    if p.event.agent_id.is_empty() {
        a.agent_id.clone()
    } else {
        p.event.agent_id.clone()
    }
}

/// First 8 characters of an id, for status lines.
pub fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::runs::fixture;

    fn item(event: AgentEvent) -> StreamItem {
        StreamItem {
            message: None,
            agent_event: Some(event),
        }
    }

    fn state(can_run: &'static [&'static str]) -> SessionsState {
        SessionsState::new(Scope::All, Arc::new(move |t: &str| can_run.contains(&t)))
    }

    async fn loaded(can_run: &'static [&'static str]) -> SessionsState {
        let mut s = state(can_run);
        assert_eq!(s.start(), vec![Effect::LoadRuns(s.query())]);
        s.handle(Input::RunsLoaded(fixture::multi_run())).await;
        s
    }

    #[tokio::test]
    async fn lists_the_tree_with_the_waiting_child_first() {
        let s = loaded(&[]).await;
        let ids: Vec<(usize, &str)> = s
            .rows
            .iter()
            .map(|r| (r.depth, r.task_id.as_str()))
            .collect();
        assert_eq!(
            ids,
            vec![
                (0, fixture::ROOT_TASK_ID),
                (1, fixture::ASKER_TASK_ID),
                (1, fixture::WORKER_TASK_ID),
            ]
        );
        assert!(s.selected_run().unwrap().needs_input);
    }

    /// list → attach → answer → detach, against the fixture's stubbed
    /// `tasks/resubscribe` stream for the waiting child.
    #[tokio::test]
    async fn list_attach_answer_detach() {
        let mut s = loaded(&[]).await;

        // Down to the asker, Enter attaches through its agent.
        s.handle(Input::Key(Key::Down)).await;
        let fx = s.handle(Input::Key(Key::Enter)).await;
        assert_eq!(
            fx,
            vec![Effect::Attach {
                agent_id: fixture::ASKER_AGENT.into(),
                task_id: fixture::ASKER_TASK_ID.into(),
            }]
        );

        // History replay arrives; the pending ask_human call is tracked and
        // the transcript is the `distri run` rendering.
        for e in fixture::pending_ask_events() {
            s.handle(Input::Stream {
                task_id: fixture::ASKER_TASK_ID.into(),
                item: Box::new(item(e)),
            })
            .await;
        }
        let a = s.attached.as_ref().unwrap();
        let pending = a.answerable().expect("ask_human waits on a person");
        assert_eq!(pending.call.tool_name, fixture::ASK_TOOL);
        assert!(!pending.runnable, "no local handler: the person answers");
        let text = a.transcript.plain_text();
        assert!(
            text.contains("I need the release owner to decide."),
            "{text}"
        );
        assert!(text.contains(fixture::ASK_TOOL), "{text}");
        assert_eq!(s.attached_state(), Some(RunState::NeedsInput));

        // `a` opens the answer box; typing and Enter sends it.
        assert!(s.handle(Input::Key(Key::Char('a'))).await.is_empty());
        assert!(matches!(s.mode, Mode::Answer { .. }));
        for c in "yes, ship".chars() {
            s.handle(Input::Key(Key::Char(c))).await;
        }
        let fx = s.handle(Input::Key(Key::Enter)).await;
        assert_eq!(
            fx,
            vec![Effect::SendText {
                agent_id: fixture::ASKER_AGENT.into(),
                call: fixture::ask_call(),
                text: "yes, ship".into(),
            }]
        );
        assert_eq!(s.mode, Mode::Browse);

        // The server confirms, then the run moves on: the call clears.
        let fx = s
            .handle(Input::AnswerDone {
                tool_call_id: fixture::ASK_CALL_ID.into(),
                error: None,
            })
            .await;
        assert!(matches!(fx.as_slice(), [Effect::LoadRuns(_)]));
        assert!(s.attached.as_ref().unwrap().answerable().is_none());
        let mut end = fixture::pending_ask_events().pop().unwrap();
        end.event = AgentEventType::ToolExecutionEnd {
            step_id: "step-1".into(),
            tool_call_id: fixture::ASK_CALL_ID.into(),
            tool_call_name: fixture::ASK_TOOL.into(),
            success: true,
        };
        s.handle(Input::Stream {
            task_id: fixture::ASKER_TASK_ID.into(),
            item: Box::new(item(end)),
        })
        .await;
        assert!(s.attached.as_ref().unwrap().pending.is_empty());

        // Esc detaches; the list keeps its cursor and keeps polling.
        let fx = s.handle(Input::Key(Key::Esc)).await;
        assert_eq!(
            fx,
            vec![Effect::Detach {
                task_id: fixture::ASKER_TASK_ID.into()
            }]
        );
        assert!(s.attached.is_none());
        assert_eq!(s.selected_run().unwrap().task_id, fixture::ASKER_TASK_ID);
        assert!(matches!(
            s.handle(Input::Tick).await.as_slice(),
            [Effect::LoadRuns(_)]
        ));
        // A late item from the old stream is ignored.
        s.handle(Input::Stream {
            task_id: fixture::ASKER_TASK_ID.into(),
            item: Box::new(item(fixture::pending_ask_events().remove(0))),
        })
        .await;
        assert!(s.attached.is_none());

        // Esc again leaves the switcher.
        assert_eq!(s.handle(Input::Key(Key::Esc)).await, vec![Effect::Quit]);
    }

    #[tokio::test]
    async fn runnable_calls_go_through_the_registry() {
        let mut s = loaded(&["ask_human"]).await;
        s.handle(Input::Key(Key::Down)).await;
        s.handle(Input::Key(Key::Enter)).await;
        for e in fixture::pending_ask_events() {
            s.handle(Input::Stream {
                task_id: fixture::ASKER_TASK_ID.into(),
                item: Box::new(item(e)),
            })
            .await;
        }
        let fx = s.handle(Input::Key(Key::Char('a'))).await;
        match fx.as_slice() {
            [Effect::RunTool { agent_id, call, .. }] => {
                assert_eq!(agent_id, fixture::ASKER_AGENT);
                assert_eq!(call.tool_call_id, fixture::ASK_CALL_ID);
            }
            other => panic!("expected RunTool, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn switching_runs_detaches_the_previous_stream() {
        let mut s = loaded(&[]).await;
        s.handle(Input::Key(Key::Down)).await;
        s.handle(Input::Key(Key::Enter)).await;
        s.handle(Input::Key(Key::Up)).await;
        let fx = s.handle(Input::Key(Key::Enter)).await;
        assert_eq!(
            fx,
            vec![
                Effect::Detach {
                    task_id: fixture::ASKER_TASK_ID.into()
                },
                Effect::Attach {
                    agent_id: fixture::ROOT_AGENT.into(),
                    task_id: fixture::ROOT_TASK_ID.into(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn cancel_needs_confirmation_and_skips_finished_runs() {
        let mut s = loaded(&[]).await;
        // Root is done: nothing to cancel.
        assert!(s.handle(Input::Key(Key::Char('x'))).await.is_empty());
        assert_eq!(s.mode, Mode::Browse);

        s.handle(Input::Key(Key::Down)).await; // asker, waiting
        assert!(s.handle(Input::Key(Key::Char('x'))).await.is_empty());
        assert!(matches!(s.mode, Mode::ConfirmCancel { .. }));
        let fx = s.handle(Input::Key(Key::Char('y'))).await;
        assert_eq!(
            fx,
            vec![Effect::Cancel {
                agent_id: fixture::ASKER_AGENT.into(),
                task_id: fixture::ASKER_TASK_ID.into(),
            }]
        );

        s.handle(Input::Key(Key::Char('x'))).await;
        assert!(s.handle(Input::Key(Key::Char('n'))).await.is_empty());
        assert_eq!(s.mode, Mode::Browse);
    }

    #[tokio::test]
    async fn refresh_keeps_the_cursor_and_scope_toggles() {
        let mut s = SessionsState::new(
            Scope::Thread(fixture::THREAD_ID.into()),
            Arc::new(|_: &str| false),
        );
        assert_eq!(s.query().thread_id.as_deref(), Some(fixture::THREAD_ID));
        s.handle(Input::RunsLoaded(fixture::multi_run())).await;
        s.handle(Input::Key(Key::Down)).await;
        s.handle(Input::Key(Key::Down)).await;
        s.handle(Input::RunsLoaded(fixture::multi_run())).await;
        assert_eq!(s.selected_run().unwrap().task_id, fixture::WORKER_TASK_ID);

        let fx = s.handle(Input::Key(Key::Char('t'))).await;
        assert!(matches!(&fx[..], [Effect::LoadRuns(q)] if q.thread_id.is_none()));
        let fx = s.handle(Input::Key(Key::Char('t'))).await;
        assert!(
            matches!(&fx[..], [Effect::LoadRuns(q)] if q.thread_id.as_deref() == Some(fixture::THREAD_ID))
        );
    }

    #[tokio::test]
    async fn approval_is_always_answered_by_the_person() {
        let mut s = loaded(&["approval_request"]).await;
        s.handle(Input::Key(Key::Down)).await;
        s.handle(Input::Key(Key::Enter)).await;
        let mut e = fixture::pending_ask_events().remove(5);
        e.event = AgentEventType::ToolCalls {
            step_id: "s".into(),
            parent_message_id: None,
            tool_calls: vec![ToolCall {
                tool_call_id: "approve-1".into(),
                tool_name: APPROVAL_TOOL.into(),
                input: serde_json::Value::Null,
            }],
        };
        s.handle(Input::Stream {
            task_id: fixture::ASKER_TASK_ID.into(),
            item: Box::new(item(e)),
        })
        .await;
        assert!(s.handle(Input::Key(Key::Char('a'))).await.is_empty());
        assert!(matches!(s.mode, Mode::Answer { .. }));
    }
}
