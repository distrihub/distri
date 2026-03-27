use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use rustyline::completion::{Completer, Pair};
use rustyline::hint::Hinter;
use rustyline::highlight::Highlighter;
use rustyline::validate::Validator;
use rustyline::{Cmd, ConditionalEventHandler, Event, EventContext, Helper, Movement, RepeatCount};

/// Rustyline helper for Distri CLI — provides slash-command completion and placeholder hint.
pub struct DistriHelper {
    slash_commands: Vec<String>,
    matcher: SkimMatcherV2,
    show_tools: Arc<AtomicBool>,
}

impl DistriHelper {
    pub fn new(show_tools: Arc<AtomicBool>) -> Self {
        let slash_commands = vec![
            "/help".to_string(),
            "/agents".to_string(),
            "/agent".to_string(),
            "/models".to_string(),
            "/model".to_string(),
            "/available-tools".to_string(),
            "/resume".to_string(),
            "/clear".to_string(),
            "/exit".to_string(),
            "/quit".to_string(),
        ];

        Self {
            slash_commands,
            matcher: SkimMatcherV2::default(),
            show_tools,
        }
    }
}

/// Ctrl+O handler — toggles tool output visibility.
pub struct ToggleToolsHandler {
    pub show_tools: Arc<AtomicBool>,
}

impl ConditionalEventHandler for ToggleToolsHandler {
    fn handle(
        &self,
        _evt: &Event,
        _n: RepeatCount,
        _positive: bool,
        _ctx: &EventContext,
    ) -> Option<Cmd> {
        let new_val = !self.show_tools.load(Ordering::Relaxed);
        self.show_tools.store(new_val, Ordering::Relaxed);
        // Move cursor to beginning of line to trigger a hint refresh
        // so the user sees the updated "[tools hidden]" status
        Some(Cmd::Move(Movement::BeginningOfLine))
    }
}

impl Validator for DistriHelper {}
impl Highlighter for DistriHelper {}
impl Helper for DistriHelper {}

impl Completer for DistriHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        if !line.starts_with('/') {
            return Ok((0, vec![]));
        }

        let input = &line[..pos];
        let mut matches: Vec<(i64, &String)> = self
            .slash_commands
            .iter()
            .filter_map(|cmd| {
                self.matcher
                    .fuzzy_match(cmd, input)
                    .map(|score| (score, cmd))
            })
            .collect();

        matches.sort_by(|a, b| b.0.cmp(&a.0));

        let pairs = matches
            .into_iter()
            .take(15)
            .map(|(_, cmd)| Pair {
                display: cmd.clone(),
                replacement: cmd.clone(),
            })
            .collect();

        Ok((0, pairs))
    }
}

impl Hinter for DistriHelper {
    type Hint = String;

    fn hint(&self, line: &str, _pos: usize, _ctx: &rustyline::Context<'_>) -> Option<String> {
        if line.is_empty() {
            let tools_status = if self.show_tools.load(Ordering::Relaxed) {
                ""
            } else {
                " [tools hidden · Ctrl+O]"
            };
            Some(format!(
                "  /help for commands... Ask me anything{}",
                tools_status
            ))
        } else {
            None
        }
    }
}
