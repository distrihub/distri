//! The run tree (`GET /v1/runs`) as an overlay over the chat (Ctrl+T,
//! `/runs`), and the plain listing `distri sessions` prints without a TTY.

use std::time::Duration;

use distri::Distri;
use distri_types::{RunListQuery, RunListResponse, RunState, RunSummary};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::wrap::display_width;

/// A run still "working" with no event for this long and nobody streaming
/// it is stale: hidden unless asked for.
const STALE_AFTER: Duration = Duration::from_secs(30 * 60);

pub struct RunsOverlay {
    pub all: bool,
    pub show_stale: bool,
    pub runs: Vec<RunSummary>,
    pub has_more: bool,
    pub rows: Vec<(usize, String)>,
    pub selected: usize,
    pub confirm_cancel: Option<String>,
    pub loaded: bool,
}

impl RunsOverlay {
    pub fn new() -> Self {
        Self {
            all: false,
            show_stale: false,
            runs: Vec::new(),
            has_more: false,
            rows: Vec::new(),
            selected: 0,
            confirm_cancel: None,
            loaded: false,
        }
    }

    pub fn set_runs(&mut self, r: &RunListResponse) {
        let keep = self.rows.get(self.selected).map(|(_, id)| id.clone());
        self.runs = r.runs.clone();
        self.has_more = r.has_more;
        self.loaded = true;
        self.rebuild();
        if let Some(id) = keep {
            if let Some(i) = self.rows.iter().position(|(_, t)| *t == id) {
                self.selected = i;
            }
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }

    fn rebuild(&mut self) {
        let now = chrono::Utc::now().timestamp_millis();
        self.rows = self
            .runs
            .iter()
            .filter(|r| self.show_stale || !is_stale(r, now))
            .flat_map(|r| r.flatten().into_iter().map(|(d, n)| (d, n.task_id.clone())))
            .collect();
    }

    pub fn stale_count(&self) -> usize {
        let now = chrono::Utc::now().timestamp_millis();
        self.runs.iter().filter(|r| is_stale(r, now)).count()
    }

    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn down(&mut self) {
        if self.selected + 1 < self.rows.len() {
            self.selected += 1;
        }
    }

    pub fn find(&self, task_id: &str) -> Option<&RunSummary> {
        self.runs.iter().find_map(|r| r.find(task_id))
    }

    pub fn selected_run(&self) -> Option<&RunSummary> {
        self.rows
            .get(self.selected)
            .and_then(|(_, id)| self.find(id))
    }

    pub fn agent_of(&self, task_id: &str) -> Option<String> {
        self.find(task_id).and_then(|r| r.agent_id.clone())
    }

    pub fn render(&mut self, width: u16, max_rows: usize) -> Vec<Line<'static>> {
        self.rebuild();
        let w = width as usize;
        let waiting = self
            .runs
            .iter()
            .flat_map(|r| r.flatten())
            .filter(|(_, n)| n.state == RunState::NeedsInput)
            .count();
        let scope = if self.all { "all runs" } else { "this chat" };
        let mut title = format!("╭─ Runs · {scope} ");
        if waiting > 0 {
            title.push_str(&format!("─ {waiting} need you "));
        }
        let hint = " t: this chat/all ─╮";
        let fill = w.saturating_sub(display_width(&title) + display_width(hint));
        let border = Style::new().fg(Color::DarkGray);
        let mut out = vec![Line::from(vec![
            Span::styled(title, border),
            Span::styled("─".repeat(fill), border),
            Span::styled(hint, border),
        ])];
        if !self.loaded {
            out.push(Line::styled("│ loading…", border));
        } else if self.rows.is_empty() {
            out.push(Line::styled("│ no runs yet", border));
        }
        let body_rows = max_rows.saturating_sub(3).max(1);
        let start = self.selected.saturating_sub(body_rows.saturating_sub(1));
        for (i, (depth, id)) in self.rows.iter().enumerate().skip(start).take(body_rows) {
            let Some(run) = self.find(id) else { continue };
            let mut line = row_line(*depth, run, i == self.selected, w);
            let used: usize = line.spans.iter().map(|s| display_width(&s.content)).sum();
            line.spans
                .push(Span::raw(" ".repeat(w.saturating_sub(used + 1))));
            line.spans.push(Span::styled("│", border));
            out.push(line);
        }
        let stale = self.stale_count();
        let mut foot = String::from("╰─ ↑↓ move · ⏎ follow / answer · x cancel tree · r refresh");
        if stale > 0 {
            foot.push_str(&format!(
                " · s {} {stale} stale",
                if self.show_stale { "hide" } else { "show" }
            ));
        }
        foot.push_str(" · esc ");
        let fill = w.saturating_sub(display_width(&foot) + 2);
        out.push(Line::styled(
            format!("{foot}{}─╯", "─".repeat(fill)),
            border,
        ));
        out
    }
}

fn is_stale(r: &RunSummary, now_ms: i64) -> bool {
    r.state == RunState::Working
        && !r.needs_input
        && now_ms - r.latest_activity() > STALE_AFTER.as_millis() as i64
}

pub fn badge(state: RunState) -> (&'static str, Color) {
    match state {
        RunState::NeedsInput => ("● needs you", Color::Yellow),
        RunState::Working => ("◐ working", Color::Cyan),
        RunState::Done => ("✓ done", Color::DarkGray),
        RunState::Failed => ("✗ failed", Color::LightRed),
        RunState::Canceled => ("⊘ canceled", Color::DarkGray),
    }
}

/// "12s", "4m", "3h", "2d" since `ms` (epoch millis).
pub fn age(ms: i64) -> String {
    let secs = ((chrono::Utc::now().timestamp_millis() - ms) / 1000).max(0);
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

/// One line for people: no JSON, no newlines.
pub fn humanize(s: &str) -> String {
    let t = s.trim();
    let t = if t.starts_with('{') || t.starts_with('[') {
        match serde_json::from_str::<serde_json::Value>(t) {
            Ok(v) => distri_formatter::state::summarize_args(&v, 80),
            Err(_) => t.to_string(),
        }
    } else {
        t.to_string()
    };
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn row_line(depth: usize, run: &RunSummary, selected: bool, width: usize) -> Line<'static> {
    let (label, color) = badge(run.state);
    let mut badge_style = Style::new().fg(color);
    if run.state == RunState::NeedsInput {
        badge_style = badge_style.add_modifier(Modifier::BOLD);
    }
    let indent = if depth == 0 {
        String::new()
    } else {
        format!("{}└ ", "  ".repeat(depth - 1))
    };
    let name = run
        .title
        .clone()
        .or_else(|| run.intent.clone().map(|i| humanize(&i)))
        .unwrap_or_else(|| run.agent_id.clone().unwrap_or_else(|| "?".into()));
    let name: String = name.chars().take(40).collect();
    let mut spans = vec![
        Span::raw(if selected { "│❯" } else { "│ " }),
        Span::raw(indent),
        Span::styled(format!("{label:<12}"), badge_style),
        Span::raw(" "),
        Span::styled(name, Style::new().add_modifier(Modifier::BOLD)),
        Span::styled(
            format!(
                "  {} · {}",
                run.agent_id.as_deref().unwrap_or("?"),
                age(run.latest_activity())
            ),
            Style::new().fg(Color::DarkGray),
        ),
    ];
    if run.needs_input && run.state != RunState::NeedsInput {
        spans.push(Span::styled("  ↓ waiting", Style::new().fg(Color::Yellow)));
    }
    if let Some(last) = run.last_event.as_deref() {
        let used: usize = spans.iter().map(|s| display_width(&s.content)).sum();
        let room = width.saturating_sub(used + 4);
        if room > 8 {
            let h: String = humanize(last).chars().take(room).collect();
            spans.push(Span::styled(
                format!("  {h}"),
                Style::new().fg(Color::DarkGray),
            ));
        }
    }
    let mut line = Line::from(spans);
    if selected {
        line.style = Style::new().bg(Color::Indexed(236));
    }
    line
}

/// No terminal (piped, CI): print the tree once.
pub async fn print_plain(client: &Distri, thread_id: Option<String>) -> anyhow::Result<()> {
    let query = RunListQuery {
        thread_id,
        limit: Some(50),
        ..Default::default()
    };
    let resp = client.list_runs(&query).await?;
    if resp.runs.is_empty() {
        println!("No runs.");
    }
    for line in plain_lines(&resp.runs) {
        println!("{line}");
    }
    if resp.has_more {
        println!("… more runs not shown");
    }
    Ok(())
}

pub fn plain_lines(runs: &[RunSummary]) -> Vec<String> {
    runs.iter()
        .flat_map(|r| r.flatten())
        .map(|(depth, n)| {
            let (label, _) = badge(n.state);
            format!(
                "{}{:<12} {:<24} {:>4}  {}  {}",
                "  ".repeat(depth),
                label,
                n.agent_id.as_deref().unwrap_or("?"),
                age(n.latest_activity()),
                n.task_id,
                humanize(n.title.as_deref().or(n.last_event.as_deref()).unwrap_or(""))
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::runs::fixture;

    #[test]
    fn plain_output_lists_the_tree() {
        let lines = plain_lines(&fixture::multi_run().runs);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("✓ done"));
        assert!(lines[1].starts_with("  ● needs you"));
        assert!(lines[1].contains(fixture::ASKER_AGENT));
    }

    #[test]
    fn overlay_rows_and_waiting_count() {
        let mut o = RunsOverlay::new();
        o.set_runs(&fixture::multi_run());
        let lines = o.render(100, 20);
        let text: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        assert!(text[0].contains("1 need you"), "{}", text[0]);
        assert!(text
            .iter()
            .any(|l| l.contains("● needs you") && l.contains("Ask the release owner")));
        o.down();
        assert_eq!(
            o.selected_run().map(|r| r.task_id.as_str()),
            Some(fixture::ASKER_TASK_ID)
        );
    }

    #[test]
    fn json_events_read_as_text() {
        assert_eq!(
            humanize("{\"status\": \"completed\", \"steps\": [1,2]}"),
            "status: \"completed\", steps: 2 items"
        );
        assert_eq!(humanize("line one\nline two"), "line one line two");
    }
}
