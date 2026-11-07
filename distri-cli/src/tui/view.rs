//! The live region: what is still happening (streaming text, running tools),
//! the spinner, queued messages, the prompt or the input box, completion and
//! the footer. Returns rows and where the cursor goes.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use distri_cli::interact::Interaction;

use super::app::{App, Prompt};
use super::cells::{fmt_secs, render_cell, RenderOpts};
use super::wrap::{display_width, prefixed, wrap_lines};

const DIM: Style = Style::new().fg(Color::DarkGray);

const VERBS: &[&str] = &[
    "Thinking",
    "Working",
    "Reading",
    "Planning",
    "Composing",
    "Checking",
    "Weaving",
    "Cooking",
];

pub fn draw(app: &mut App) -> (Vec<Line<'static>>, Option<(u16, u16)>) {
    let w = app.width;
    let max = app.height.saturating_sub(1) as usize;
    let footer = footer(app);

    if app.overlay.is_some() {
        let rows = max.saturating_sub(2).clamp(6, 30);
        let mut out = app
            .overlay
            .as_mut()
            .map(|o| o.render(w, rows))
            .unwrap_or_default();
        out.extend(status_rows(app));
        out.extend(footer);
        return (out, None);
    }

    let mut bottom: Vec<Line<'static>> = Vec::new();
    let cursor_in_bottom: Option<(u16, u16)>;

    // Spinner.
    let focused_live = match &app.focus {
        None => app.turn.is_some(),
        Some(t) => app
            .followed
            .get(t)
            .is_some_and(|f| f.live && f.transcript.finished.is_none()),
    };
    if focused_live {
        bottom.push(spinner(app));
    }
    // Queued messages.
    if !app.queue.is_empty() {
        bottom.push(Line::styled("  queued · sent when this turn ends", DIM));
        for q in &app.queue {
            let one: String = q
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(w as usize - 6)
                .collect();
            bottom.push(Line::from(vec![Span::styled("  › ", DIM), Span::raw(one)]));
        }
    }

    if let Some(prompt) = app.prompts.front() {
        let (rows, cursor) = prompt_box(app, prompt);
        let base = bottom.len() as u16;
        cursor_in_bottom = cursor.map(|(r, c)| (base + r, c));
        bottom.extend(rows);
    } else if let Some(q) = &app.editor.search {
        let found = app.editor.search_match().unwrap_or("").to_string();
        bottom.push(Line::from(vec![
            Span::styled("(reverse-i-search) ", Style::new().fg(Color::Yellow)),
            Span::raw(format!("`{q}`: ")),
            Span::styled(found, DIM),
        ]));
        let x = (display_width("(reverse-i-search) `") + display_width(q)) as u16;
        cursor_in_bottom = Some((bottom.len() as u16 - 1, x));
    } else {
        let border = Style::new().fg(if app.working() {
            Color::DarkGray
        } else {
            Color::Gray
        });
        let placeholder = if app.working() {
            "type to queue a message · esc to interrupt"
        } else if app.focus.is_some() {
            "message the chat · esc returns to it"
        } else {
            "ask anything · / for commands · @ for files"
        };
        let (rows, (cr, cc)) = app.editor.render(w.saturating_sub(3), placeholder);
        let base = bottom.len() as u16;
        bottom.push(Line::styled(
            format!("╭{}╮", "─".repeat(w as usize - 2)),
            border,
        ));
        for row in rows {
            let used: usize = row.spans.iter().map(|s| display_width(&s.content)).sum();
            let mut spans = vec![Span::styled("│ ", border)];
            spans.extend(row.spans);
            spans.push(Span::raw(" ".repeat((w as usize).saturating_sub(used + 3))));
            spans.push(Span::styled("│", border));
            bottom.push(Line::from(spans));
        }
        bottom.push(Line::styled(
            format!("╰{}╯", "─".repeat(w as usize - 2)),
            border,
        ));
        cursor_in_bottom = Some((base + 1 + cr, cc + 2));
        if let Some(p) = &app.popup {
            for (i, (item, desc)) in p.items.iter().enumerate() {
                let sel = i == p.selected;
                let mut line = Line::from(vec![
                    Span::styled(
                        if sel { "  ❯ " } else { "    " },
                        Style::new().fg(Color::Cyan),
                    ),
                    Span::styled(
                        format!("{item:<20}"),
                        if sel {
                            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                        } else {
                            Style::new()
                        },
                    ),
                    Span::styled(desc.clone(), DIM),
                ]);
                if sel {
                    line.style = Style::new();
                }
                bottom.push(line);
            }
        }
    }
    bottom.extend(status_rows(app));
    bottom.extend(footer);

    // Live cells take what is left, newest at the bottom.
    let room = max.saturating_sub(bottom.len());
    let opts = RenderOpts {
        width: w,
        expand: app.expand_next,
        frame: app.frame,
    };
    let mut cells: Vec<Line<'static>> = app
        .focused()
        .live()
        .iter()
        .flat_map(|c| render_cell(c, &opts))
        .collect();
    // Trailing blank rows waste the live region.
    while cells
        .last()
        .is_some_and(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
    {
        cells.pop();
    }
    if cells.len() > room {
        let hidden = cells.len() - room + 1;
        cells.drain(..hidden);
        cells.insert(0, Line::styled(format!("  ↑ {hidden} more lines"), DIM));
    }
    if !cells.is_empty() {
        cells.push(Line::default());
    }
    let base = cells.len() as u16;
    let mut out = cells;
    out.extend(bottom);
    (out, cursor_in_bottom.map(|(r, c)| (base + r, c)))
}

fn spinner(app: &App) -> Line<'static> {
    const FRAMES: &[&str] = &["✻", "✽", "✶", "✳", "✢", "·", "✢", "✳", "✶", "✽"];
    let f = FRAMES[(app.frame / 2) % FRAMES.len()];
    let (verb, secs) = match (&app.focus, &app.turn) {
        (None, Some(t)) => {
            let v = if t.interrupting {
                "Interrupting"
            } else {
                VERBS[(t.id as usize) % VERBS.len()]
            };
            (v, t.started.elapsed().as_secs())
        }
        _ => ("Following", 0),
    };
    let tokens = app.chat.usage_out;
    let mut tail = format!("({}", fmt_secs(secs));
    if tokens > 0 {
        tail.push_str(&format!(" · ↓ {tokens} tokens"));
    }
    tail.push_str(" · esc to interrupt)");
    Line::from(vec![
        Span::styled(format!("{f} "), Style::new().fg(Color::Magenta)),
        Span::styled(format!("{verb}… "), Style::new().fg(Color::Magenta)),
        Span::styled(tail, DIM),
    ])
}

fn status_rows(app: &App) -> Vec<Line<'static>> {
    match &app.status {
        Some((s, _)) => vec![Line::styled(
            format!("  {s}"),
            Style::new().fg(Color::Yellow),
        )],
        None => vec![],
    }
}

fn footer(app: &App) -> Vec<Line<'static>> {
    let mut spans = vec![
        Span::styled("  ⏵⏵ ", Style::new().fg(mode_color(&app.mode_label))),
        Span::styled(
            app.mode_label.clone(),
            Style::new().fg(mode_color(&app.mode_label)),
        ),
        Span::styled(format!(" · {}", app.agent), DIM),
    ];
    if let Some(m) = &app.model {
        spans.push(Span::styled(format!(" · {m}"), DIM));
    }
    if !app.status_line.is_empty() {
        spans.push(Span::styled(format!(" · {}", app.status_line), DIM));
    }
    let (working, waiting) = app.run_counts();
    if working > 0 {
        spans.push(Span::styled(
            format!(" · ◐ {working} running"),
            Style::new().fg(Color::Cyan),
        ));
    }
    if waiting > 0 {
        spans.push(Span::styled(
            format!(
                " · ● {waiting} need{} you",
                if waiting == 1 { "s" } else { "" }
            ),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    }
    if app.runs_supported != Some(false) {
        spans.push(Span::styled(" · ctrl+t runs", DIM));
    }
    let line = Line::from(spans);
    // The footer never wraps: cut it to the width.
    let rows = wrap_lines(&[line], app.width);
    rows.into_iter().take(1).collect()
}

fn mode_color(label: &str) -> Color {
    match label {
        "plan" => Color::Cyan,
        "accept-edits" | "accept edits" => Color::Magenta,
        "bypass" => Color::LightRed,
        _ => Color::DarkGray,
    }
}

fn prompt_box(app: &App, prompt: &Prompt) -> (Vec<Line<'static>>, Option<(u16, u16)>) {
    let w = app.width as usize;
    let yellow = Style::new().fg(Color::Yellow);
    let (title, body): (String, Vec<Line<'static>>) = match prompt {
        Prompt::Broker { interaction, .. } => match interaction {
            Interaction::Approve { calls, reason } => {
                let mut b: Vec<Line> = calls
                    .iter()
                    .map(|c| {
                        Line::raw(distri_formatter::state::format_tool_call(
                            &c.tool_name,
                            &c.input,
                        ))
                    })
                    .collect();
                if let Some(r) = reason {
                    b.push(Line::styled(r.clone(), DIM));
                }
                ("Approve these calls?".into(), b)
            }
            Interaction::Grant {
                tool,
                script,
                request,
            } => (
                "Script tool wants more access".into(),
                vec![
                    Line::from(vec![
                        Span::styled(tool.clone(), Style::new().add_modifier(Modifier::BOLD)),
                        Span::styled(format!("  {}", script.display()), DIM),
                    ]),
                    Line::styled(request.clone(), Style::new().fg(Color::Cyan)),
                    Line::styled("(declared in the script's frontmatter)", DIM),
                ],
            ),
            Interaction::Permit { tool, summary, .. } => (
                format!("{tool} needs permission"),
                summary
                    .lines()
                    .take(8)
                    .map(|l| Line::raw(l.to_string()))
                    .collect(),
            ),
            Interaction::Answer {
                question, agent_id, ..
            } => (
                format!("{agent_id} asks"),
                question.lines().map(|l| Line::raw(l.to_string())).collect(),
            ),
        },
        Prompt::Answer {
            agent_id, question, ..
        } => (
            format!("{agent_id} asks"),
            question.lines().map(|l| Line::raw(l.to_string())).collect(),
        ),
    };
    let more = app.prompts.len().saturating_sub(1);
    let mut head = format!("╭─ {title} ");
    if more > 0 {
        head.push_str(&format!("(+{more} more) "));
    }
    let fill = w.saturating_sub(display_width(&head) + 1);
    let mut rows = vec![Line::styled(format!("{head}{}╮", "─".repeat(fill)), yellow)];
    for l in prefixed(&body, (w - 2) as u16, Span::raw(" "), Span::raw(" ")) {
        rows.push(boxed(l, w, yellow));
    }
    let mut cursor = None;
    let options = prompt.options();
    if let Some(editor) = &app.prompt_text {
        let label = if prompt.is_text() {
            ""
        } else {
            "Tell the agent what to do instead (enter sends, empty = plain no):"
        };
        if !label.is_empty() {
            rows.push(boxed(Line::styled(format!(" {label}"), DIM), w, yellow));
        }
        let (erows, (cr, cc)) = editor.render((w - 4) as u16, "");
        let base = rows.len() as u16;
        for r in erows {
            let mut spans = vec![Span::raw(" ")];
            spans.extend(r.spans);
            rows.push(boxed(Line::from(spans), w, yellow));
        }
        cursor = Some((base + cr, cc + 2));
    } else if options.is_empty() {
        let base = rows.len() as u16;
        rows.push(boxed(
            Line::from(vec![
                Span::raw(" "),
                Span::styled("> ", Style::new().add_modifier(Modifier::BOLD)),
            ]),
            w,
            yellow,
        ));
        cursor = Some((base, 3));
    } else {
        rows.push(boxed(Line::default(), w, yellow));
        for (i, o) in options.iter().enumerate() {
            let sel = i == app.prompt_sel;
            rows.push(boxed(
                Line::from(vec![
                    Span::styled(
                        if sel { " ❯ " } else { "   " },
                        Style::new().fg(Color::Cyan),
                    ),
                    Span::styled(
                        format!("{}. {o}", i + 1),
                        if sel {
                            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                        } else {
                            DIM
                        },
                    ),
                ]),
                w,
                yellow,
            ));
        }
    }
    let hint = if prompt.is_text() {
        "─ ⏎ send · ⇧⏎ newline · esc later "
    } else if app.prompt_text.is_some() {
        "─ ⏎ send · esc back "
    } else {
        "─ ↑↓ choose · ⏎ confirm · esc no "
    };
    let fill = w.saturating_sub(display_width(hint) + 2);
    rows.push(Line::styled(
        format!("╰{hint}{}╯", "─".repeat(fill)),
        yellow,
    ));
    (rows, cursor)
}

fn boxed(line: Line<'static>, w: usize, border: Style) -> Line<'static> {
    let used: usize = line.spans.iter().map(|s| display_width(&s.content)).sum();
    let mut spans = vec![Span::styled("│", border)];
    let content_w = w.saturating_sub(2);
    if used > content_w {
        // Cut: the prompt box never wraps a row it already wrapped.
        let mut acc = 0;
        for s in line.spans {
            let sw = display_width(&s.content);
            if acc + sw <= content_w {
                acc += sw;
                spans.push(s);
            } else {
                break;
            }
        }
        spans.push(Span::raw(" ".repeat(content_w.saturating_sub(acc))));
    } else {
        spans.extend(line.spans);
        spans.push(Span::raw(" ".repeat(content_w - used)));
    }
    spans.push(Span::styled("│", border));
    Line::from(spans)
}
