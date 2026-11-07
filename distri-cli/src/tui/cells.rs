//! The typed transcript: agent events reduce to [`Cell`]s, and each cell
//! renders itself to styled lines. A run's [`Transcript`] knows which cells
//! are finished, so the app can move them into the terminal's scrollback in
//! order (a streaming answer is committed paragraph by paragraph).

use std::collections::HashMap;
use std::time::Instant;

use ansi_to_tui::IntoText;
use distri::StreamItem;
use distri_types::{
    AgentEvent, AgentEventType, MessageRole, Part, RunState, ToolCall, ToolResponse,
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde::Deserialize;

use distri_cli::local_events::{LocalToolEventKind, LogLevel};

use super::markdown;
use super::wrap::{display_width, prefixed, wrap_lines};

/// Body lines a finished tool shows before folding.
pub const FOLD_LINES: usize = 3;
const DIM: Style = Style::new().fg(Color::DarkGray);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Ok,
    Err,
}

#[derive(Debug, Clone)]
pub struct SubRun {
    pub agent: String,
    pub title: Option<String>,
    pub state: RunState,
    pub tool_uses: u32,
    pub last: Option<String>,
    pub started: Instant,
    pub ended: Option<Instant>,
    /// Final answer text, when the child reported one.
    pub answer: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ToolCell {
    pub call: ToolCall,
    pub status: ToolStatus,
    /// The formatter's rendering of the result (ANSI), one entry per line.
    pub result: Vec<String>,
    pub logs: Vec<(LogLevel, String)>,
    pub progress: Option<(Option<f64>, Option<String>)>,
    /// A child run this call started (`invoke_agent`).
    pub sub: Option<SubRun>,
    /// Runs on this machine (a CLI tool).
    pub local: bool,
    /// Script tool (`.distri/tools/*.ts`).
    pub script: bool,
}

#[derive(Debug, Clone)]
pub enum Cell {
    User {
        text: String,
    },
    Assistant {
        md: String,
        streaming: bool,
        /// Bytes of `md` already committed to scrollback.
        committed: usize,
    },
    Tool(Box<ToolCell>),
    SubRun(SubRun),
    Todos {
        text: String,
    },
    Notice {
        text: String,
    },
    Error {
        text: String,
    },
    /// A header line: a run the view switched to.
    Header {
        text: String,
    },
}

impl Cell {
    pub fn is_final(&self) -> bool {
        match self {
            Cell::Assistant { streaming, .. } => !streaming,
            // A background child does not hold the transcript: its end is
            // reported separately when it finishes.
            Cell::Tool(t) => {
                t.status != ToolStatus::Running
                    && t.sub
                        .as_ref()
                        .is_none_or(|s| s.state.is_terminal() || is_background(&t.call))
            }
            Cell::SubRun(s) => s.state.is_terminal(),
            _ => true,
        }
    }
}

/// `invoke_agent` with `mode: "background"`: the parent does not wait.
pub fn is_background(call: &ToolCall) -> bool {
    call.input.get("mode").and_then(|m| m.as_str()) == Some("background")
}

pub struct RenderOpts {
    pub width: u16,
    pub expand: bool,
    pub frame: usize,
}

/// Animated dot for running things.
fn blink(frame: usize) -> &'static str {
    if (frame / 5).is_multiple_of(2) {
        "⏺"
    } else {
        " "
    }
}

pub fn render_cell(cell: &Cell, o: &RenderOpts) -> Vec<Line<'static>> {
    let w = o.width;
    match cell {
        Cell::User { text } => {
            let lines: Vec<Line> = text.lines().map(|l| Line::raw(l.to_string())).collect();
            let mut out = prefixed(
                &lines,
                w,
                Span::styled(
                    "> ",
                    Style::new()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
            );
            for l in &mut out {
                l.style = Style::new().fg(Color::Gray);
            }
            out.push(Line::default());
            out
        }
        Cell::Assistant { md, committed, .. } => {
            render_assistant(&md[*committed..], *committed == 0, w)
        }
        Cell::Tool(t) => render_tool(t, o),
        Cell::SubRun(s) => render_subrun_cell(s, o),
        Cell::Todos { text } => {
            let lines: Vec<Line> = text
                .lines()
                .map(|l| Line::styled(l.to_string(), DIM))
                .collect();
            prefixed(&lines, w, Span::styled("  ⎿  ", DIM), Span::raw("     "))
        }
        Cell::Notice { text } => {
            let lines: Vec<Line> = text
                .lines()
                .map(|l| Line::styled(l.to_string(), DIM))
                .collect();
            prefixed(&lines, w, Span::styled("· ", DIM), Span::raw("  "))
        }
        Cell::Error { text } => {
            let red = Style::new().fg(Color::LightRed);
            let lines: Vec<Line> = text
                .lines()
                .map(|l| Line::styled(l.to_string(), red))
                .collect();
            prefixed(&lines, w, Span::styled("✗ ", red), Span::raw("  "))
        }
        Cell::Header { text } => vec![
            Line::default(),
            Line::styled(
                format!(
                    "── {text} {}",
                    "─".repeat((w as usize).saturating_sub(display_width(text) + 4))
                ),
                Style::new().fg(Color::Cyan),
            ),
        ],
    }
}

fn render_assistant(md: &str, first: bool, w: u16) -> Vec<Line<'static>> {
    let body = markdown::render(md);
    if body.is_empty() {
        return Vec::new();
    }
    let lead = if first {
        Span::styled("⏺ ", Style::new().fg(Color::White))
    } else {
        Span::raw("  ")
    };
    let mut out = prefixed(&body, w, lead, Span::raw("  "));
    out.push(Line::default());
    out
}

fn tool_header(t: &ToolCell, o: &RenderOpts) -> Line<'static> {
    let (dot, color) = match t.status {
        ToolStatus::Running => (blink(o.frame), Color::Yellow),
        ToolStatus::Ok => ("⏺", Color::Green),
        ToolStatus::Err => ("⏺", Color::LightRed),
    };
    let summary = distri_formatter::state::format_tool_call(&t.call.tool_name, &t.call.input);
    let (name, args) = match summary.find('(') {
        Some(i) => (summary[..i].to_string(), summary[i..].to_string()),
        None => (summary.clone(), String::new()),
    };
    let mut spans = vec![
        Span::styled(format!("{dot} "), Style::new().fg(color)),
        Span::styled(name, Style::new().add_modifier(Modifier::BOLD)),
        Span::styled(args, DIM),
    ];
    if t.script {
        spans.push(Span::styled(" · script", Style::new().fg(Color::Magenta)));
    } else if t.local {
        spans.push(Span::styled(" · local", DIM));
    }
    Line::from(spans)
}

fn render_tool(t: &ToolCell, o: &RenderOpts) -> Vec<Line<'static>> {
    let w = o.width;
    let mut out = wrap_lines(&[tool_header(t, o)], w);
    if let Some(sub) = &t.sub {
        out.extend(render_subrun_body(sub, o));
        return out;
    }
    if t.status == ToolStatus::Running {
        if let Some((frac, msg)) = &t.progress {
            let mut spans = vec![Span::styled("  ⎿  ", DIM)];
            if let Some(f) = frac {
                let f = f.clamp(0.0, 1.0);
                let filled = (f * 20.0).round() as usize;
                spans.push(Span::styled(
                    "█".repeat(filled),
                    Style::new().fg(Color::Cyan),
                ));
                spans.push(Span::styled("░".repeat(20 - filled), DIM));
                spans.push(Span::styled(format!(" {:>3.0}%", f * 100.0), DIM));
            }
            if let Some(m) = msg {
                spans.push(Span::styled(format!("  {m}"), DIM));
            }
            out.extend(wrap_lines(&[Line::from(spans)], w));
        } else if t.logs.is_empty() {
            out.push(Line::styled("  ⎿  running…", DIM));
        }
        let tail = t.logs.len().saturating_sub(4);
        for (level, line) in &t.logs[tail..] {
            out.extend(log_line(*level, line, w));
        }
        return out;
    }
    let body: Vec<Line<'static>> = t
        .result
        .iter()
        .flat_map(|l| {
            l.as_bytes()
                .into_text()
                .map(|t| t.lines)
                .unwrap_or_default()
        })
        .collect();
    let mut body = wrap_lines(&body, w);
    if body.is_empty() && !t.logs.is_empty() {
        for (level, line) in t.logs.iter().take(FOLD_LINES) {
            body.extend(log_line(*level, line, w));
        }
    }
    if !o.expand && body.len() > FOLD_LINES + 1 {
        let hidden = body.len() - FOLD_LINES;
        body.truncate(FOLD_LINES);
        body.push(Line::styled(
            format!("     … +{hidden} lines (ctrl+o to expand)"),
            DIM,
        ));
    }
    out.extend(body);
    out
}

fn log_line(level: LogLevel, line: &str, w: u16) -> Vec<Line<'static>> {
    let (tag, style) = match level {
        LogLevel::Log => ("log  ", DIM),
        LogLevel::Warn => ("warn ", Style::new().fg(Color::Yellow)),
    };
    prefixed(
        &[Line::styled(line.to_string(), style)],
        w,
        Span::styled(format!("     {tag}"), style),
        Span::raw("          "),
    )
}

fn state_badge(state: RunState) -> (&'static str, Color) {
    match state {
        RunState::NeedsInput => ("● needs you", Color::Yellow),
        RunState::Working => ("◐ working", Color::Cyan),
        RunState::Done => ("✓ done", Color::Green),
        RunState::Failed => ("✗ failed", Color::LightRed),
        RunState::Canceled => ("⊘ canceled", Color::DarkGray),
    }
}

fn elapsed(s: &SubRun) -> String {
    let d = s
        .ended
        .unwrap_or_else(Instant::now)
        .duration_since(s.started);
    fmt_secs(d.as_secs())
}

pub fn fmt_secs(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn render_subrun_body(s: &SubRun, o: &RenderOpts) -> Vec<Line<'static>> {
    let (badge, color) = state_badge(s.state);
    let uses = format!(
        " · {} tool use{} · {}",
        s.tool_uses,
        if s.tool_uses == 1 { "" } else { "s" },
        elapsed(s)
    );
    let mut spans = vec![
        Span::styled("  ⎿  ", DIM),
        Span::styled(badge, Style::new().fg(color).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {}", s.agent), Style::new().fg(Color::Gray)),
        Span::styled(uses, DIM),
    ];
    if !s.state.is_terminal() {
        spans.push(Span::styled(" · ctrl+t to follow", DIM));
    }
    let mut out = wrap_lines(&[Line::from(spans)], o.width);
    let detail = if s.state.is_terminal() {
        s.answer.as_deref().or(s.last.as_deref())
    } else {
        s.last.as_deref()
    };
    if let Some(last) = detail {
        let one: String = last
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(200)
            .collect();
        out.extend(prefixed(
            &[Line::styled(one, DIM)],
            o.width,
            Span::raw("     "),
            Span::raw("     "),
        ));
    }
    out
}

fn render_subrun_cell(s: &SubRun, o: &RenderOpts) -> Vec<Line<'static>> {
    let title = s.title.clone().unwrap_or_else(|| s.agent.clone());
    let dot = if s.state.is_terminal() {
        "⏺"
    } else {
        blink(o.frame)
    };
    let mut out = wrap_lines(
        &[Line::from(vec![
            Span::styled(format!("{dot} "), Style::new().fg(Color::Cyan)),
            Span::styled("agent", Style::new().add_modifier(Modifier::BOLD)),
            Span::styled(format!("({title})"), DIM),
        ])],
        o.width,
    );
    out.extend(render_subrun_body(s, o));
    out
}

/// A call waiting on something that is not a server tool.
#[derive(Debug, Clone)]
pub struct PendingCall {
    pub call: ToolCall,
    pub event: AgentEvent,
    pub task_id: String,
    pub agent_id: String,
    pub seen: Instant,
    pub exec_started: bool,
    /// Already handed to the registry or a prompt.
    pub dispatched: bool,
}

/// `invoke_agent`'s result for background children.
#[derive(Debug, Deserialize)]
struct TaskIdsResult {
    kind: String,
    #[serde(default)]
    task_ids: Vec<String>,
}

/// `(runs locally, is a script tool)` for a tool name.
pub type LocalToolFn = Box<dyn Fn(&str) -> (bool, bool) + Send + Sync>;

/// One run's transcript.
pub struct Transcript {
    pub cells: Vec<Cell>,
    /// Cells before this index are in scrollback.
    committed: usize,
    pub root_task: Option<String>,
    msg_index: HashMap<String, usize>,
    tool_index: HashMap<String, usize>,
    /// child task id → cell index (a Tool with `sub`, or a SubRun cell).
    sub_index: HashMap<String, usize>,
    /// Calls not answered yet, by call id.
    pub pending: HashMap<String, PendingCall>,
    pub finished: Option<RunState>,
    /// What the run answered through its `final` tool.
    pub final_answer: Option<String>,
    pub usage_in: u64,
    pub usage_out: u64,
    /// Children started in the background that this run knows of.
    pub children: Vec<(String, String)>,
    pub last_activity: Instant,
    pub local_tool: LocalToolFn,
}

impl Transcript {
    /// `local_tool(name) -> (runs locally, is a script tool)`.
    pub fn new(root_task: Option<String>, local_tool: LocalToolFn) -> Self {
        Self {
            cells: Vec::new(),
            committed: 0,
            root_task,
            msg_index: HashMap::new(),
            tool_index: HashMap::new(),
            sub_index: HashMap::new(),
            pending: HashMap::new(),
            finished: None,
            final_answer: None,
            usage_in: 0,
            usage_out: 0,
            children: Vec::new(),
            last_activity: Instant::now(),
            local_tool,
        }
    }

    /// A new turn on the same thread: a new root task.
    pub fn begin_turn(&mut self) {
        self.root_task = None;
        self.finished = None;
        self.pending.clear();
    }

    /// Show everything again from the first cell (switching to this run).
    pub fn rewind(&mut self) {
        self.committed = 0;
        for c in &mut self.cells {
            if let Cell::Assistant { committed, .. } = c {
                *committed = 0;
            }
        }
    }

    pub fn push(&mut self, cell: Cell) {
        self.cells.push(cell);
    }

    /// Cells not yet in scrollback.
    pub fn live(&self) -> &[Cell] {
        &self.cells[self.committed..]
    }

    pub fn last_tool_index(&self) -> Option<usize> {
        self.cells.iter().rposition(|c| matches!(c, Cell::Tool(_)))
    }

    /// Lines to move into scrollback now: finished cells in order, and the
    /// finished paragraphs of a streaming answer.
    pub fn drain_commits(&mut self, o: &RenderOpts) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        while self.committed < self.cells.len() {
            let idx = self.committed;
            if self.cells[idx].is_final() {
                out.extend(render_cell(&self.cells[idx], o));
                self.committed += 1;
                continue;
            }
            if let Cell::Assistant { md, committed, .. } = &mut self.cells[idx] {
                let cut = markdown::committable_prefix(&md[*committed..]);
                if cut > 0 {
                    let piece = md[*committed..*committed + cut].to_string();
                    out.extend(render_assistant(&piece, *committed == 0, o.width));
                    *committed += cut;
                }
            }
            break;
        }
        out
    }

    pub fn apply(&mut self, item: &StreamItem) {
        self.last_activity = Instant::now();
        if let Some(event) = &item.agent_event {
            self.apply_event(event);
        }
        if let Some(msg) = &item.message {
            if msg.role == MessageRole::Assistant {
                if let Some(text) = msg.as_text().filter(|t| !t.trim().is_empty()) {
                    let task = msg_task(item);
                    let is_child = matches!((&task, &self.root_task), (Some(t), Some(r)) if t != r);
                    if is_child {
                        if let Some(sub) = task.and_then(|t| self.sub_mut(&t)) {
                            sub.answer = Some(text.clone());
                        }
                    } else if !self.cells.iter().rev().take(4).any(
                        |c| matches!(c, Cell::Assistant { md, .. } if md.trim() == text.trim()),
                    ) {
                        self.cells.push(Cell::Assistant {
                            md: text,
                            streaming: false,
                            committed: 0,
                        });
                    }
                }
            }
        }
    }

    fn is_root(&mut self, event: &AgentEvent) -> bool {
        match &self.root_task {
            Some(r) => &event.task_id == r,
            None => {
                if event.parent_task_id.is_none()
                    || matches!(event.event, AgentEventType::RunStarted {})
                {
                    self.root_task = Some(event.task_id.clone());
                    true
                } else {
                    false
                }
            }
        }
    }

    fn sub_mut(&mut self, task_id: &str) -> Option<&mut SubRun> {
        let idx = *self.sub_index.get(task_id)?;
        match self.cells.get_mut(idx)? {
            Cell::Tool(t) => t.sub.as_mut(),
            Cell::SubRun(s) => Some(s),
            _ => None,
        }
    }

    /// Find or make the sub-run for a child event.
    fn child(&mut self, event: &AgentEvent) -> &mut SubRun {
        if !self.sub_index.contains_key(&event.task_id) {
            let parent_call = event
                .child
                .as_ref()
                .and_then(|c| c.parent_tool_call_id.clone());
            let title = event.child.as_ref().and_then(|c| c.title.clone());
            let sub = SubRun {
                agent: event.agent_id.clone(),
                title,
                state: RunState::Working,
                tool_uses: 0,
                last: None,
                started: Instant::now(),
                ended: None,
                answer: None,
            };
            let idx = match parent_call.and_then(|id| self.tool_index.get(&id).copied()) {
                Some(i) if matches!(&self.cells[i], Cell::Tool(t) if t.sub.is_none()) => {
                    if let Cell::Tool(t) = &mut self.cells[i] {
                        t.sub = Some(sub);
                    }
                    i
                }
                _ => {
                    self.cells.push(Cell::SubRun(sub));
                    self.cells.len() - 1
                }
            };
            self.sub_index.insert(event.task_id.clone(), idx);
            if !self.children.iter().any(|(t, _)| t == &event.task_id) {
                self.children
                    .push((event.task_id.clone(), event.agent_id.clone()));
            }
        }
        self.sub_mut(&event.task_id.clone()).expect("just inserted")
    }

    fn apply_child_event(&mut self, event: &AgentEvent) {
        let pending_calls: Vec<ToolCall> = match &event.event {
            AgentEventType::ToolCalls { tool_calls, .. } => tool_calls.clone(),
            _ => Vec::new(),
        };
        {
            let sub = self.child(event);
            match &event.event {
                AgentEventType::ToolExecutionStart {
                    tool_call_name,
                    input,
                    ..
                } if !distri_formatter::state::is_probe_call(tool_call_name, input) => {
                    sub.tool_uses += 1;
                    sub.last = Some(distri_formatter::state::format_tool_call(
                        tool_call_name,
                        input,
                    ));
                }
                AgentEventType::TextMessageContent { delta, .. } => {
                    let mut last = sub
                        .last
                        .take()
                        .filter(|l| !l.starts_with('⏺'))
                        .unwrap_or_default();
                    last.push_str(delta);
                    let tail: String = last.lines().last().unwrap_or("").to_string();
                    sub.last = Some(tail);
                }
                AgentEventType::RunFinished { success, .. } => {
                    sub.state = if *success {
                        RunState::Done
                    } else {
                        RunState::Failed
                    };
                    sub.ended = Some(Instant::now());
                }
                AgentEventType::RunError { message, .. } => {
                    sub.state = if message.to_ascii_lowercase().contains("cancel") {
                        RunState::Canceled
                    } else {
                        RunState::Failed
                    };
                    sub.last = Some(message.clone());
                    sub.ended = Some(Instant::now());
                }
                _ => {}
            }
        }
        for call in pending_calls {
            if call.tool_name == "final" {
                if let Ok(v) = serde_json::from_value::<FinalInput>(call.input.clone()) {
                    if let Some(sub) = self.sub_mut(&event.task_id) {
                        sub.answer = Some(v.text());
                    }
                }
                continue;
            }
            if let Some(sub) = self.sub_mut(&event.task_id) {
                sub.last = Some(distri_formatter::state::format_tool_call(
                    &call.tool_name,
                    &call.input,
                ));
            }
            self.track_pending(&call, event);
        }
        if let AgentEventType::ToolExecutionStart { tool_call_id, .. } = &event.event {
            if let Some(p) = self.pending.get_mut(tool_call_id) {
                p.exec_started = true;
            }
        }
        self.clear_answered(event);
    }

    fn track_pending(&mut self, call: &ToolCall, event: &AgentEvent) {
        if distri_formatter::state::is_probe_call(&call.tool_name, &call.input) {
            return;
        }
        self.pending
            .entry(call.tool_call_id.clone())
            .or_insert_with(|| PendingCall {
                call: call.clone(),
                event: event.clone(),
                task_id: event.task_id.clone(),
                agent_id: event.agent_id.clone(),
                seen: Instant::now(),
                exec_started: false,
                dispatched: false,
            });
    }

    fn clear_answered(&mut self, event: &AgentEvent) {
        match &event.event {
            AgentEventType::ToolExecutionEnd { tool_call_id, .. } => {
                self.pending.remove(tool_call_id);
            }
            AgentEventType::ToolResults { results, .. } => {
                for r in results {
                    self.pending.remove(&r.tool_call_id);
                }
            }
            AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. } => {
                let task = event.task_id.clone();
                self.pending.retain(|_, p| p.task_id != task);
            }
            _ => {}
        }
    }

    fn apply_event(&mut self, event: &AgentEvent) {
        if !self.is_root(event) {
            self.apply_child_event(event);
            return;
        }
        match &event.event {
            AgentEventType::TextMessageStart {
                message_id, role, ..
            } => {
                if *role == MessageRole::Assistant && !self.msg_index.contains_key(message_id) {
                    self.cells.push(Cell::Assistant {
                        md: String::new(),
                        streaming: true,
                        committed: 0,
                    });
                    self.msg_index
                        .insert(message_id.clone(), self.cells.len() - 1);
                }
            }
            AgentEventType::TextMessageContent {
                message_id, delta, ..
            } => {
                let idx = match self.msg_index.get(message_id) {
                    Some(i) => *i,
                    None => {
                        self.cells.push(Cell::Assistant {
                            md: String::new(),
                            streaming: true,
                            committed: 0,
                        });
                        self.msg_index
                            .insert(message_id.clone(), self.cells.len() - 1);
                        self.cells.len() - 1
                    }
                };
                if let Cell::Assistant { md, .. } = &mut self.cells[idx] {
                    md.push_str(delta);
                }
            }
            AgentEventType::TextMessageEnd { message_id, .. } => {
                if let Some(idx) = self.msg_index.get(message_id).copied() {
                    if let Cell::Assistant {
                        streaming,
                        md,
                        committed,
                        ..
                    } = &mut self.cells[idx]
                    {
                        *streaming = false;
                        if !md.ends_with('\n') && *committed < md.len() {
                            md.push('\n');
                        }
                    }
                }
            }
            AgentEventType::ToolCalls { tool_calls, .. } => {
                for call in tool_calls {
                    if call.tool_name == "final" {
                        if let Ok(v) = serde_json::from_value::<FinalInput>(call.input.clone()) {
                            self.final_answer = Some(v.text());
                        }
                    }
                    self.ensure_tool(call);
                    self.track_pending(call, event);
                }
            }
            AgentEventType::ToolExecutionStart {
                tool_call_id,
                tool_call_name,
                input,
                ..
            } => {
                let call = ToolCall {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_call_name.clone(),
                    input: input.clone(),
                };
                self.ensure_tool(&call);
                if let Some(p) = self.pending.get_mut(tool_call_id) {
                    p.exec_started = true;
                }
            }
            AgentEventType::ToolExecutionEnd {
                tool_call_id,
                success,
                ..
            } => {
                if let Some(Cell::Tool(t)) = self
                    .tool_index
                    .get(tool_call_id)
                    .and_then(|i| self.cells.get_mut(*i))
                {
                    if t.status == ToolStatus::Running {
                        t.status = if *success {
                            ToolStatus::Ok
                        } else {
                            ToolStatus::Err
                        };
                    }
                }
            }
            AgentEventType::ToolResults { results, .. } => {
                for r in results {
                    self.apply_result(r);
                }
            }
            AgentEventType::TodosUpdated {
                formatted_todos, ..
            } => {
                if !formatted_todos.trim().is_empty() {
                    self.cells.push(Cell::Todos {
                        text: formatted_todos.clone(),
                    });
                }
            }
            AgentEventType::RunFinished { success, usage, .. } => {
                self.finished = Some(if *success {
                    RunState::Done
                } else {
                    RunState::Failed
                });
                if let Some(u) = usage {
                    self.usage_in += u.input_tokens as u64;
                    self.usage_out += u.output_tokens as u64;
                }
                self.close_running(ToolStatus::Ok);
            }
            AgentEventType::RunError { message, .. } => {
                let canceled = message.to_ascii_lowercase().contains("cancel");
                self.finished = Some(if canceled {
                    RunState::Canceled
                } else {
                    RunState::Failed
                });
                self.close_running(ToolStatus::Err);
                if canceled {
                    self.cells.push(Cell::Notice {
                        text: "Interrupted · the run was canceled".into(),
                    });
                } else {
                    self.cells.push(Cell::Error {
                        text: message.clone(),
                    });
                }
            }
            AgentEventType::ContextCompaction {
                tokens_before,
                tokens_after,
                entries_affected,
                ..
            } => {
                self.cells.push(Cell::Notice {
                    text: format!("compacted {entries_affected} entries · {tokens_before} → {tokens_after} tokens"),
                });
            }
            AgentEventType::AgentHandover {
                from_agent,
                to_agent,
                reason,
            } => {
                self.cells.push(Cell::Notice {
                    text: format!(
                        "handover {from_agent} → {to_agent}{}",
                        reason
                            .as_deref()
                            .map(|r| format!(": {r}"))
                            .unwrap_or_default()
                    ),
                });
            }
            AgentEventType::LiveView { url, title, .. } => {
                self.cells.push(Cell::Notice {
                    text: format!(
                        "live view: {}{url}",
                        title
                            .as_deref()
                            .map(|t| format!("{t} · "))
                            .unwrap_or_default()
                    ),
                });
            }
            _ => {}
        }
        self.clear_answered(event);
    }

    /// The run ended: nothing in it is still running. A child it was
    /// waiting on in the foreground ended with it; a background child keeps
    /// its own state.
    fn close_running(&mut self, status: ToolStatus) {
        let child_state = if status == ToolStatus::Ok {
            RunState::Done
        } else {
            RunState::Canceled
        };
        for c in &mut self.cells {
            match c {
                Cell::Tool(t) if t.status == ToolStatus::Running => {
                    t.status = status;
                    if let Some(s) = t.sub.as_mut().filter(|_| !is_background(&t.call)) {
                        if !s.state.is_terminal() {
                            s.state = child_state;
                            s.ended = Some(Instant::now());
                        }
                    }
                }
                Cell::SubRun(s) if !s.state.is_terminal() => {
                    s.state = child_state;
                    s.ended = Some(Instant::now());
                }
                Cell::Assistant { streaming, .. } => *streaming = false,
                _ => {}
            }
        }
    }

    /// The stream ended (finished, failed or dropped): close what is open so
    /// the transcript can move on.
    pub fn close_open(&mut self) {
        let status = match self.finished {
            Some(RunState::Done) => ToolStatus::Ok,
            _ => ToolStatus::Err,
        };
        self.close_running(status);
    }

    fn ensure_tool(&mut self, call: &ToolCall) {
        if self.tool_index.contains_key(&call.tool_call_id)
            || distri_formatter::state::is_probe_call(&call.tool_name, &call.input)
        {
            return;
        }
        let (local, script) = (self.local_tool)(&call.tool_name);
        self.cells.push(Cell::Tool(Box::new(ToolCell {
            call: call.clone(),
            status: ToolStatus::Running,
            result: Vec::new(),
            logs: Vec::new(),
            progress: None,
            sub: None,
            local,
            script,
        })));
        self.tool_index
            .insert(call.tool_call_id.clone(), self.cells.len() - 1);
    }

    fn apply_result(&mut self, r: &ToolResponse) {
        let Some(idx) = self.tool_index.get(&r.tool_call_id).copied() else {
            return;
        };
        // Background children: `{"kind":"task_ids","task_ids":[…]}`.
        let mut started: Vec<String> = Vec::new();
        for part in &r.parts {
            let data = match part {
                Part::Data(v) => Some(v),
                Part::ToolResult(inner) => inner.parts.iter().find_map(|p| match p {
                    Part::Data(v) => Some(v),
                    _ => None,
                }),
                _ => None,
            };
            if let Some(Ok(ids)) = data.map(|v| serde_json::from_value::<TaskIdsResult>(v.clone()))
            {
                if ids.kind == "task_ids" {
                    started.extend(ids.task_ids);
                }
            }
        }
        let is_error = r
            .parts
            .iter()
            .any(|p| matches!(p, Part::Data(v) if v.get("error").is_some()));
        let mut buf: Vec<u8> = Vec::new();
        distri_formatter::renderers::render_tool_output(&mut buf, r, false);
        let text = String::from_utf8_lossy(&buf).into_owned();
        let agent_hint = match &self.cells[idx] {
            Cell::Tool(t) => t
                .call
                .input
                .get("agent")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            _ => None,
        };
        if let Cell::Tool(t) = &mut self.cells[idx] {
            t.result = text
                .lines()
                .map(str::to_string)
                .filter(|l| !l.trim().is_empty())
                .collect();
            if t.status == ToolStatus::Running {
                t.status = if is_error {
                    ToolStatus::Err
                } else {
                    ToolStatus::Ok
                };
            }
            if t.sub.is_none() && !started.is_empty() {
                t.sub = Some(SubRun {
                    agent: agent_hint.clone().unwrap_or_else(|| "agent".into()),
                    title: t
                        .call
                        .input
                        .get("title")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    state: RunState::Working,
                    tool_uses: 0,
                    last: Some("started in the background".into()),
                    started: Instant::now(),
                    ended: None,
                    answer: None,
                });
                t.result.clear();
            }
        }
        for id in started {
            self.sub_index.entry(id.clone()).or_insert(idx);
            if !self.children.iter().any(|(t, _)| t == &id) {
                self.children
                    .push((id, agent_hint.clone().unwrap_or_default()));
            }
        }
    }

    /// A local tool reported progress or a log line.
    pub fn apply_local(&mut self, tool_call_id: &str, kind: &LocalToolEventKind) -> bool {
        let Some(idx) = self.tool_index.get(tool_call_id).copied() else {
            return false;
        };
        if let LocalToolEventKind::Finished(result) = kind {
            match result {
                Ok(resp) => {
                    // The server does not echo results of calls this client
                    // ran; show what was sent back, unless it already did.
                    let empty = matches!(&self.cells[idx], Cell::Tool(t) if t.result.is_empty());
                    if empty {
                        self.apply_result(resp);
                    }
                }
                Err(e) => {
                    if let Cell::Tool(t) = &mut self.cells[idx] {
                        t.status = ToolStatus::Err;
                        t.result = vec![format!("\x1b[31m  ⎿  {e}\x1b[0m")];
                    }
                }
            }
            return true;
        }
        if let Cell::Tool(t) = &mut self.cells[idx] {
            match kind {
                LocalToolEventKind::Finished(_) => {}
                LocalToolEventKind::Progress { fraction, message } => {
                    t.progress = Some((*fraction, message.clone()));
                }
                LocalToolEventKind::Log { level, line } => {
                    t.logs.push((*level, line.clone()));
                    if t.logs.len() > 200 {
                        t.logs.remove(0);
                    }
                }
            }
            return true;
        }
        false
    }

    /// Update a child's state from a run listing or its own stream.
    pub fn set_child_state(&mut self, task_id: &str, state: RunState, answer: Option<String>) {
        if let Some(sub) = self.sub_mut(task_id) {
            if sub.state != state {
                sub.state = state;
                if state.is_terminal() {
                    sub.ended = Some(Instant::now());
                }
            }
            if answer.is_some() {
                sub.answer = answer;
            }
        }
    }

    /// The title the parent gave a child (`invoke_agent`'s `title`).
    pub fn child_title(&self, task_id: &str) -> Option<String> {
        let idx = *self.sub_index.get(task_id)?;
        match self.cells.get(idx)? {
            Cell::Tool(t) => t.sub.as_ref().and_then(|s| s.title.clone()),
            Cell::SubRun(s) => s.title.clone(),
            _ => None,
        }
    }

    /// Children still running in the background.
    pub fn open_children(&self) -> Vec<(String, String)> {
        self.children
            .iter()
            .filter(|(id, _)| {
                self.sub_index
                    .get(id)
                    .and_then(|i| match self.cells.get(*i) {
                        Some(Cell::Tool(t)) => t.sub.as_ref().map(|s| !s.state.is_terminal()),
                        Some(Cell::SubRun(s)) => Some(!s.state.is_terminal()),
                        _ => None,
                    })
                    .unwrap_or(true)
            })
            .cloned()
            .collect()
    }

    /// Tool output to show in full (ctrl+o): the last finished tool.
    pub fn expanded_last_tool(&self, o: &RenderOpts) -> Option<Vec<Line<'static>>> {
        let idx = self.last_tool_index()?;
        let expanded = RenderOpts {
            width: o.width,
            expand: true,
            frame: o.frame,
        };
        Some(render_cell(&self.cells[idx], &expanded))
    }
}

/// The `final` tool's input: `{"input": …}` (the server's `FinalInput`), or
/// the bare value.
#[derive(Deserialize)]
#[serde(untagged)]
enum FinalInput {
    Obj {
        #[serde(alias = "result")]
        input: serde_json::Value,
    },
    Other(serde_json::Value),
}

impl FinalInput {
    fn text(self) -> String {
        match self {
            FinalInput::Obj {
                input: serde_json::Value::String(s),
            }
            | FinalInput::Other(serde_json::Value::String(s)) => s,
            FinalInput::Obj { input } | FinalInput::Other(input) => input.to_string(),
        }
    }
}

fn msg_task(item: &StreamItem) -> Option<String> {
    item.agent_event.as_ref().map(|e| e.task_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::ChildRun;

    fn ev(task: &str, parent: Option<&str>, agent: &str, event: AgentEventType) -> StreamItem {
        StreamItem {
            message: None,
            agent_event: Some(AgentEvent {
                timestamp: chrono::Utc::now(),
                thread_id: "th".into(),
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
        }
    }

    fn t() -> Transcript {
        Transcript::new(
            None,
            Box::new(|n: &str| (n == "Bash" || n == "lookup_order", n == "lookup_order")),
        )
    }

    fn opts() -> RenderOpts {
        RenderOpts {
            width: 60,
            expand: false,
            frame: 0,
        }
    }

    fn text(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn call(id: &str, name: &str, input: serde_json::Value) -> ToolCall {
        ToolCall {
            tool_call_id: id.into(),
            tool_name: name.into(),
            input,
        }
    }

    #[test]
    fn streaming_text_commits_paragraph_by_paragraph() {
        let mut tr = t();
        tr.apply(&ev("root", None, "coder", AgentEventType::RunStarted {}));
        tr.apply(&ev(
            "root",
            None,
            "coder",
            AgentEventType::TextMessageStart {
                message_id: "m".into(),
                step_id: "s".into(),
                role: MessageRole::Assistant,
            },
        ));
        tr.apply(&ev(
            "root",
            None,
            "coder",
            AgentEventType::TextMessageContent {
                message_id: "m".into(),
                step_id: "s".into(),
                delta: "First **para**.\n\nSecond".into(),
                stripped_content: None,
            },
        ));
        let c = text(&tr.drain_commits(&opts()));
        assert!(c.contains("⏺ First para."), "{c}");
        assert!(!c.contains("Second"));
        assert_eq!(tr.live().len(), 1);
        tr.apply(&ev(
            "root",
            None,
            "coder",
            AgentEventType::TextMessageEnd {
                message_id: "m".into(),
                step_id: "s".into(),
            },
        ));
        let c = text(&tr.drain_commits(&opts()));
        assert!(c.contains("  Second"), "continuation has no bullet: {c}");
        assert!(tr.live().is_empty());
    }

    #[test]
    fn tools_fold_and_wait_for_results_in_order() {
        let mut tr = t();
        tr.apply(&ev("root", None, "coder", AgentEventType::RunStarted {}));
        tr.apply(&ev(
            "root",
            None,
            "coder",
            AgentEventType::ToolCalls {
                step_id: "s".into(),
                parent_message_id: None,
                tool_calls: vec![call("c1", "Bash", serde_json::json!({"command": "seq 10"}))],
            },
        ));
        assert!(
            tr.drain_commits(&opts()).is_empty(),
            "running tool stays live"
        );
        assert!(tr.pending.contains_key("c1"));
        let out = (1..=10)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let resp = ToolResponse::direct(
            "c1".into(),
            "Bash".into(),
            serde_json::json!({"stdout": out, "exit_code": 0}),
        );
        tr.apply(&ev(
            "root",
            None,
            "coder",
            AgentEventType::ToolResults {
                step_id: "s".into(),
                parent_message_id: None,
                results: vec![resp],
            },
        ));
        assert!(!tr.pending.contains_key("c1"));
        let c = text(&tr.drain_commits(&opts()));
        assert!(c.contains("Bash(\"seq 10\")"), "{c}");
        assert!(c.contains("· local"));
        assert!(c.contains("ctrl+o to expand"), "{c}");
    }

    #[test]
    fn child_events_nest_under_the_invoking_call() {
        let mut tr = t();
        tr.apply(&ev("root", None, "parent", AgentEventType::RunStarted {}));
        tr.apply(&ev(
            "root",
            None,
            "parent",
            AgentEventType::ToolCalls {
                step_id: "s".into(),
                parent_message_id: None,
                tool_calls: vec![call(
                    "inv",
                    "invoke_agent",
                    serde_json::json!({"agent": "worker", "prompt": "do it"}),
                )],
            },
        ));
        let mut child = ev(
            "kid",
            Some("root"),
            "worker",
            AgentEventType::ToolExecutionStart {
                step_id: "s".into(),
                tool_call_id: "b1".into(),
                tool_call_name: "Bash".into(),
                input: serde_json::json!({"command": "ls"}),
            },
        );
        child.agent_event.as_mut().unwrap().child = Some(ChildRun {
            parent_tool_call_id: Some("inv".into()),
            title: Some("List files".into()),
            agent_type: None,
        });
        tr.apply(&child);
        tr.apply(&ev(
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
        ));
        assert_eq!(tr.cells.len(), 1, "no separate cell for the child");
        let live = text(&render_cell(&tr.cells[0], &opts()));
        assert!(live.contains("✓ done worker · 1 tool use"), "{live}");
        assert_eq!(tr.children, vec![("kid".to_string(), "worker".to_string())]);
    }

    #[test]
    fn background_children_are_known_from_task_ids() {
        let mut tr = t();
        tr.apply(&ev("root", None, "parent", AgentEventType::RunStarted {}));
        tr.apply(&ev(
            "root",
            None,
            "parent",
            AgentEventType::ToolCalls {
                step_id: "s".into(),
                parent_message_id: None,
                tool_calls: vec![call(
                    "inv",
                    "invoke_agent",
                    serde_json::json!({"agent": "asker", "mode": "background", "prompt": "ask"}),
                )],
            },
        ));
        let resp = ToolResponse::direct(
            "inv".into(),
            "invoke_agent".into(),
            serde_json::json!({"kind": "task_ids", "task_ids": ["bg1"]}),
        );
        tr.apply(&ev(
            "root",
            None,
            "parent",
            AgentEventType::ToolResults {
                step_id: "s".into(),
                parent_message_id: None,
                results: vec![resp],
            },
        ));
        assert_eq!(
            tr.open_children(),
            vec![("bg1".to_string(), "asker".to_string())]
        );
        tr.set_child_state("bg1", RunState::Done, Some("Ship it".into()));
        assert!(tr.open_children().is_empty());
        let c = text(&render_cell(&tr.cells[0], &opts()));
        assert!(c.contains("Ship it"), "{c}");
    }

    #[test]
    fn local_progress_and_logs_show_while_running() {
        let mut tr = t();
        tr.apply(&ev("root", None, "coder", AgentEventType::RunStarted {}));
        tr.apply(&ev(
            "root",
            None,
            "coder",
            AgentEventType::ToolCalls {
                step_id: "s".into(),
                parent_message_id: None,
                tool_calls: vec![call("c", "lookup_order", serde_json::json!({"id": "A-17"}))],
            },
        ));
        assert!(tr.apply_local(
            "c",
            &LocalToolEventKind::Progress {
                fraction: Some(0.5),
                message: Some("half".into())
            }
        ));
        tr.apply_local(
            "c",
            &LocalToolEventKind::Log {
                level: LogLevel::Warn,
                line: "slow".into(),
            },
        );
        let c = text(&render_cell(&tr.cells[0], &opts()));
        assert!(c.contains("· script"));
        assert!(c.contains(" 50%  half"), "{c}");
        assert!(c.contains("warn slow"), "{c}");
    }

    #[test]
    fn a_canceled_run_closes_its_waiting_child_call() {
        let mut tr = t();
        tr.apply(&ev("root", None, "parent", AgentEventType::RunStarted {}));
        tr.apply(&ev(
            "root",
            None,
            "parent",
            AgentEventType::ToolCalls {
                step_id: "s".into(),
                parent_message_id: None,
                tool_calls: vec![call(
                    "inv",
                    "invoke_agent",
                    serde_json::json!({"agent": "worker", "prompt": "go"}),
                )],
            },
        ));
        let mut child = ev("kid", Some("root"), "worker", AgentEventType::RunStarted {});
        child.agent_event.as_mut().unwrap().child = Some(ChildRun {
            parent_tool_call_id: Some("inv".into()),
            title: None,
            agent_type: None,
        });
        tr.apply(&child);
        tr.apply(&ev(
            "root",
            None,
            "parent",
            AgentEventType::RunError {
                message: "Task canceled".into(),
                code: None,
                usage: None,
            },
        ));
        let out = text(&tr.drain_commits(&opts()));
        assert!(out.contains("⏺ invoke_agent"), "{out}");
        assert!(out.contains("⊘ canceled worker"), "{out}");
        assert!(
            out.contains("Interrupted"),
            "later cells are not held back: {out}"
        );
        assert!(tr.live().is_empty());
    }

    #[test]
    fn results_of_local_calls_show_without_a_server_echo() {
        let mut tr = t();
        tr.apply(&ev("root", None, "coder", AgentEventType::RunStarted {}));
        tr.apply(&ev(
            "root",
            None,
            "coder",
            AgentEventType::ToolCalls {
                step_id: "s".into(),
                parent_message_id: None,
                tool_calls: vec![call("c", "lookup_order", serde_json::json!({"id": "A-17"}))],
            },
        ));
        let resp = ToolResponse::direct(
            "c".into(),
            "lookup_order".into(),
            serde_json::json!({"id": "A-17", "status": "shipped"}),
        );
        assert!(tr.apply_local("c", &LocalToolEventKind::Finished(Ok(resp))));
        let out = text(&tr.drain_commits(&opts()));
        assert!(out.contains("shipped"), "{out}");
        tr.apply(&ev(
            "root",
            None,
            "coder",
            AgentEventType::ToolCalls {
                step_id: "s".into(),
                parent_message_id: None,
                tool_calls: vec![call(
                    "d",
                    "Bash",
                    serde_json::json!({"command": "rm -rf /"}),
                )],
            },
        ));
        tr.apply_local(
            "d",
            &LocalToolEventKind::Finished(Err("The user denied this Bash call.".into())),
        );
        let out = text(&tr.drain_commits(&opts()));
        assert!(out.contains("The user denied this Bash call."), "{out}");
    }
}
