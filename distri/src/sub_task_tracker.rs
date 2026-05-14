//! Sub-task collapse printer for the CLI.
//!
//! Mirrors how Claude Code surfaces the `Task` tool: a sub-agent run
//! shows up as a single tool-call-style line in the parent's output,
//! not a wall of streamed text. The body — tool calls, intermediate
//! messages, deltas — is suppressed by default.
//!
//! Default (non-verbose):
//! ```text
//! ⏺ subtask(researcher)
//!   ⎿ done (3.4s)
//! ```
//! `--verbose`:
//! ```text
//! ┌─ subtask · researcher ─────────
//! │ ⏺ search_web(...)
//! │ ⎿ 5 results
//! │ ◆ researcher: here is what I found…
//! └─ ✓ researcher (3.4s)
//! ```
//!
//! Every `AgentEvent` already carries `task_id` + `parent_task_id` from
//! the orchestrator, so the tracker only needs to walk the envelope —
//! no protocol changes.

use std::collections::HashMap;
use std::time::Instant;

use distri_types::{AgentEvent, AgentEventType};

use distri_formatter::colors::{COLOR_BRIGHT_CYAN, COLOR_GRAY, COLOR_RED, COLOR_RESET, COLOR_YELLOW};

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct TaskNode {
    task_id: String,
    parent_task_id: Option<String>,
    agent_id: String,
    depth: usize,
    started_at: Option<Instant>,
    /// Verbose-mode fence header has been printed for this task.
    fence_header_printed: bool,
    /// Verbose-mode fence footer has been printed for this task.
    fence_footer_printed: bool,
    /// Compact `⏺ subtask(agent)` start line has been printed.
    compact_start_printed: bool,
    /// Compact `  ⎿ done (Xs)` end line has been printed.
    compact_end_printed: bool,
}

/// Tells the caller (`EventPrinter`) whether the underlying handler
/// should print this event or stay silent — the tracker has already
/// rendered whatever the user needs to see for this sub-task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressDecision {
    /// Event belongs to the root task, or we're in verbose mode and
    /// the regular printer should still write its line(s).
    Print,
    /// Sub-task event in non-verbose mode — the tracker already
    /// emitted the collapsed tool-call-style line; the printer should
    /// drop this event silently.
    Suppress,
}

/// Tracks sub-task lineage and prints either the compact tool-call
/// summary (default) or the fenced full body (verbose).
#[derive(Default)]
pub struct SubTaskTracker {
    tasks: HashMap<String, TaskNode>,
    last_task_id: Option<String>,
}

#[allow(dead_code)]
impl SubTaskTracker {
    pub fn new() -> Self {
        Self::default()
    }

    fn indent(depth: usize) -> String {
        let mut s = String::new();
        for _ in 0..depth {
            s.push_str("│ ");
        }
        s
    }

    /// Process a freshly-arrived event. Updates the task tree, emits
    /// any header/footer/status lines this transition warrants, and
    /// returns whether the caller should print the event itself.
    pub fn handle(&mut self, event: &AgentEvent, verbose: bool) -> SuppressDecision {
        let task_id = event.task_id.clone();
        if task_id.is_empty() {
            return SuppressDecision::Print;
        }

        // Insert / backfill the node up front so depth lookups work
        // even on the first event for a task.
        let parent_task_id = event.parent_task_id.clone();
        let computed_depth = parent_task_id
            .as_ref()
            .and_then(|p| self.tasks.get(p).map(|n| n.depth + 1))
            .unwrap_or(0);
        let node = self.tasks.entry(task_id.clone()).or_insert(TaskNode {
            task_id: task_id.clone(),
            parent_task_id: parent_task_id.clone(),
            agent_id: event.agent_id.clone(),
            depth: computed_depth,
            started_at: Some(Instant::now()),
            fence_header_printed: false,
            fence_footer_printed: false,
            compact_start_printed: false,
            compact_end_printed: false,
        });
        if node.agent_id.is_empty() && !event.agent_id.is_empty() {
            node.agent_id = event.agent_id.clone();
        }
        if node.parent_task_id.is_none() && parent_task_id.is_some() {
            node.parent_task_id = parent_task_id.clone();
            node.depth = computed_depth;
        }

        // Root task: nothing to do, regular printer handles it.
        if node.depth == 0 {
            self.last_task_id = Some(task_id);
            return SuppressDecision::Print;
        }

        // Verbose mode: emit fence headers/footers around the body and
        // let the regular printer write the body itself.
        if verbose {
            let transitioned = self
                .last_task_id
                .as_ref()
                .map(|prev| prev != &task_id)
                .unwrap_or(true);
            if transitioned {
                self.print_fence_header_if_needed(&task_id);
            }
            self.last_task_id = Some(task_id.clone());
            if let AgentEventType::RunFinished { success, .. } = &event.event {
                self.print_fence_footer(&task_id, *success);
            } else if let AgentEventType::RunError { .. } = &event.event {
                self.print_fence_footer(&task_id, false);
            }
            return SuppressDecision::Print;
        }

        // Non-verbose, sub-task: render the compact tool-call summary
        // on lifecycle events, suppress everything else.
        match &event.event {
            AgentEventType::RunStarted {} => {
                self.print_compact_start(&task_id);
            }
            AgentEventType::RunFinished { success, .. } => {
                self.print_compact_end(&task_id, *success);
            }
            AgentEventType::RunError { .. } => {
                self.print_compact_end(&task_id, false);
            }
            _ => {}
        }
        self.last_task_id = Some(task_id);
        SuppressDecision::Suppress
    }

    fn print_compact_start(&mut self, task_id: &str) {
        let Some(node) = self.tasks.get_mut(task_id) else {
            return;
        };
        if node.compact_start_printed {
            return;
        }
        // One indent level per ancestor, like the verbose fence — so a
        // sub-sub-task still nests inside its parent's compact block.
        let indent = Self::indent(node.depth.saturating_sub(1));
        let agent = if node.agent_id.is_empty() {
            "subtask"
        } else {
            &node.agent_id
        };
        println!(
            "{}{}⏺ subtask({}){}",
            indent, COLOR_YELLOW, agent, COLOR_RESET
        );
        node.compact_start_printed = true;
    }

    fn print_compact_end(&mut self, task_id: &str, success: bool) {
        let Some(node) = self.tasks.get_mut(task_id) else {
            return;
        };
        if node.compact_end_printed || !node.compact_start_printed {
            return;
        }
        let indent = Self::indent(node.depth.saturating_sub(1));
        let elapsed = node
            .started_at
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        let (glyph, color, label) = if success {
            ("✓", COLOR_GRAY, "done")
        } else {
            ("✖", COLOR_RED, "failed")
        };
        println!(
            "{}  {}⎿ {} {} ({:.1}s){}",
            indent, color, glyph, label, elapsed, COLOR_RESET
        );
        node.compact_end_printed = true;
    }

    fn print_fence_header_if_needed(&mut self, task_id: &str) {
        let Some(node) = self.tasks.get_mut(task_id) else {
            return;
        };
        if node.fence_header_printed {
            return;
        }
        let indent = Self::indent(node.depth.saturating_sub(1));
        let agent = if node.agent_id.is_empty() {
            "subtask"
        } else {
            &node.agent_id
        };
        println!(
            "{}{}┌─ subtask · {}{}",
            indent, COLOR_BRIGHT_CYAN, agent, COLOR_RESET
        );
        node.fence_header_printed = true;
    }

    fn print_fence_footer(&mut self, task_id: &str, success: bool) {
        let Some(node) = self.tasks.get_mut(task_id) else {
            return;
        };
        if node.fence_footer_printed || !node.fence_header_printed {
            return;
        }
        let indent = Self::indent(node.depth.saturating_sub(1));
        let elapsed = node
            .started_at
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        let glyph = if success { "✓" } else { "✖" };
        let color = if success { COLOR_GRAY } else { COLOR_RED };
        let agent = if node.agent_id.is_empty() {
            "subtask"
        } else {
            &node.agent_id
        };
        println!(
            "{}{}└─ {} {} ({:.1}s){}",
            indent, color, glyph, agent, elapsed, COLOR_RESET
        );
        node.fence_footer_printed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::AgentEvent;

    fn ev(task_id: &str, parent: Option<&str>, agent: &str, event: AgentEventType) -> AgentEvent {
        AgentEvent {
            timestamp: chrono::Utc::now(),
            thread_id: "t".into(),
            run_id: "r".into(),
            event,
            task_id: task_id.into(),
            parent_task_id: parent.map(|s| s.into()),
            agent_id: agent.into(),
            user_id: None,
            identifier_id: None,
            workspace_id: None,
            channel_id: None,
        }
    }

    #[test]
    fn root_events_always_print() {
        let mut t = SubTaskTracker::new();
        let d = t.handle(
            &ev("root", None, "main", AgentEventType::RunStarted {}),
            false,
        );
        assert_eq!(d, SuppressDecision::Print);
    }

    #[test]
    fn subtask_body_suppressed_by_default() {
        let mut t = SubTaskTracker::new();
        t.handle(
            &ev("root", None, "main", AgentEventType::RunStarted {}),
            false,
        );
        t.handle(
            &ev("a", Some("root"), "alpha", AgentEventType::RunStarted {}),
            false,
        );
        // A streamed text content event from a sub-task — should be
        // dropped on the floor in non-verbose mode.
        let d = t.handle(
            &ev(
                "a",
                Some("root"),
                "alpha",
                AgentEventType::TextMessageContent {
                    message_id: "m".into(),
                    delta: "hello".into(),
                    step_id: "s".into(),
                    stripped_content: None,
                },
            ),
            false,
        );
        assert_eq!(d, SuppressDecision::Suppress);
    }

    #[test]
    fn subtask_body_visible_in_verbose() {
        let mut t = SubTaskTracker::new();
        t.handle(
            &ev("root", None, "main", AgentEventType::RunStarted {}),
            true,
        );
        t.handle(
            &ev("a", Some("root"), "alpha", AgentEventType::RunStarted {}),
            true,
        );
        let d = t.handle(
            &ev(
                "a",
                Some("root"),
                "alpha",
                AgentEventType::TextMessageContent {
                    message_id: "m".into(),
                    delta: "hello".into(),
                    step_id: "s".into(),
                    stripped_content: None,
                },
            ),
            true,
        );
        assert_eq!(d, SuppressDecision::Print);
    }
}
