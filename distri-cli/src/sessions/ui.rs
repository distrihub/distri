//! Drawing for `distri sessions`. Reads [`SessionsState`]; never changes it.

use ansi_to_tui::IntoText;
use distri_types::{RunState, RunSummary};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use super::state::{short, Attached, Mode, Scope, SessionsState};

pub fn draw(f: &mut Frame, s: &SessionsState) {
    let [body, footer] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).areas(f.area());
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)]).areas(body);

    draw_list(f, s, left);
    match &s.attached {
        Some(a) => draw_attached(f, s, a, right),
        None => draw_details(f, s, right),
    }
    draw_footer(f, s, footer);
}

/// Badge text and colour for a run state.
pub fn badge(state: RunState) -> (&'static str, Color) {
    match state {
        RunState::NeedsInput => ("● needs you", Color::Yellow),
        RunState::Working => ("◐ working", Color::Cyan),
        RunState::Done => ("✓ done", Color::DarkGray),
        RunState::Failed => ("✗ failed", Color::Red),
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

fn row_line(depth: usize, run: &RunSummary, attached: bool) -> Line<'static> {
    let (label, color) = badge(run.state);
    let mut badge_style = Style::default().fg(color);
    if run.state == RunState::NeedsInput {
        badge_style = badge_style.add_modifier(Modifier::BOLD);
    }
    let indent = if depth == 0 {
        String::new()
    } else {
        format!("{}└ ", "  ".repeat(depth - 1))
    };
    let mut spans = vec![
        Span::raw(indent),
        Span::styled(format!("{label:<12}"), badge_style),
        Span::raw(" "),
        Span::styled(
            run.agent_id.clone().unwrap_or_else(|| "?".into()),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}", age(run.latest_activity())),
            Style::default().fg(Color::DarkGray),
        ),
    ];
    // A finished parent with something waiting underneath.
    if run.needs_input && run.state != RunState::NeedsInput {
        spans.push(Span::styled(
            "  ↓ waiting",
            Style::default().fg(Color::Yellow),
        ));
    }
    if attached {
        spans.push(Span::styled(
            "  ⏵ attached",
            Style::default().fg(Color::Green),
        ));
    }
    if let Some(last) = run.last_event.as_deref().or(run.intent.as_deref()) {
        spans.push(Span::styled(
            format!("  {}", one_line(last, 60)),
            Style::default().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

fn draw_list(f: &mut Frame, s: &SessionsState, area: Rect) {
    let scope = match &s.scope {
        Scope::All => "all runs".to_string(),
        Scope::Thread(t) => format!("this chat · {}", short(t)),
    };
    let waiting = s
        .runs
        .iter()
        .flat_map(|r| r.flatten())
        .filter(|(_, n)| n.state == RunState::NeedsInput)
        .count();
    let mut title = vec![
        Span::raw(" Runs "),
        Span::styled(scope, Style::default().fg(Color::DarkGray)),
    ];
    if waiting > 0 {
        title.push(Span::styled(
            format!("  {waiting} need you "),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Line::from(title));

    if s.rows.is_empty() {
        let msg = if s.loaded {
            "No runs yet.\n\nStart one with `distri run --task …`; runs it forks show up here."
        } else {
            "Loading runs…"
        };
        f.render_widget(
            Paragraph::new(msg)
                .block(block)
                .wrap(Wrap { trim: true })
                .fg(Color::DarkGray),
            area,
        );
        return;
    }

    let attached_id = s.attached.as_ref().map(|a| a.task_id.as_str());
    let mut items: Vec<ListItem> = s
        .rows
        .iter()
        .filter_map(|row| {
            s.find(&row.task_id).map(|run| {
                ListItem::new(row_line(
                    row.depth,
                    run,
                    Some(run.task_id.as_str()) == attached_id,
                ))
            })
        })
        .collect();
    if s.has_more {
        items.push(ListItem::new(Line::from(Span::styled(
            "  … older runs not shown",
            Style::default().fg(Color::DarkGray),
        ))));
    }
    let mut list_state = ListState::default().with_selected(Some(s.selected));
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().bg(Color::Rgb(40, 44, 52)))
        .highlight_symbol("▌");
    f.render_stateful_widget(list, area, &mut list_state);
}

fn draw_details(f: &mut Frame, s: &SessionsState, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" Run ");
    let Some(run) = s.selected_run() else {
        f.render_widget(Paragraph::new("").block(block), area);
        return;
    };
    let (label, color) = badge(run.state);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                run.agent_id.clone().unwrap_or_default(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(label, Style::default().fg(color)),
        ]),
        Line::from(Span::styled(
            format!("task {} · thread {}", run.task_id, short(&run.thread_id)),
            Style::default().fg(Color::DarkGray),
        )),
        Line::raw(""),
    ];
    if let Some(intent) = &run.intent {
        lines.push(Line::from(vec![
            Span::styled("asked  ", Style::default().fg(Color::DarkGray)),
            Span::raw(intent.clone()),
        ]));
    }
    if let Some(last) = &run.last_event {
        lines.push(Line::from(vec![
            Span::styled("latest ", Style::default().fg(Color::DarkGray)),
            Span::raw(last.clone()),
        ]));
    }
    if run.child_count > 0 {
        lines.push(Line::from(Span::styled(
            format!("{} child run(s)", run.child_count),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        if run.needs_input {
            "enter to attach and answer"
        } else {
            "enter to attach (history, then live)"
        },
        Style::default().fg(Color::Green),
    )));
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_attached(f: &mut Frame, s: &SessionsState, a: &Attached, area: Rect) {
    let state = s.attached_state();
    let (label, color) = state.map(badge).unwrap_or(("…", Color::DarkGray));
    let title = Line::from(vec![
        Span::raw(format!(" {} ", a.agent_id)),
        Span::styled(label, Style::default().fg(color)),
        Span::styled(
            if a.streaming { "  live " } else { "  closed " },
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    let block = Block::default().borders(Borders::ALL).title(title);

    let answer_box = a.answerable().is_some() || matches!(s.mode, Mode::Answer { .. });
    let [log_area, ask_area] = if answer_box {
        Layout::vertical([Constraint::Min(3), Constraint::Length(4)]).areas(area)
    } else {
        Layout::vertical([Constraint::Min(3), Constraint::Length(0)]).areas(area)
    };

    let text: Text = a.transcript.bytes().into_text().unwrap_or_default();
    let inner_w = log_area.width.saturating_sub(2).max(1) as usize;
    let inner_h = log_area.height.saturating_sub(2) as usize;
    let total: usize = text
        .lines
        .iter()
        .map(|l| l.width().max(1).div_ceil(inner_w))
        .sum();
    let bottom = total.saturating_sub(inner_h);
    let offset = bottom.saturating_sub(a.scroll as usize) as u16;
    f.render_widget(
        Paragraph::new(text)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((offset, 0)),
        log_area,
    );

    if answer_box {
        draw_answer(f, s, a, ask_area);
    }
}

fn draw_answer(f: &mut Frame, s: &SessionsState, a: &Attached, area: Rect) {
    f.render_widget(Clear, area);
    let (title, body) = match (&s.mode, a.answerable()) {
        (Mode::Answer { text, .. }, Some(p)) => (
            format!(" Answer {} · enter sends · esc cancels ", p.call.tool_name),
            Line::from(vec![
                Span::raw(text.clone()),
                Span::styled("▏", Style::default().fg(Color::Yellow)),
            ]),
        ),
        (_, Some(p)) if p.runnable => (
            format!(" {} is waiting on this machine ", p.call.tool_name),
            Line::from(vec![
                Span::styled(
                    "a",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    " run {} here with your local tools",
                    p.call.tool_name
                )),
            ]),
        ),
        (_, Some(p)) => (
            format!(" {} needs you ", p.call.tool_name),
            Line::from(vec![
                Span::styled(
                    "a",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    " answer · {}",
                    one_line(&p.call.input.to_string(), 120)
                )),
            ]),
        ),
        (_, None) => (" Answer ".to_string(), Line::raw("")),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(title);
    f.render_widget(
        Paragraph::new(body).block(block).wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_footer(f: &mut Frame, s: &SessionsState, area: Rect) {
    let keys = match (&s.mode, s.attached.is_some()) {
        (Mode::Answer { .. }, _) => "type the answer · enter send · esc cancel",
        (Mode::ConfirmCancel { .. }, _) => {
            "y cancel the run and its children · any other key keeps it"
        }
        (_, true) => {
            "↑↓ move · enter switch · a answer · x cancel · pgup/pgdn scroll · esc detach · q quit"
        }
        (_, false) => "↑↓ move · enter attach · x cancel · r refresh · t this chat/all · q quit",
    };
    let mut lines = vec![Line::from(Span::styled(
        keys,
        Style::default().fg(Color::DarkGray),
    ))];
    if let Some(status) = &s.status {
        lines.push(Line::from(Span::raw(status.clone())));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn one_line(s: &str, max: usize) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        format!("{}…", flat.chars().take(max).collect::<String>())
    } else {
        flat
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use distri_types::runs::fixture;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::super::state::{Input, Key, Scope, SessionsState};
    use super::*;

    fn screen(s: &SessionsState) -> String {
        let mut term = Terminal::new(TestBackend::new(140, 24)).unwrap();
        term.draw(|f| draw(f, s)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn draws_list_attached_view_and_answer_box() {
        let mut s = SessionsState::new(Scope::All, Arc::new(|_: &str| false));
        assert!(screen(&s).contains("Loading runs"));
        s.handle(Input::RunsLoaded(fixture::multi_run())).await;
        let list = screen(&s);
        assert!(list.contains("1 need you"), "{list}");
        assert!(list.contains("needs you"));
        assert!(list.contains(fixture::ASKER_AGENT));
        assert!(list.contains("↓ waiting"), "{list}");

        s.handle(Input::Key(Key::Down)).await;
        s.handle(Input::Key(Key::Enter)).await;
        for e in fixture::pending_ask_events() {
            s.handle(Input::Stream {
                task_id: fixture::ASKER_TASK_ID.into(),
                item: Box::new(distri::StreamItem {
                    message: None,
                    agent_event: Some(e),
                }),
            })
            .await;
        }
        let attached = screen(&s);
        assert!(
            attached.contains("I need the release owner to decide."),
            "{attached}"
        );
        assert!(attached.contains("ask_human needs you"), "{attached}");

        s.handle(Input::Key(Key::Char('a'))).await;
        s.handle(Input::Key(Key::Char('y'))).await;
        let answering = screen(&s);
        assert!(answering.contains("Answer ask_human"), "{answering}");
    }

    #[tokio::test]
    async fn draws_the_empty_state() {
        let mut s = SessionsState::new(Scope::All, Arc::new(|_: &str| false));
        s.handle(Input::RunsLoaded(Default::default())).await;
        assert!(screen(&s).contains("No runs yet."));
    }
}
