//! `distri`'s terminal UI: one inline app, modelled on Claude Code.
//!
//! Finished output goes into the terminal's own scrollback; a live region
//! at the bottom shows what is still happening, the prompt or input box and
//! a footer. Runs (this chat's turns, sub-agents, background children, any
//! run from `GET /v1/runs`) are followed in the same app; CLI tools answer
//! calls from all of them; every question to the person goes through the
//! [`distri_cli::interact`] broker and shows as one inline prompt.
//!
//! `app.rs` is the state machine (tested without a terminal), `view.rs`
//! draws the live region, `term.rs` writes rows, and this file is the I/O
//! loop that feeds the state machine and carries out its effects.

pub mod app;
pub mod cells;
pub mod editor;
pub mod markdown;
pub mod runs;
pub mod term;
pub mod view;
pub mod wrap;

use std::collections::HashMap;
use std::io::{BufWriter, IsTerminal, Stdout, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use distri::{AgentStreamClient, ContextHealth, Distri};
use distri_a2a::MessageSendParams;
use distri_types::AgentEventType;
use ratatui::crossterm::{
    cursor,
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{self, disable_raw_mode, enable_raw_mode},
};
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;

use distri_cli::interact::Ask;
use distri_cli::local_events::LocalEvents;
use distri_cli::permissions::PermissionEngine;

use app::{App, AppInit, Effect, Input};
use term::InlineTerm;

/// How often the run list refreshes.
const POLL_RUNS: Duration = Duration::from_secs(3);
const TICK: Duration = Duration::from_millis(80);

/// What a slash command the app does not handle itself asked for.
pub enum SlashOutcome {
    Continue,
    Exit,
}

/// The parts of the chat that live outside the TUI: building a turn's
/// request (agent card, connections context, the client's tool schemas) and
/// the commands that print and pick on a plain terminal.
#[async_trait::async_trait(?Send)]
pub trait Host {
    /// The agent id to stream and the request for one user message.
    async fn prepare_turn(
        &mut self,
        agent: &str,
        model: Option<&str>,
        thread_id: &str,
        text: &str,
    ) -> Result<(String, MessageSendParams)>;

    /// Run a command with the terminal in normal mode (it may print and
    /// prompt). It may change the agent, the model or the thread.
    async fn slash(
        &mut self,
        line: &str,
        agent: &mut String,
        model: &mut Option<String>,
        thread_id: &mut String,
        health: &Arc<RwLock<ContextHealth>>,
    ) -> Result<SlashOutcome>;
}

pub struct TuiDeps {
    pub client: Distri,
    pub stream: Arc<AgentStreamClient>,
    pub broker_rx: mpsc::Receiver<Ask>,
    pub events: LocalEvents,
    pub engine: PermissionEngine,
    /// Open with the run overlay showing (`distri sessions`, `/runs`).
    pub open_runs: bool,
    /// …listing every run, not only this chat's.
    pub all_runs: bool,
}

pub struct TuiExit {
    pub thread_id: String,
    pub model: Option<String>,
    pub history: Vec<String>,
}

/// True when both ends are a terminal: the TUI can run.
pub fn available() -> bool {
    std::io::stdout().is_terminal() && std::io::stdin().is_terminal()
}

/// Restores the terminal however the TUI ends.
struct TermGuard {
    enhanced: bool,
}

impl TermGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut out = std::io::stdout();
        execute!(out, EnableBracketedPaste)?;
        let enhanced = terminal::supports_keyboard_enhancement().unwrap_or(false);
        if enhanced {
            execute!(
                out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?;
        }
        Ok(Self { enhanced })
    }

    fn suspend(&self) {
        let mut out = std::io::stdout();
        if self.enhanced {
            let _ = execute!(out, PopKeyboardEnhancementFlags);
        }
        let _ = execute!(out, DisableBracketedPaste, cursor::Show);
        let _ = disable_raw_mode();
    }

    fn resume(&self) {
        let _ = enable_raw_mode();
        let mut out = std::io::stdout();
        let _ = execute!(out, EnableBracketedPaste);
        if self.enhanced {
            let _ = execute!(
                out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            );
        }
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        self.suspend();
    }
}

/// Terminal events → [`Input`], on a blocking thread, until `stop`. While
/// `paused` (a command owns the terminal) it reads nothing.
fn spawn_reader(
    tx: mpsc::Sender<Input>,
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
) -> JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        while !stop.load(Ordering::Relaxed) {
            if paused.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            match event::poll(Duration::from_millis(50)) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(_) => break,
            }
            let input = match event::read() {
                Ok(Event::Key(k)) if k.kind != KeyEventKind::Release => Input::Key(k),
                Ok(Event::Paste(s)) => Input::Paste(s),
                Ok(Event::Resize(w, h)) => Input::Resize(w, h),
                _ => continue,
            };
            if tx.blocking_send(input).is_err() {
                break;
            }
        }
    })
}

pub async fn run(host: &mut dyn Host, init: AppInit, deps: TuiDeps) -> Result<TuiExit> {
    let TuiDeps {
        client,
        stream,
        mut broker_rx,
        events,
        engine,
        open_runs,
        all_runs,
    } = deps;
    let guard = TermGuard::enter()?;
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), DisableBracketedPaste, cursor::Show);
        prev_hook(info);
    }));

    let (w, h) = terminal::size().unwrap_or((100, 30));
    let row = cursor::position()
        .map(|(_, r)| r)
        .unwrap_or(h.saturating_sub(1));
    let mut term: InlineTerm<BufWriter<Stdout>> = InlineTerm::new(
        BufWriter::with_capacity(1 << 16, std::io::stdout()),
        (w, h),
        row,
    );
    let mut app = App::new(AppInit {
        width: w,
        height: h,
        ..init
    });
    let health: Arc<RwLock<ContextHealth>> = Arc::new(RwLock::new(ContextHealth::default()));

    let (tx, mut rx) = mpsc::channel::<Input>(1024);
    let stop = Arc::new(AtomicBool::new(false));
    let paused = Arc::new(AtomicBool::new(false));
    let reader = spawn_reader(tx.clone(), stop.clone(), paused.clone());
    let ticker = {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(TICK);
            loop {
                every.tick().await;
                if tx.send(Input::Tick).await.is_err() {
                    break;
                }
            }
        })
    };
    let asks = {
        let tx = tx.clone();
        tokio::spawn(async move {
            while let Some(ask) = broker_rx.recv().await {
                if tx.send(Input::Ask(ask)).await.is_err() {
                    break;
                }
            }
        })
    };
    let locals = {
        let tx = tx.clone();
        let mut sub = events.subscribe();
        tokio::spawn(async move {
            loop {
                match sub.recv().await {
                    Ok(ev) => {
                        if tx.send(Input::Local(ev)).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        })
    };

    let mut tasks: HashMap<String, JoinHandle<()>> = HashMap::new();
    let mut turn_task: Option<JoinHandle<()>> = None;
    let mut last_poll = Instant::now();
    app.mode_label = engine.mode().to_string();
    let mut pending = app.start();
    if open_runs {
        pending.extend(app.handle(Input::Key(event::KeyEvent::new(
            event::KeyCode::Char('t'),
            event::KeyModifiers::CONTROL,
        ))));
        if let Some(o) = app.overlay.as_mut() {
            o.all = all_runs;
        }
        pending.push(Effect::LoadRuns(app.runs_query()));
    }

    let result: Result<()> = async {
        'outer: loop {
            while !pending.is_empty() {
                let effects: Vec<Effect> = std::mem::take(&mut pending);
                for effect in effects {
                    match effect {
                        Effect::Quit => break 'outer,
                        Effect::SendTurn { turn, text } => {
                            match host
                                .prepare_turn(
                                    &app.agent,
                                    app.model.as_deref(),
                                    &app.thread_id,
                                    &text,
                                )
                                .await
                            {
                                Ok((agent_id, params)) => {
                                    if let Some(t) = app.turn.as_mut() {
                                        t.agent_id = Some(agent_id.clone());
                                    }
                                    health.write().await.reset_run_tokens();
                                    let sc = stream.clone();
                                    let tx = tx.clone();
                                    turn_task = Some(tokio::spawn(async move {
                                        let items = tx.clone();
                                        let res = sc
                                            .stream_agent(&agent_id, params, move |item| {
                                                let items = items.clone();
                                                async move {
                                                    let _ = items
                                                        .send(Input::Turn {
                                                            turn,
                                                            item: Box::new(item),
                                                        })
                                                        .await;
                                                }
                                            })
                                            .await;
                                        let _ = tx
                                            .send(Input::TurnEnded {
                                                turn,
                                                error: res.err().map(|e| e.to_string()),
                                            })
                                            .await;
                                    }));
                                }
                                Err(e) => pending.extend(app.handle(Input::TurnFailedToStart {
                                    turn,
                                    error: format!("{e:#}"),
                                })),
                            }
                        }
                        Effect::Cancel { agent_id, task_id } => {
                            let sc = stream.clone();
                            let tx = tx.clone();
                            tokio::spawn(async move {
                                let res = sc.cancel_task(&agent_id, &task_id).await;
                                let _ = tx
                                    .send(Input::CancelDone {
                                        task_id,
                                        error: res.err().map(|e| e.to_string()),
                                    })
                                    .await;
                            });
                        }
                        Effect::Follow { agent_id, task_id } => {
                            let sc = stream.clone();
                            let tx = tx.clone();
                            let id = task_id.clone();
                            let handle = tokio::spawn(async move {
                                let items = tx.clone();
                                let sid = id.clone();
                                let res = sc
                                    .resubscribe_task(&agent_id, &id, move |item| {
                                        let items = items.clone();
                                        let task_id = sid.clone();
                                        async move {
                                            let _ = items
                                                .send(Input::Follow {
                                                    task_id,
                                                    item: Box::new(item),
                                                })
                                                .await;
                                        }
                                    })
                                    .await;
                                let _ = tx
                                    .send(Input::FollowEnded {
                                        task_id: id,
                                        error: res.err().map(|e| e.to_string()),
                                    })
                                    .await;
                            });
                            if let Some(old) = tasks.insert(task_id, handle) {
                                old.abort();
                            }
                        }
                        Effect::RunTool {
                            agent_id,
                            event,
                            call,
                        } => {
                            let sc = stream.clone();
                            let tx = tx.clone();
                            tokio::spawn(async move {
                                let res = sc.answer_with_registry(&agent_id, &event, &call).await;
                                let _ = tx
                                    .send(Input::AnswerDone {
                                        call_id: call.tool_call_id,
                                        error: res.err().map(|e| e.to_string()),
                                    })
                                    .await;
                            });
                        }
                        Effect::SendText {
                            agent_id,
                            call,
                            text,
                        } => {
                            let sc = stream.clone();
                            let tx = tx.clone();
                            tokio::spawn(async move {
                                let res = sc.answer_with_text(&agent_id, &call, &text).await;
                                let _ = tx
                                    .send(Input::AnswerDone {
                                        call_id: call.tool_call_id,
                                        error: res.err().map(|e| e.to_string()),
                                    })
                                    .await;
                            });
                        }
                        Effect::LoadRuns(query) => {
                            if app.runs_supported == Some(false) {
                                continue;
                            }
                            last_poll = Instant::now();
                            let client = client.clone();
                            let tx = tx.clone();
                            tokio::spawn(async move {
                                let res = client.list_runs(&query).await.map_err(|e| e.to_string());
                                let _ = tx.send(Input::RunsLoaded(res)).await;
                            });
                        }
                        Effect::CycleMode => {
                            let mode = engine.cycle_mode();
                            app.mode_label = mode.to_string();
                            app.set_status(format!("permission mode: {mode} (shift+tab)"));
                        }
                        Effect::Slash(line) => {
                            // The command owns the terminal until it returns.
                            let hist = app.take_history();
                            term.insert_history(&hist)?;
                            term.clear_live()?;
                            paused.store(true, Ordering::Relaxed);
                            tokio::time::sleep(Duration::from_millis(60)).await;
                            guard.suspend();
                            let outcome = host
                                .slash(
                                    &line,
                                    &mut app.agent,
                                    &mut app.model,
                                    &mut app.thread_id,
                                    &health,
                                )
                                .await;
                            let _ = std::io::stdout().flush();
                            guard.resume();
                            let row = cursor::position()
                                .map(|(_, r)| r)
                                .unwrap_or(term.height() - 1);
                            term.set_top(row);
                            paused.store(false, Ordering::Relaxed);
                            match outcome {
                                Ok(SlashOutcome::Exit) => break 'outer,
                                Ok(SlashOutcome::Continue) => {}
                                Err(e) => app.chat.push(cells::Cell::Error {
                                    text: format!("{line}: {e:#}"),
                                }),
                            }
                        }
                    }
                }
            }

            let hist = app.take_history();
            if !hist.is_empty() {
                term.insert_history(&hist)?;
            }
            let (rows, cursor) = view::draw(&mut app);
            term.draw_live(&rows, cursor)?;

            let Some(first) = rx.recv().await else { break };
            let mut batch = vec![first];
            while batch.len() < 256 {
                match rx.try_recv() {
                    Ok(i) => batch.push(i),
                    Err(_) => break,
                }
            }
            for input in batch {
                match &input {
                    Input::Resize(w, h) => term.resize(*w, *h)?,
                    Input::Turn { item, .. } => {
                        if let Some(ev) = &item.agent_event {
                            let mut hl = health.write().await;
                            match &ev.event {
                                AgentEventType::ContextBudgetUpdate { budget, .. } => {
                                    hl.update_from_budget(budget)
                                }
                                AgentEventType::StepCompleted {
                                    context_budget,
                                    usage,
                                    ..
                                } => {
                                    if let Some(b) = context_budget {
                                        hl.update_from_budget(b);
                                    }
                                    if let Some(u) = usage {
                                        hl.update_from_usage(u);
                                    }
                                }
                                AgentEventType::RunFinished {
                                    context_budget: Some(b),
                                    ..
                                } => hl.update_from_budget(b),
                                _ => {}
                            }
                            app.status_line = strip_ansi(&hl.format_status_line());
                        }
                    }
                    Input::Tick
                        if last_poll.elapsed() > POLL_RUNS && app.runs_supported != Some(false) =>
                    {
                        pending.push(Effect::LoadRuns(app.runs_query()));
                        last_poll = Instant::now();
                    }
                    _ => {}
                }
                pending.extend(app.handle(input));
            }
            if app.quit {
                break;
            }
        }
        Ok(())
    }
    .await;

    // Leave what is on screen as a log: the rest of the transcript goes into
    // the scrollback, then the live region is erased.
    let mut rest = app.take_history();
    let opts = app.opts();
    for c in app.chat.live() {
        rest.extend(cells::render_cell(c, &opts));
    }
    let _ = term.insert_history(&rest);
    let _ = term.clear_live();
    stop.store(true, Ordering::Relaxed);
    ticker.abort();
    asks.abort();
    locals.abort();
    if let Some(t) = turn_task {
        t.abort();
    }
    for (_, h) in tasks.drain() {
        h.abort();
    }
    drop(guard);
    let _ = reader.await;
    let _ = std::panic::take_hook();
    result?;
    Ok(TuiExit {
        thread_id: app.thread_id.clone(),
        model: app.model.clone(),
        history: app.editor.history().to_vec(),
    })
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Files under `root` for `@` completion: relative paths, skipping VCS,
/// build and dependency folders, at most `limit`.
pub fn project_files(root: &Path, limit: usize) -> Vec<String> {
    const SKIP: &[&str] = &[
        ".git",
        "target",
        "node_modules",
        "dist",
        "build",
        ".next",
        ".venv",
        "venv",
        "__pycache__",
    ];
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            if SKIP.contains(&name.as_str()) || (name.starts_with('.') && name != ".distri") {
                continue;
            }
            let path = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(t) if t.is_file() => {
                    if let Ok(rel) = path.strip_prefix(root) {
                        out.push(rel.to_string_lossy().into_owned());
                        if out.len() >= limit {
                            return out;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out.sort();
    out
}

/// The slash commands the input offers, with what each does.
pub fn command_list() -> Vec<(String, String)> {
    [
        ("/help", "keys and commands"),
        ("/runs", "every run in this chat (ctrl+t)"),
        ("/agents", "pick an agent"),
        ("/agent", "switch agent: /agent <name>"),
        ("/models", "list models"),
        ("/model", "set the model: /model <name> | auto"),
        ("/mode", "cycle the permission mode (shift+tab)"),
        ("/context", "context window breakdown"),
        ("/compact", "compact this thread's context"),
        ("/resume", "pick a thread to resume"),
        ("/clear", "start a new conversation"),
        ("/available-tools", "tools this client can run"),
        ("/traces", "recent traces"),
        ("/usage", "token usage"),
        ("/exit", "quit"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect()
}
