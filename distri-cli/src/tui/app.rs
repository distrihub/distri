//! The TUI's state machine. No terminal, no network: it takes [`Input`]s
//! (keys, stream items, questions from the broker, run listings) and returns
//! [`Effect`]s for the runtime in `mod.rs` to carry out. `view.rs` draws it.
//! Finished cells come out of [`App::take_history`] for the scrollback.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use distri::StreamItem;
use distri_types::{AgentEvent, RunListQuery, RunListResponse, RunState, ToolCall};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line;
use tokio::sync::oneshot;

use distri_cli::interact::{question_of, Ask, Interaction, Reply};
use distri_cli::local_events::LocalToolEvent;

use super::cells::{Cell, PendingCall, RenderOpts, Transcript};
use super::editor::Editor;
use super::runs::RunsOverlay;

/// A non-local call with no execution start after this long is waiting on
/// a person (when the server cannot say so through `/v1/runs`).
const ASK_AFTER: Duration = Duration::from_millis(2500);
/// Wait this long before running a followed run's pending call locally, so a
/// history replay that already holds its result can catch up.
const RUN_AFTER: Duration = Duration::from_millis(600);
const DOUBLE_PRESS: Duration = Duration::from_millis(1500);

#[derive(Debug)]
pub enum Input {
    Key(KeyEvent),
    Paste(String),
    Resize(u16, u16),
    Tick,
    Turn {
        turn: u64,
        item: Box<StreamItem>,
    },
    TurnEnded {
        turn: u64,
        error: Option<String>,
    },
    Follow {
        task_id: String,
        item: Box<StreamItem>,
    },
    FollowEnded {
        task_id: String,
        error: Option<String>,
    },
    Ask(Ask),
    Local(LocalToolEvent),
    RunsLoaded(Result<RunListResponse, String>),
    AnswerDone {
        call_id: String,
        error: Option<String>,
    },
    CancelDone {
        task_id: String,
        error: Option<String>,
    },
    TurnFailedToStart {
        turn: u64,
        error: String,
    },
}

#[derive(Debug, Clone)]
pub enum Effect {
    SendTurn {
        turn: u64,
        text: String,
    },
    Cancel {
        agent_id: String,
        task_id: String,
    },
    Follow {
        agent_id: String,
        task_id: String,
    },
    RunTool {
        agent_id: String,
        event: Box<AgentEvent>,
        call: ToolCall,
    },
    SendText {
        agent_id: String,
        call: ToolCall,
        text: String,
    },
    LoadRuns(RunListQuery),
    /// A command the app does not handle itself (it runs outside the TUI).
    Slash(String),
    CycleMode,
    Quit,
}

/// What the person is being asked now.
pub enum Prompt {
    Broker {
        interaction: Interaction,
        reply: Option<oneshot::Sender<Reply>>,
    },
    Answer {
        agent_id: String,
        task_id: String,
        call: ToolCall,
        question: String,
    },
}

impl Prompt {
    /// Options for a select prompt; empty for a text answer.
    pub fn options(&self) -> Vec<String> {
        match self {
            Prompt::Broker { interaction, .. } => match interaction {
                Interaction::Approve { .. } => {
                    vec!["Yes".into(), "No, and tell the agent why (esc)".into()]
                }
                Interaction::Grant { .. } => vec![
                    "Yes, for this call".into(),
                    "Yes, and remember for this project".into(),
                    "No, and tell the agent why (esc)".into(),
                ],
                Interaction::Permit { rule_hint, .. } => {
                    let mut o = vec!["Yes".into()];
                    if let Some(r) = rule_hint {
                        o.push(format!("Yes, and don't ask again for {r}"));
                    }
                    o.push("No, and tell the agent what to do instead (esc)".into());
                    o
                }
                Interaction::Answer { .. } => Vec::new(),
            },
            Prompt::Answer { .. } => Vec::new(),
        }
    }

    pub fn is_text(&self) -> bool {
        self.options().is_empty()
    }

    fn task_id(&self) -> Option<&str> {
        match self {
            Prompt::Answer { task_id, .. } => Some(task_id),
            _ => None,
        }
    }
}

pub struct TurnState {
    pub id: u64,
    pub started: Instant,
    pub agent_id: Option<String>,
    pub interrupting: bool,
}

pub struct Followed {
    pub agent_id: String,
    pub transcript: Transcript,
    pub live: bool,
    /// Run its CLI tool calls here (a background child of this chat).
    pub auto: bool,
    pub announced: bool,
    pub title: Option<String>,
}

pub struct Popup {
    pub items: Vec<(String, String)>,
    pub selected: usize,
    pub kind: PopupKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKind {
    Command,
    File,
}

pub type ToolPredicate = Arc<dyn Fn(&str) -> bool + Send + Sync>;

pub struct App {
    pub agent: String,
    pub model: Option<String>,
    pub thread_id: String,
    pub editor: Editor,
    pub chat: Transcript,
    pub turn: Option<TurnState>,
    next_turn: u64,
    pub queue: VecDeque<String>,
    pub followed: HashMap<String, Followed>,
    /// `None`: the chat. `Some(task)`: a followed run.
    pub focus: Option<String>,
    pub prompts: VecDeque<Prompt>,
    pub prompt_sel: usize,
    /// Typing a reason (after "No") or an answer.
    pub prompt_text: Option<Editor>,
    pub overlay: Option<RunsOverlay>,
    pub runs: Option<RunListResponse>,
    /// `Some(false)` once the server said it has no `/v1/runs`.
    pub runs_supported: Option<bool>,
    history: Vec<Line<'static>>,
    pub status: Option<(String, Instant)>,
    last_ctrl_c: Option<Instant>,
    last_esc: Option<Instant>,
    pub mode_label: String,
    pub status_line: String,
    pub frame: usize,
    pub width: u16,
    pub height: u16,
    can_run: ToolPredicate,
    is_script: ToolPredicate,
    pub commands: Vec<(String, String)>,
    pub files: Vec<String>,
    pub popup: Option<Popup>,
    pub expand_next: bool,
    pub quit: bool,
}

pub struct AppInit {
    pub agent: String,
    pub model: Option<String>,
    pub thread_id: String,
    pub history: Vec<String>,
    pub can_run: ToolPredicate,
    pub is_script: ToolPredicate,
    pub commands: Vec<(String, String)>,
    pub files: Vec<String>,
    pub mode_label: String,
    pub width: u16,
    pub height: u16,
}

impl App {
    pub fn new(init: AppInit) -> Self {
        let chat = transcript_for(None, &init.can_run, &init.is_script);
        Self {
            agent: init.agent,
            model: init.model,
            thread_id: init.thread_id,
            editor: Editor::new(init.history),
            chat,
            turn: None,
            next_turn: 1,
            queue: VecDeque::new(),
            followed: HashMap::new(),
            focus: None,
            prompts: VecDeque::new(),
            prompt_sel: 0,
            prompt_text: None,
            overlay: None,
            runs: None,
            runs_supported: None,
            history: Vec::new(),
            status: None,
            last_ctrl_c: None,
            last_esc: None,
            mode_label: init.mode_label,
            status_line: String::new(),
            frame: 0,
            width: init.width,
            height: init.height,
            can_run: init.can_run,
            is_script: init.is_script,
            commands: init.commands,
            files: init.files,
            popup: None,
            expand_next: false,
            quit: false,
        }
    }

    pub fn opts(&self) -> RenderOpts {
        RenderOpts {
            width: self.width,
            expand: self.expand_next,
            frame: self.frame,
        }
    }

    /// Lines to print into the scrollback now.
    pub fn take_history(&mut self) -> Vec<Line<'static>> {
        let opts = self.opts();
        match &self.focus {
            None => {
                let lines = self.chat.drain_commits(&opts);
                self.history.extend(lines);
            }
            Some(task) => {
                if let Some(f) = self.followed.get_mut(task) {
                    let lines = f.transcript.drain_commits(&opts);
                    self.history.extend(lines);
                }
            }
        }
        std::mem::take(&mut self.history)
    }

    pub fn set_status(&mut self, text: impl Into<String>) {
        self.status = Some((text.into(), Instant::now()));
    }

    pub fn working(&self) -> bool {
        self.turn.is_some()
    }

    /// The transcript the live region shows.
    pub fn focused(&self) -> &Transcript {
        match &self.focus {
            Some(t) => self
                .followed
                .get(t)
                .map(|f| &f.transcript)
                .unwrap_or(&self.chat),
            None => &self.chat,
        }
    }

    pub fn start(&self) -> Vec<Effect> {
        vec![Effect::LoadRuns(self.runs_query())]
    }

    pub fn runs_query(&self) -> RunListQuery {
        let thread = match &self.overlay {
            Some(o) if o.all => None,
            _ => Some(self.thread_id.clone()),
        };
        RunListQuery {
            thread_id: thread,
            limit: Some(50),
            ..Default::default()
        }
    }

    /// Counts for the footer: (working, needs you) over this chat's runs.
    pub fn run_counts(&self) -> (usize, usize) {
        let mut working = 0;
        let mut waiting = 0;
        for f in self.followed.values() {
            match f.transcript.finished {
                None if !f.transcript.pending.is_empty()
                    && f.transcript
                        .pending
                        .values()
                        .any(|p| !(self.can_run)(&p.call.tool_name)) =>
                {
                    waiting += 1
                }
                None if f.live => working += 1,
                _ => {}
            }
        }
        if let Some(runs) = &self.runs {
            for r in runs.runs.iter().flat_map(|r| r.flatten()) {
                let (_, n) = r;
                if self.followed.contains_key(&n.task_id) {
                    continue;
                }
                if Some(&n.task_id) == self.chat.root_task.as_ref() && self.turn.is_some() {
                    continue;
                }
                if n.state == RunState::NeedsInput {
                    waiting += 1;
                }
            }
        }
        (working, waiting)
    }

    pub fn handle(&mut self, input: Input) -> Vec<Effect> {
        match input {
            Input::Key(k) => self.on_key(k),
            Input::Paste(s) => {
                if let Some(e) = self.prompt_text.as_mut() {
                    e.paste(&s);
                } else if self.prompts.front().is_some_and(|p| p.is_text()) {
                    let mut e = Editor::new(Vec::new());
                    e.paste(&s);
                    self.prompt_text = Some(e);
                } else {
                    self.editor.paste(&s);
                }
                self.refresh_popup();
                vec![]
            }
            Input::Resize(w, h) => {
                self.width = w;
                self.height = h;
                vec![]
            }
            Input::Tick => self.on_tick(),
            Input::Turn { turn, item } => {
                if self.turn.as_ref().is_some_and(|t| t.id == turn) {
                    self.chat.apply(&item);
                }
                vec![]
            }
            Input::TurnEnded { turn, error } => self.on_turn_ended(turn, error),
            Input::TurnFailedToStart { turn, error } => {
                if self.turn.as_ref().is_some_and(|t| t.id == turn) {
                    self.turn = None;
                    self.chat.push(Cell::Error { text: error });
                }
                self.next_queued()
            }
            Input::Follow { task_id, item } => {
                if let Some(f) = self.followed.get_mut(&task_id) {
                    f.transcript.apply(&item);
                }
                self.sync_child(&task_id);
                vec![]
            }
            Input::FollowEnded { task_id, error } => {
                if let Some(f) = self.followed.get_mut(&task_id) {
                    f.live = false;
                    if let Some(e) = error {
                        f.transcript.push(Cell::Error {
                            text: format!("stream closed: {e}"),
                        });
                    }
                }
                self.sync_child(&task_id);
                vec![]
            }
            Input::Ask(ask) => {
                self.prompts.push_back(Prompt::Broker {
                    interaction: ask.interaction,
                    reply: Some(ask.reply),
                });
                vec![]
            }
            Input::Local(ev) => {
                if !self.chat.apply_local(&ev.tool_call_id, &ev.kind) {
                    for f in self.followed.values_mut() {
                        if f.transcript.apply_local(&ev.tool_call_id, &ev.kind) {
                            break;
                        }
                    }
                }
                vec![]
            }
            Input::RunsLoaded(res) => {
                match res {
                    Ok(r) => {
                        self.runs_supported = Some(true);
                        self.apply_run_states(&r);
                        if let Some(o) = self.overlay.as_mut() {
                            o.set_runs(&r);
                        }
                        self.runs = Some(r);
                    }
                    Err(e) => {
                        if e.contains("404") || e.contains("Not Found") {
                            if self.runs_supported.is_none() {
                                self.runs_supported = Some(false);
                            }
                        } else if self.overlay.is_some() {
                            self.set_status(format!("could not load runs: {e}"));
                        }
                    }
                }
                vec![]
            }
            Input::AnswerDone { call_id, error } => {
                if let Some(e) = error {
                    self.set_status(format!("answer failed: {e}"));
                    for f in self.followed.values_mut() {
                        if let Some(p) = f.transcript.pending.get_mut(&call_id) {
                            p.dispatched = false;
                        }
                    }
                } else {
                    self.set_status("answer sent");
                }
                vec![]
            }
            Input::CancelDone { task_id, error } => {
                self.set_status(match error {
                    Some(e) => format!("cancel failed: {e}"),
                    None => format!("canceled {}", short(&task_id)),
                });
                vec![Effect::LoadRuns(self.runs_query())]
            }
        }
    }

    fn apply_run_states(&mut self, r: &RunListResponse) {
        for (_, n) in r.runs.iter().flat_map(|r| r.flatten()) {
            self.chat.set_child_state(&n.task_id, n.state, None);
        }
    }

    /// A followed run's state flows into the chat's cell for that child; when
    /// a background child finishes, the chat says so.
    fn sync_child(&mut self, task_id: &str) {
        let Some(f) = self.followed.get_mut(task_id) else {
            return;
        };
        let Some(state) = f.transcript.finished else {
            return;
        };
        let answer = f.transcript.final_answer.clone().or_else(|| {
            f.transcript.cells.iter().rev().find_map(|c| match c {
                Cell::Assistant { md, .. } if !md.trim().is_empty() => Some(md.trim().to_string()),
                _ => None,
            })
        });
        let announce = f.auto && !f.announced;
        f.announced = true;
        let agent = f.agent_id.clone();
        let title = f.title.clone();
        self.chat.set_child_state(task_id, state, answer.clone());
        if announce {
            let label = match title {
                Some(t) => format!("“{t}” ({agent})"),
                None => agent,
            };
            let (icon, word) = match state {
                RunState::Done => ("✓", "finished"),
                RunState::Failed => ("✗", "failed"),
                RunState::Canceled => ("⊘", "was canceled"),
                _ => ("·", "stopped"),
            };
            let mut text = format!("{icon} background agent {label} {word}");
            if let Some(a) = answer {
                text.push_str(":\n");
                text.push_str(&a);
            }
            if self.focus.is_none() {
                self.chat.push(Cell::Notice { text });
            } else {
                self.set_status(format!("{icon} {label} {word}"));
                self.chat.push(Cell::Notice { text });
            }
        }
    }

    fn on_turn_ended(&mut self, turn: u64, error: Option<String>) -> Vec<Effect> {
        if !self.turn.as_ref().is_some_and(|t| t.id == turn) {
            return vec![];
        }
        let interrupted = self.turn.as_ref().is_some_and(|t| t.interrupting);
        self.turn = None;
        self.chat.close_open();
        if let Some(e) = error {
            if !interrupted {
                self.chat.push(Cell::Error { text: e });
            }
        }
        if interrupted && self.chat.finished.is_none() {
            self.chat.push(Cell::Notice {
                text: "Interrupted · what should the agent do instead?".into(),
            });
        }
        // Drop questions from the finished turn's stream.
        self.prompts.retain(|p| !matches!(p, Prompt::Answer { task_id, .. } if Some(task_id) == self.chat.root_task.as_ref()));
        let mut effects = Vec::new();
        // Background children keep running: follow them so their CLI tools
        // run here and their end is reported.
        for (task_id, agent_id) in self.chat.open_children() {
            if self.followed.contains_key(&task_id) || agent_id.is_empty() {
                continue;
            }
            effects.extend(self.follow(&task_id, &agent_id, true));
        }
        effects.push(Effect::LoadRuns(self.runs_query()));
        effects.extend(self.next_queued());
        effects
    }

    fn next_queued(&mut self) -> Vec<Effect> {
        if self.turn.is_some() {
            return vec![];
        }
        match self.queue.pop_front() {
            Some(text) => self.start_turn(text),
            None => vec![],
        }
    }

    pub fn follow(&mut self, task_id: &str, agent_id: &str, auto: bool) -> Vec<Effect> {
        if let Some(f) = self.followed.get_mut(task_id) {
            f.auto |= auto;
            if f.live {
                return vec![];
            }
            f.live = true;
            return vec![Effect::Follow {
                agent_id: f.agent_id.clone(),
                task_id: task_id.to_string(),
            }];
        }
        let title = self
            .runs
            .as_ref()
            .and_then(|r| r.runs.iter().find_map(|x| x.find(task_id)))
            .and_then(|n| n.title.clone())
            .or_else(|| self.chat.child_title(task_id));
        let transcript = transcript_for(Some(task_id.to_string()), &self.can_run, &self.is_script);
        self.followed.insert(
            task_id.to_string(),
            Followed {
                agent_id: agent_id.to_string(),
                transcript,
                live: true,
                auto,
                announced: false,
                title,
            },
        );
        vec![Effect::Follow {
            agent_id: agent_id.to_string(),
            task_id: task_id.to_string(),
        }]
    }

    fn start_turn(&mut self, text: String) -> Vec<Effect> {
        let id = self.next_turn;
        self.next_turn += 1;
        self.chat.begin_turn();
        self.chat.push(Cell::User { text: text.clone() });
        self.turn = Some(TurnState {
            id,
            started: Instant::now(),
            agent_id: None,
            interrupting: false,
        });
        self.expand_next = false;
        vec![Effect::SendTurn { turn: id, text }]
    }

    fn on_tick(&mut self) -> Vec<Effect> {
        self.frame = self.frame.wrapping_add(1);
        if self
            .status
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(6))
        {
            self.status = None;
        }
        let mut effects = Vec::new();
        let now = Instant::now();
        let needs_input: Vec<String> = self
            .runs
            .as_ref()
            .map(|r| {
                r.runs
                    .iter()
                    .flat_map(|x| x.flatten())
                    .filter(|(_, n)| n.state == RunState::NeedsInput)
                    .map(|(_, n)| n.task_id.clone())
                    .collect()
            })
            .unwrap_or_default();
        let runs_known = self.runs_supported == Some(true);
        // The server also emits `ToolExecutionStart` for external calls, so
        // "started" says nothing about who runs a call. A call this client's
        // registry can run is ours; any other call is waiting on a person when
        // the run says so (`needs_input`), or, without a run list, when it
        // never started.
        let waiting_on_person = |p: &PendingCall| {
            if runs_known {
                needs_input.contains(&p.task_id)
            } else {
                !p.exec_started && now.duration_since(p.seen) > ASK_AFTER
            }
        };
        let mut asks = Vec::new();
        // The chat's own stream runs local calls in the stream client.
        for p in self.chat.pending.values_mut() {
            if p.dispatched {
                continue;
            }
            if (self.can_run)(&p.call.tool_name) {
                p.dispatched = true;
                continue;
            }
            if waiting_on_person(p) {
                p.dispatched = true;
                asks.push(Prompt::Answer {
                    agent_id: p.agent_id.clone(),
                    task_id: p.task_id.clone(),
                    question: question_of(&p.call),
                    call: p.call.clone(),
                });
            }
        }
        // Tasks whose calls the live turn's stream already carries: running
        // them from a follow too would run them twice.
        let in_turn: Vec<String> = if self.turn.is_some() {
            self.chat
                .root_task
                .iter()
                .cloned()
                .chain(self.chat.children.iter().map(|(t, _)| t.clone()))
                .collect()
        } else {
            Vec::new()
        };
        // Followed runs: nobody else is attached to answer for them.
        for f in self.followed.values_mut() {
            for p in f.transcript.pending.values_mut() {
                if p.dispatched || now.duration_since(p.seen) < RUN_AFTER {
                    continue;
                }
                if (self.can_run)(&p.call.tool_name) {
                    if in_turn.contains(&p.task_id) {
                        continue;
                    }
                    p.dispatched = true;
                    effects.push(Effect::RunTool {
                        agent_id: agent_or(&p.agent_id, &f.agent_id),
                        event: Box::new(p.event.clone()),
                        call: p.call.clone(),
                    });
                } else if waiting_on_person(p) {
                    p.dispatched = true;
                    asks.push(Prompt::Answer {
                        agent_id: agent_or(&p.agent_id, &f.agent_id),
                        task_id: p.task_id.clone(),
                        question: question_of(&p.call),
                        call: p.call.clone(),
                    });
                }
            }
        }
        self.prompts.extend(asks);
        effects
    }

    // ----------------------------------------------------------------- keys

    fn on_key(&mut self, k: KeyEvent) -> Vec<Effect> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            return self.on_ctrl_c();
        }
        if !self.prompts.is_empty() {
            return self.on_prompt_key(k);
        }
        if self.overlay.is_some() {
            return self.on_overlay_key(k);
        }
        if self.editor.search.is_some() {
            return self.on_search_key(k);
        }
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        match k.code {
            KeyCode::Char('d') if ctrl => {
                if self.editor.is_empty() {
                    return self.do_quit();
                }
                self.editor.delete();
            }
            KeyCode::Char('t') if ctrl => return self.open_overlay(),
            KeyCode::Char('o') if ctrl => {
                self.expand_next = !self.expand_next;
                if let Some(lines) = self.focused().expanded_last_tool(&RenderOpts {
                    width: self.width,
                    expand: true,
                    frame: self.frame,
                }) {
                    self.history.push(Line::styled(
                        "── expanded ──",
                        ratatui::style::Style::new().fg(ratatui::style::Color::DarkGray),
                    ));
                    self.history.extend(lines);
                }
                self.set_status(if self.expand_next {
                    "tool output: expanded (ctrl+o)"
                } else {
                    "tool output: folded (ctrl+o)"
                });
            }
            KeyCode::Char('r') if ctrl => self.editor.search = Some(String::new()),
            KeyCode::Char('a') if ctrl => self.editor.home(),
            KeyCode::Char('e') if ctrl => self.editor.end(),
            KeyCode::Char('u') if ctrl => self.editor.kill_to_start(),
            KeyCode::Char('k') if ctrl => self.editor.kill_to_end(),
            KeyCode::Char('w') if ctrl => self.editor.delete_word(),
            KeyCode::Char('j') if ctrl => self.editor.newline(),
            KeyCode::Char('l') if ctrl => {}
            KeyCode::BackTab => return vec![Effect::CycleMode],
            KeyCode::Enter if shift || alt => self.editor.newline(),
            KeyCode::Enter => return self.on_enter(),
            KeyCode::Tab => {
                self.accept_popup();
            }
            KeyCode::Esc => return self.on_esc(),
            KeyCode::Up if self.popup.is_some() => {
                if let Some(p) = self.popup.as_mut() {
                    p.selected = p.selected.saturating_sub(1);
                }
                return vec![];
            }
            KeyCode::Down if self.popup.is_some() => {
                if let Some(p) = self.popup.as_mut() {
                    if p.selected + 1 < p.items.len() {
                        p.selected += 1;
                    }
                }
                return vec![];
            }
            KeyCode::Up => self.editor.up(),
            KeyCode::Down => self.editor.down(),
            KeyCode::Left if alt || ctrl => self.editor.word_left(),
            KeyCode::Right if alt || ctrl => self.editor.word_right(),
            KeyCode::Left => self.editor.left(),
            KeyCode::Right => self.editor.right(),
            KeyCode::Home => self.editor.home(),
            KeyCode::End => self.editor.end(),
            KeyCode::Backspace if alt => self.editor.delete_word(),
            KeyCode::Backspace => self.editor.backspace(),
            KeyCode::Delete => self.editor.delete(),
            KeyCode::Char(c) if !ctrl => self.editor.insert_char(c),
            _ => {}
        }
        self.refresh_popup();
        vec![]
    }

    fn on_ctrl_c(&mut self) -> Vec<Effect> {
        let double = self.last_ctrl_c.is_some_and(|t| t.elapsed() < DOUBLE_PRESS);
        self.last_ctrl_c = Some(Instant::now());
        if double {
            return self.do_quit();
        }
        if !self.prompts.is_empty() {
            return self.answer_prompt_deny(Some("interrupted".into()));
        }
        if self.overlay.is_some() {
            self.overlay = None;
            return vec![];
        }
        if !self.editor.is_empty() || self.editor.search.is_some() {
            self.editor.clear();
            self.popup = None;
            return vec![];
        }
        if self.working() {
            return self.interrupt();
        }
        self.set_status("press ctrl+c again to exit");
        vec![]
    }

    fn do_quit(&mut self) -> Vec<Effect> {
        // Nobody will answer what is still asked.
        while let Some(p) = self.prompts.pop_front() {
            if let Prompt::Broker {
                reply: Some(tx), ..
            } = p
            {
                let _ = tx.send(Reply::Unavailable);
            }
        }
        self.quit = true;
        vec![Effect::Quit]
    }

    fn on_esc(&mut self) -> Vec<Effect> {
        if self.popup.take().is_some() {
            return vec![];
        }
        if self.working() {
            return self.interrupt();
        }
        if self.focus.is_some() {
            return self.focus_chat();
        }
        let double = self.last_esc.is_some_and(|t| t.elapsed() < DOUBLE_PRESS);
        self.last_esc = Some(Instant::now());
        if double && self.editor.is_empty() {
            // Esc Esc: edit the last message again.
            if let Some(last) = self.chat.cells.iter().rev().find_map(|c| match c {
                Cell::User { text } => Some(text.clone()),
                _ => None,
            }) {
                self.editor.set_text(&last);
                self.set_status("editing your last message · enter sends it again");
            }
        } else if !self.editor.is_empty() {
            self.set_status("esc again to edit your last message · ctrl+c clears");
        }
        vec![]
    }

    /// Esc while working: cancel this turn's run (and its children).
    fn interrupt(&mut self) -> Vec<Effect> {
        let Some(turn) = self.turn.as_mut() else {
            return vec![];
        };
        if turn.interrupting {
            return vec![];
        }
        turn.interrupting = true;
        let agent = turn.agent_id.clone().unwrap_or_else(|| self.agent.clone());
        // Questions from this turn will not be answered now.
        let root = self.chat.root_task.clone();
        let mut keep = VecDeque::new();
        while let Some(p) = self.prompts.pop_front() {
            match p {
                Prompt::Broker {
                    reply: Some(tx), ..
                } => {
                    let _ = tx.send(Reply::Deny {
                        reason: Some("The user interrupted the run.".into()),
                    });
                }
                other if other.task_id().is_some() && other.task_id() != root.as_deref() => {
                    keep.push_back(other)
                }
                _ => {}
            }
        }
        self.prompts = keep;
        self.prompt_text = None;
        match root {
            Some(task_id) => {
                self.set_status("interrupting…");
                vec![Effect::Cancel {
                    agent_id: agent,
                    task_id,
                }]
            }
            None => {
                self.set_status("interrupting… (the run has not started yet)");
                vec![]
            }
        }
    }

    fn on_enter(&mut self) -> Vec<Effect> {
        // A trailing backslash continues on a new line.
        let text = self.editor.text();
        if text.ends_with('\\') {
            self.editor.backspace();
            self.editor.newline();
            return vec![];
        }
        if let Some(p) = &self.popup {
            if p.kind == PopupKind::File {
                self.accept_popup();
                return vec![];
            }
            let typed = self.editor.text();
            if let Some((cmd, _)) = p.items.get(p.selected) {
                if !self.commands.iter().any(|(c, _)| *c == typed) {
                    let cmd = cmd.clone();
                    self.editor.set_text(&cmd);
                }
            }
            self.popup = None;
        }
        if self.editor.is_empty() {
            return vec![];
        }
        let text = self.editor.submit();
        let trimmed = text.trim().to_string();
        if trimmed.starts_with('/') {
            return self.on_slash(&trimmed);
        }
        if self.working() {
            self.queue.push_back(trimmed);
            self.set_status("queued · sends when the agent finishes this turn");
            return vec![];
        }
        if self.focus.is_some() {
            let mut e = self.focus_chat();
            e.extend(self.start_turn(trimmed));
            return e;
        }
        self.start_turn(trimmed)
    }

    fn on_slash(&mut self, line: &str) -> Vec<Effect> {
        let mut parts = line.splitn(2, ' ');
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next().map(str::trim).unwrap_or("");
        match cmd {
            "/exit" | "/quit" => self.do_quit(),
            "/runs" | "/sessions" => self.open_overlay(),
            "/help" => {
                self.chat.push(Cell::Notice {
                    text: help_text(&self.commands),
                });
                vec![]
            }
            "/clear" if !self.working() => {
                self.thread_id = uuid::Uuid::new_v4().to_string();
                self.chat.push(Cell::Notice {
                    text: format!("new conversation · thread {}", short(&self.thread_id)),
                });
                vec![]
            }
            "/mode" if arg.is_empty() => vec![Effect::CycleMode],
            _ if self.working() => {
                self.set_status("wait for the turn to finish (or esc) before running commands");
                vec![]
            }
            _ => vec![Effect::Slash(line.to_string())],
        }
    }

    fn on_search_key(&mut self, k: KeyEvent) -> Vec<Effect> {
        let Some(q) = self.editor.search.as_mut() else {
            return vec![];
        };
        match k.code {
            KeyCode::Esc => self.editor.search = None,
            KeyCode::Enter | KeyCode::Tab | KeyCode::Right => self.editor.accept_search(),
            KeyCode::Backspace => {
                q.pop();
            }
            KeyCode::Char(c) => q.push(c),
            _ => {}
        }
        vec![]
    }

    // -------------------------------------------------------------- prompts

    fn on_prompt_key(&mut self, k: KeyEvent) -> Vec<Effect> {
        let is_text = self.prompts.front().is_some_and(|p| p.is_text());
        if is_text || self.prompt_text.is_some() {
            let editor = self
                .prompt_text
                .get_or_insert_with(|| Editor::new(Vec::new()));
            let shift = k.modifiers.contains(KeyModifiers::SHIFT)
                || k.modifiers.contains(KeyModifiers::ALT);
            match k.code {
                KeyCode::Esc => {
                    self.prompt_text = None;
                    // Back to the choices, on "Yes".
                    self.prompt_sel = 0;
                    if is_text {
                        // Answer later: the run keeps waiting.
                        if let Some(p) = self.prompts.pop_front() {
                            self.prompts.push_back(p);
                        }
                        self.set_status("answer it later · ctrl+t shows what is waiting");
                    }
                }
                KeyCode::Enter if shift => editor.newline(),
                KeyCode::Char('j') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    editor.newline()
                }
                KeyCode::Enter => {
                    let text = editor.expanded_text();
                    if is_text && text.trim().is_empty() {
                        return vec![];
                    }
                    self.prompt_text = None;
                    return if is_text {
                        self.answer_text(text)
                    } else {
                        self.answer_prompt_deny(Some(text).filter(|t| !t.trim().is_empty()))
                    };
                }
                KeyCode::Backspace => editor.backspace(),
                KeyCode::Delete => editor.delete(),
                KeyCode::Left => editor.left(),
                KeyCode::Right => editor.right(),
                KeyCode::Up => editor.up(),
                KeyCode::Down => editor.down(),
                KeyCode::Home => editor.home(),
                KeyCode::End => editor.end(),
                KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    editor.kill_to_start()
                }
                KeyCode::Char('w') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    editor.delete_word()
                }
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                    editor.insert_char(c)
                }
                _ => {}
            }
            return vec![];
        }
        let n = self.prompts.front().map(|p| p.options().len()).unwrap_or(0);
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.prompt_sel = self.prompt_sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                if self.prompt_sel + 1 < n {
                    self.prompt_sel += 1;
                }
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let i = c.to_digit(10).unwrap_or(0) as usize;
                if i >= 1 && i <= n {
                    self.prompt_sel = i - 1;
                    return self.choose(i - 1);
                }
            }
            KeyCode::Char('y') => return self.choose(0),
            KeyCode::Char('n') => return self.choose(n.saturating_sub(1)),
            KeyCode::Enter => return self.choose(self.prompt_sel),
            KeyCode::Esc => return self.choose(n.saturating_sub(1)),
            _ => {}
        }
        vec![]
    }

    /// Pick option `i` of the select prompt in front.
    fn choose(&mut self, i: usize) -> Vec<Effect> {
        let Some(front) = self.prompts.front() else {
            return vec![];
        };
        let n = front.options().len();
        if i + 1 == n {
            // "No …": ask what to tell the agent, Enter with nothing sends a
            // plain no.
            self.prompt_text = Some(Editor::new(Vec::new()));
            return vec![];
        }
        let remember = i == 1;
        self.reply_front(Reply::Allow { remember });
        vec![]
    }

    fn reply_front(&mut self, reply: Reply) {
        self.prompt_sel = 0;
        if let Some(Prompt::Broker {
            reply: Some(tx),
            interaction,
        }) = self.prompts.pop_front()
        {
            let what = match &reply {
                Reply::Allow { remember: true } => "allowed · remembered",
                Reply::Allow { .. } => "allowed",
                _ => "denied",
            };
            self.set_status(format!("{} {what}", interaction.tool_name()));
            let _ = tx.send(reply);
        }
    }

    fn answer_prompt_deny(&mut self, reason: Option<String>) -> Vec<Effect> {
        match self.prompts.front() {
            Some(Prompt::Broker { .. }) => {
                self.reply_front(Reply::Deny { reason });
                vec![]
            }
            Some(Prompt::Answer { .. }) => {
                if let Some(Prompt::Answer { .. }) = self.prompts.front() {
                    // Answer prompts have no "no"; keep it for later.
                    if let Some(p) = self.prompts.pop_front() {
                        self.prompts.push_back(p);
                    }
                }
                vec![]
            }
            None => vec![],
        }
    }

    fn answer_text(&mut self, text: String) -> Vec<Effect> {
        match self.prompts.pop_front() {
            Some(Prompt::Answer { agent_id, call, .. }) => {
                self.set_status(format!("sending your answer to {}…", call.tool_name));
                vec![Effect::SendText {
                    agent_id,
                    call,
                    text,
                }]
            }
            Some(Prompt::Broker {
                reply: Some(tx), ..
            }) => {
                let _ = tx.send(Reply::Text(text));
                vec![]
            }
            _ => vec![],
        }
    }

    // -------------------------------------------------------------- overlay

    fn open_overlay(&mut self) -> Vec<Effect> {
        if self.runs_supported == Some(false) {
            self.set_status("this server has no run list (GET /v1/runs)");
            return vec![];
        }
        let mut o = RunsOverlay::new();
        if let Some(r) = &self.runs {
            o.set_runs(r);
        }
        self.overlay = Some(o);
        vec![Effect::LoadRuns(self.runs_query())]
    }

    fn on_overlay_key(&mut self, k: KeyEvent) -> Vec<Effect> {
        let Some(o) = self.overlay.as_mut() else {
            return vec![];
        };
        if let Some(task) = o.confirm_cancel.take() {
            if k.code == KeyCode::Char('y') {
                let agent = o.agent_of(&task).unwrap_or_else(|| self.agent.clone());
                self.set_status(format!("canceling {}…", short(&task)));
                return vec![Effect::Cancel {
                    agent_id: agent,
                    task_id: task,
                }];
            }
            self.set_status("cancel aborted");
            return vec![];
        }
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => self.overlay = None,
            KeyCode::Char('t') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.overlay = None
            }
            KeyCode::Up | KeyCode::Char('k') => o.up(),
            KeyCode::Down | KeyCode::Char('j') => o.down(),
            KeyCode::Char('s') => o.show_stale = !o.show_stale,
            KeyCode::Char('t') => {
                o.all = !o.all;
                return vec![Effect::LoadRuns(self.runs_query())];
            }
            KeyCode::Char('r') => return vec![Effect::LoadRuns(self.runs_query())],
            KeyCode::Char('x') => {
                if let Some(run) = o.selected_run().filter(|r| !r.state.is_terminal()) {
                    let id = run.task_id.clone();
                    self.set_status(format!("cancel {} and its children? y/n", short(&id)));
                    if let Some(o) = self.overlay.as_mut() {
                        o.confirm_cancel = Some(id);
                    }
                } else {
                    self.set_status("that run has already finished");
                }
            }
            KeyCode::Enter | KeyCode::Char('a') => {
                let Some(run) = o.selected_run().cloned() else {
                    return vec![];
                };
                self.overlay = None;
                return self.focus_run(
                    &run.task_id,
                    run.agent_id.as_deref(),
                    run.title.clone().or(run.intent.clone()),
                );
            }
            _ => {}
        }
        vec![]
    }

    pub fn focus_run(
        &mut self,
        task_id: &str,
        agent: Option<&str>,
        title: Option<String>,
    ) -> Vec<Effect> {
        if self.chat.root_task.as_deref() == Some(task_id) && self.turn.is_some() {
            return self.focus_chat();
        }
        let agent = agent
            .map(str::to_string)
            .unwrap_or_else(|| self.agent.clone());
        let mut effects = self.follow(task_id, &agent, false);
        if let Some(f) = self.followed.get_mut(task_id) {
            if f.title.is_none() {
                f.title = title.clone();
            }
            let label = f.title.clone().unwrap_or_else(|| agent.clone());
            // The chat's own pending lines go out first.
            let opts = self.opts();
            let lines = self.chat.drain_commits(&opts);
            self.history.extend(lines);
            self.history.extend(super::cells::render_cell(
                &Cell::Header {
                    text: format!("following {label} · {agent} · esc returns to the chat"),
                },
                &opts,
            ));
            if let Some(f) = self.followed.get_mut(task_id) {
                // Replay what we already have from the start.
                f.transcript.rewind();
            }
        }
        self.focus = Some(task_id.to_string());
        effects.push(Effect::LoadRuns(self.runs_query()));
        effects
    }

    fn focus_chat(&mut self) -> Vec<Effect> {
        if let Some(task) = self.focus.take() {
            let opts = self.opts();
            if let Some(f) = self.followed.get_mut(&task) {
                let lines = f.transcript.drain_commits(&opts);
                self.history.extend(lines);
                if !f.auto && f.transcript.finished.is_some() {
                    // Nothing more to watch.
                }
            }
            self.chat.push(Cell::Header {
                text: "back to the chat".into(),
            });
        }
        vec![]
    }

    // ---------------------------------------------------------- completion

    fn refresh_popup(&mut self) {
        let text = self.editor.text();
        if self.editor.is_single_line() && text.starts_with('/') && !text.contains(' ') {
            let items: Vec<(String, String)> = self
                .commands
                .iter()
                .filter(|(c, _)| c.starts_with(&text) || (text.len() > 1 && c.contains(&text[1..])))
                .take(8)
                .cloned()
                .collect();
            let selected = self.popup.as_ref().map(|p| p.selected).unwrap_or(0);
            self.popup = (!items.is_empty()).then(|| Popup {
                selected: selected.min(items.len() - 1),
                items,
                kind: PopupKind::Command,
            });
            return;
        }
        let (_, token) = self.editor.current_token();
        if let Some(q) = token.strip_prefix('@') {
            let q = q.to_ascii_lowercase();
            let items: Vec<(String, String)> = self
                .files
                .iter()
                .filter(|f| f.to_ascii_lowercase().contains(&q))
                .take(8)
                .map(|f| (format!("@{f}"), String::new()))
                .collect();
            self.popup = (!items.is_empty()).then_some(Popup {
                items,
                selected: 0,
                kind: PopupKind::File,
            });
            return;
        }
        self.popup = None;
    }

    fn accept_popup(&mut self) {
        let Some(p) = self.popup.take() else { return };
        let Some((item, _)) = p.items.get(p.selected) else {
            return;
        };
        match p.kind {
            PopupKind::Command => self.editor.set_text(&format!("{item} ")),
            PopupKind::File => self.editor.replace_token(&format!("{item} ")),
        }
    }
}

fn agent_or(a: &str, b: &str) -> String {
    if a.is_empty() {
        b.to_string()
    } else {
        a.to_string()
    }
}

pub fn transcript_for(
    root: Option<String>,
    can_run: &ToolPredicate,
    is_script: &ToolPredicate,
) -> Transcript {
    let can = can_run.clone();
    let script = is_script.clone();
    Transcript::new(root, Box::new(move |name: &str| (can(name), script(name))))
}

pub fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn help_text(commands: &[(String, String)]) -> String {
    let mut s = String::from("Commands\n");
    for (c, d) in commands {
        s.push_str(&format!("  {c:<18} {d}\n"));
    }
    s.push_str(
        "\nKeys\n  enter send (queues while the agent works) · shift+enter or \\ newline\n  esc interrupt · esc esc edit last message · ctrl+c clear / twice exit\n  ctrl+t runs · ctrl+o expand tool output · ctrl+r search history\n  shift+tab permission mode · @ file · / command",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::{AgentEventType, ToolResponse};
    use ratatui::crossterm::event::KeyEvent;

    fn app() -> App {
        App::new(AppInit {
            agent: "coder".into(),
            model: None,
            thread_id: "thread-1".into(),
            history: vec![],
            can_run: Arc::new(|n: &str| n == "Bash" || n == "lookup_order"),
            is_script: Arc::new(|n: &str| n == "lookup_order"),
            commands: crate::tui::command_list(),
            files: vec!["src/main.rs".into(), "README.md".into()],
            mode_label: "default".into(),
            width: 80,
            height: 30,
        })
    }

    fn key(code: KeyCode) -> Input {
        Input::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl(c: char) -> Input {
        Input::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    fn type_text(a: &mut App, s: &str) {
        for c in s.chars() {
            a.handle(key(KeyCode::Char(c)));
        }
    }

    fn item(
        task: &str,
        parent: Option<&str>,
        agent: &str,
        event: AgentEventType,
    ) -> Box<StreamItem> {
        Box::new(StreamItem {
            message: None,
            agent_event: Some(AgentEvent {
                timestamp: chrono::Utc::now(),
                thread_id: "thread-1".into(),
                run_id: "r".into(),
                task_id: task.into(),
                parent_task_id: parent.map(str::to_string),
                agent_id: agent.into(),
                child: None,
                user_id: None,
                identifier_id: None,
                workspace_id: None,
                channel_id: None,
                event,
            }),
        })
    }

    fn call(id: &str, name: &str, input: serde_json::Value) -> ToolCall {
        ToolCall {
            tool_call_id: id.into(),
            tool_name: name.into(),
            input,
        }
    }

    fn send(a: &mut App, text: &str) -> u64 {
        type_text(a, text);
        let fx = a.handle(key(KeyCode::Enter));
        match fx.as_slice() {
            [Effect::SendTurn { turn, text: t }] => {
                assert_eq!(t, text);
                *turn
            }
            other => panic!("expected SendTurn, got {other:?}"),
        }
    }

    #[test]
    fn typing_while_working_queues_and_sends_after_the_turn() {
        let mut a = app();
        let turn = send(&mut a, "first");
        type_text(&mut a, "second");
        assert!(a.handle(key(KeyCode::Enter)).is_empty());
        assert_eq!(a.queue.len(), 1);
        let fx = a.handle(Input::TurnEnded { turn, error: None });
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::SendTurn { text, .. } if text == "second")));
        assert!(a.queue.is_empty());
    }

    #[test]
    fn esc_interrupts_the_turn_root_and_denies_open_questions() {
        let mut a = app();
        let turn = send(&mut a, "go");
        a.handle(Input::Turn {
            turn,
            item: item("root", None, "coder", AgentEventType::RunStarted {}),
        });
        a.turn.as_mut().unwrap().agent_id = Some("coder".into());
        let (tx, mut rx) = oneshot::channel();
        a.handle(Input::Ask(Ask {
            interaction: Interaction::Permit {
                tool: "Bash".into(),
                summary: "rm -rf x".into(),
                rule_hint: None,
            },
            reply: tx,
        }));
        // Esc on a select prompt picks "No" and opens the reason box…
        a.handle(key(KeyCode::Esc));
        assert!(a.prompt_text.is_some());
        // …ctrl+c there denies outright.
        a.handle(ctrl('c'));
        assert!(matches!(rx.try_recv(), Ok(Reply::Deny { .. })));
        let fx = a.handle(key(KeyCode::Esc));
        assert!(
            matches!(fx.as_slice(), [Effect::Cancel { agent_id, task_id }] if agent_id == "coder" && task_id == "root"),
            "{fx:?}"
        );
        a.handle(Input::TurnEnded { turn, error: None });
        assert!(a.turn.is_none());
    }

    #[test]
    fn permit_prompt_options_reply_typed() {
        let mut a = app();
        let (tx, mut rx) = oneshot::channel();
        a.handle(Input::Ask(Ask {
            interaction: Interaction::Permit {
                tool: "Bash".into(),
                summary: "cargo test".into(),
                rule_hint: Some("Bash(cargo test:*)".into()),
            },
            reply: tx,
        }));
        assert_eq!(a.prompts.front().unwrap().options().len(), 3);
        a.handle(key(KeyCode::Down));
        a.handle(key(KeyCode::Enter));
        assert_eq!(rx.try_recv().unwrap(), Reply::Allow { remember: true });
        assert!(a.prompts.is_empty());

        let (tx, mut rx) = oneshot::channel();
        a.handle(Input::Ask(Ask {
            interaction: Interaction::Grant {
                tool: "lookup_order".into(),
                script: "x.ts".into(),
                request: "net: db".into(),
            },
            reply: tx,
        }));
        a.handle(key(KeyCode::Char('3')));
        type_text(&mut a, "use the cache");
        a.handle(key(KeyCode::Enter));
        assert_eq!(
            rx.try_recv().unwrap(),
            Reply::Deny {
                reason: Some("use the cache".into())
            }
        );
    }

    #[test]
    fn a_question_no_local_tool_answers_becomes_a_text_prompt() {
        let mut a = app();
        let turn = send(&mut a, "ask the owner");
        a.handle(Input::Turn {
            turn,
            item: item("root", None, "asker", AgentEventType::RunStarted {}),
        });
        a.handle(Input::Turn {
            turn,
            item: item(
                "root",
                None,
                "asker",
                AgentEventType::ToolCalls {
                    step_id: "s".into(),
                    parent_message_id: None,
                    tool_calls: vec![call(
                        "q1",
                        "ask_human",
                        serde_json::json!({"question": "Ship Friday?"}),
                    )],
                },
            ),
        });
        a.runs_supported = Some(false);
        if let Some(p) = a.chat.pending.get_mut("q1") {
            p.seen -= ASK_AFTER * 2;
        }
        a.handle(Input::Tick);
        assert!(
            matches!(a.prompts.front(), Some(Prompt::Answer { question, .. }) if question == "Ship Friday?")
        );
        a.handle(Input::Paste("no, hold\nuntil Monday".into()));
        let fx = a.handle(key(KeyCode::Enter));
        assert!(
            matches!(fx.as_slice(), [Effect::SendText { text, agent_id, .. }] if text == "no, hold\nuntil Monday" && agent_id == "asker"),
            "a pasted newline does not send early: {fx:?}"
        );
    }

    #[test]
    fn background_children_are_followed_and_their_cli_tools_run_here() {
        let mut a = app();
        let turn = send(&mut a, "fork two");
        a.handle(Input::Turn {
            turn,
            item: item("root", None, "parent", AgentEventType::RunStarted {}),
        });
        a.handle(Input::Turn {
            turn,
            item: item("root", None, "parent", AgentEventType::ToolCalls {
                step_id: "s".into(),
                parent_message_id: None,
                tool_calls: vec![call("inv", "invoke_agent", serde_json::json!({"agent": "worker", "mode": "background", "prompt": "build it"}))],
            }),
        });
        a.handle(Input::Turn {
            turn,
            item: item(
                "root",
                None,
                "parent",
                AgentEventType::ToolResults {
                    step_id: "s".into(),
                    parent_message_id: None,
                    results: vec![ToolResponse::direct(
                        "inv".into(),
                        "invoke_agent".into(),
                        serde_json::json!({"kind": "task_ids", "task_ids": ["kid"]}),
                    )],
                },
            ),
        });
        let fx = a.handle(Input::TurnEnded { turn, error: None });
        assert!(fx.iter().any(|e| matches!(e, Effect::Follow { agent_id, task_id } if agent_id == "worker" && task_id == "kid")), "{fx:?}");
        // The child asks for a CLI tool after its parent's stream closed.
        a.handle(Input::Follow {
            task_id: "kid".into(),
            item: item(
                "kid",
                Some("root"),
                "worker",
                AgentEventType::ToolCalls {
                    step_id: "s".into(),
                    parent_message_id: None,
                    tool_calls: vec![call(
                        "b1",
                        "Bash",
                        serde_json::json!({"command": "cargo build"}),
                    )],
                },
            ),
        });
        // The server emits an execution start for external calls too.
        a.handle(Input::Follow {
            task_id: "kid".into(),
            item: item(
                "kid",
                Some("root"),
                "worker",
                AgentEventType::ToolExecutionStart {
                    step_id: "s".into(),
                    tool_call_id: "b1".into(),
                    tool_call_name: "Bash".into(),
                    input: serde_json::json!({"command": "cargo build"}),
                },
            ),
        });
        if let Some(p) = a
            .followed
            .get_mut("kid")
            .unwrap()
            .transcript
            .pending
            .get_mut("b1")
        {
            p.seen -= RUN_AFTER * 2;
        }
        let fx = a.handle(Input::Tick);
        assert!(fx.iter().any(|e| matches!(e, Effect::RunTool { agent_id, call, .. } if agent_id == "worker" && call.tool_name == "Bash")), "{fx:?}");
        // Only once.
        assert!(!a
            .handle(Input::Tick)
            .iter()
            .any(|e| matches!(e, Effect::RunTool { .. })));
        // It finishes: the chat says so with its answer.
        a.handle(Input::Follow {
            task_id: "kid".into(),
            item: item(
                "kid",
                Some("root"),
                "worker",
                AgentEventType::TextMessageStart {
                    message_id: "m".into(),
                    step_id: "s".into(),
                    role: distri_types::MessageRole::Assistant,
                },
            ),
        });
        a.handle(Input::Follow {
            task_id: "kid".into(),
            item: item(
                "kid",
                Some("root"),
                "worker",
                AgentEventType::TextMessageContent {
                    message_id: "m".into(),
                    step_id: "s".into(),
                    delta: "Built OK".into(),
                    stripped_content: None,
                },
            ),
        });
        a.handle(Input::Follow {
            task_id: "kid".into(),
            item: item(
                "kid",
                Some("root"),
                "worker",
                AgentEventType::RunFinished {
                    success: true,
                    total_steps: 1,
                    failed_steps: 0,
                    usage: None,
                    context_budget: None,
                },
            ),
        });
        let notice = a.chat.cells.iter().rev().find_map(|c| match c {
            Cell::Notice { text } => Some(text.clone()),
            _ => None,
        });
        assert_eq!(
            notice.as_deref(),
            Some("✓ background agent worker finished:\nBuilt OK")
        );
    }

    #[test]
    fn slash_and_file_completion() {
        let mut a = app();
        type_text(&mut a, "/ru");
        assert!(a.popup.as_ref().is_some_and(|p| p.items[0].0 == "/runs"));
        let fx = a.handle(key(KeyCode::Enter));
        assert!(matches!(fx.as_slice(), [Effect::LoadRuns(_)]), "{fx:?}");
        assert!(a.overlay.is_some());
        a.handle(key(KeyCode::Esc));
        assert!(a.overlay.is_none());
        type_text(&mut a, "look at @mai");
        a.handle(key(KeyCode::Tab));
        assert_eq!(a.editor.text(), "look at @src/main.rs ");
    }

    #[test]
    fn ctrl_c_clears_then_exits() {
        let mut a = app();
        type_text(&mut a, "draft");
        a.handle(ctrl('c'));
        assert!(a.editor.is_empty());
        let fx = a.handle(ctrl('c'));
        assert!(matches!(fx.as_slice(), [Effect::Quit]));
    }

    #[test]
    fn a_child_call_the_live_turn_carries_is_not_run_twice() {
        let mut a = app();
        let turn = send(&mut a, "go");
        a.handle(Input::Turn {
            turn,
            item: item("root", None, "parent", AgentEventType::RunStarted {}),
        });
        a.handle(Input::Turn {
            turn,
            item: item("kid", Some("root"), "worker", AgentEventType::RunStarted {}),
        });
        // The person follows the child from the overlay while the turn runs.
        a.focus_run("kid", Some("worker"), None);
        a.handle(Input::Follow {
            task_id: "kid".into(),
            item: item(
                "kid",
                Some("root"),
                "worker",
                AgentEventType::ToolCalls {
                    step_id: "s".into(),
                    parent_message_id: None,
                    tool_calls: vec![call("b2", "Bash", serde_json::json!({"command": "ls"}))],
                },
            ),
        });
        if let Some(p) = a
            .followed
            .get_mut("kid")
            .unwrap()
            .transcript
            .pending
            .get_mut("b2")
        {
            p.seen -= RUN_AFTER * 2;
        }
        assert!(!a
            .handle(Input::Tick)
            .iter()
            .any(|e| matches!(e, Effect::RunTool { .. })));
    }
}
