//! Markdown → styled lines for the terminal: headings, emphasis, inline and
//! fenced code, lists, quotes, rules and tables. Lines are logical (not
//! wrapped); [`super::wrap`] wraps them to the width.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

const CODE: Style = Style::new().fg(Color::LightYellow);
const CODE_BLOCK: Style = Style::new().fg(Color::Gray);
const DIM: Style = Style::new().fg(Color::DarkGray);

pub fn render(md: &str) -> Vec<Line<'static>> {
    let mut r = Renderer::default();
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(md, opts) {
        r.event(event);
    }
    r.flush_line();
    // No trailing blank rows.
    while r
        .lines
        .last()
        .is_some_and(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
    {
        r.lines.pop();
    }
    r.lines
}

#[derive(Default)]
struct Renderer {
    lines: Vec<Line<'static>>,
    cur: Vec<Span<'static>>,
    styles: Vec<Style>,
    /// Stack of list counters: `None` bullet, `Some(n)` ordered.
    lists: Vec<Option<u64>>,
    quote: usize,
    in_code: bool,
    pending_item_prefix: Option<String>,
    link: Option<String>,
    table_row: Vec<String>,
    in_cell: bool,
    cell: String,
}

impl Renderer {
    fn style(&self) -> Style {
        self.styles
            .iter()
            .fold(Style::default(), |a, s| a.patch(*s))
    }

    fn indent(&self) -> String {
        let depth = self.lists.len().saturating_sub(1);
        "  ".repeat(depth)
    }

    fn start_line_prefix(&mut self) {
        if !self.cur.is_empty() {
            return;
        }
        if self.quote > 0 {
            self.cur.push(Span::styled("│ ".repeat(self.quote), DIM));
        }
        if let Some(p) = self.pending_item_prefix.take() {
            self.cur.push(Span::raw(format!("{}{}", self.indent(), p)));
        } else if !self.lists.is_empty() {
            self.cur.push(Span::raw(format!("{}  ", self.indent())));
        }
    }

    fn push_text(&mut self, text: &str, style: Style) {
        if self.in_cell {
            self.cell.push_str(text);
            return;
        }
        for (i, part) in text.split('\n').enumerate() {
            if i > 0 {
                self.flush_line();
            }
            if part.is_empty() {
                continue;
            }
            self.start_line_prefix();
            self.cur.push(Span::styled(part.to_string(), style));
        }
    }

    fn flush_line(&mut self) {
        if !self.cur.is_empty() {
            self.lines.push(Line::from(std::mem::take(&mut self.cur)));
        }
    }

    fn blank(&mut self) {
        self.flush_line();
        if self.lines.last().is_some_and(|l| !l.spans.is_empty()) {
            self.lines.push(Line::default());
        }
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                if self.in_code {
                    for line in t.lines() {
                        self.lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(line.to_string(), CODE_BLOCK),
                        ]));
                    }
                } else {
                    let s = self.style();
                    self.push_text(&t, s);
                }
            }
            Event::Code(t) => {
                let s = self.style().patch(CODE);
                self.push_text(&t, s);
            }
            Event::SoftBreak => {
                let s = self.style();
                self.push_text(" ", s);
            }
            Event::HardBreak => self.flush_line(),
            Event::Rule => {
                self.blank();
                self.lines
                    .push(Line::styled("────────────────────────────────", DIM));
            }
            Event::TaskListMarker(done) => {
                self.push_text(if done { "■ " } else { "□ " }, Style::default());
            }
            Event::Html(t) | Event::InlineHtml(t) => {
                let s = self.style();
                self.push_text(&t, s);
            }
            Event::FootnoteReference(_) | Event::InlineMath(_) | Event::DisplayMath(_) => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.blank();
                let style = match level {
                    HeadingLevel::H1 => Style::new()
                        .fg(Color::LightMagenta)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                    HeadingLevel::H2 => Style::new()
                        .fg(Color::LightMagenta)
                        .add_modifier(Modifier::BOLD),
                    _ => Style::new().add_modifier(Modifier::BOLD),
                };
                self.styles.push(style);
            }
            Tag::BlockQuote(_) => {
                self.flush_line();
                self.quote += 1;
                self.styles
                    .push(Style::new().add_modifier(Modifier::ITALIC));
            }
            Tag::CodeBlock(kind) => {
                self.blank();
                if let CodeBlockKind::Fenced(lang) = kind {
                    if !lang.is_empty() {
                        self.lines.push(Line::styled(format!("  {lang}"), DIM));
                    }
                }
                self.in_code = true;
            }
            Tag::List(start) => {
                self.flush_line();
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush_line();
                let prefix = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let p = format!("{n}. ");
                        *n += 1;
                        p
                    }
                    _ => "• ".to_string(),
                };
                self.pending_item_prefix = Some(prefix);
            }
            Tag::Emphasis => self
                .styles
                .push(Style::new().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.styles.push(Style::new().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self
                .styles
                .push(Style::new().add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { dest_url, .. } => {
                self.link = Some(dest_url.to_string());
                self.styles.push(
                    Style::new()
                        .fg(Color::LightBlue)
                        .add_modifier(Modifier::UNDERLINED),
                );
            }
            Tag::Image { dest_url, .. } => {
                let s = self.style();
                self.push_text(&format!("[image: {dest_url}]"), s);
            }
            Tag::Table(_) => self.blank(),
            Tag::TableHead | Tag::TableRow => self.table_row.clear(),
            Tag::TableCell => {
                self.in_cell = true;
                self.cell.clear();
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush_line();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Heading(_) => {
                self.styles.pop();
                self.flush_line();
            }
            TagEnd::BlockQuote(_) => {
                self.styles.pop();
                self.flush_line();
                self.quote = self.quote.saturating_sub(1);
            }
            TagEnd::CodeBlock => {
                self.in_code = false;
                self.blank();
            }
            TagEnd::List(_) => {
                self.flush_line();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Item => self.flush_line(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link => {
                self.styles.pop();
                if let Some(url) = self.link.take() {
                    self.push_text(&format!(" ({url})"), DIM);
                }
            }
            TagEnd::TableCell => {
                self.in_cell = false;
                self.table_row.push(std::mem::take(&mut self.cell));
            }
            TagEnd::TableHead => {
                let row = self.table_row.join(" │ ");
                self.lines
                    .push(Line::styled(row, Style::new().add_modifier(Modifier::BOLD)));
                self.table_row.clear();
            }
            TagEnd::TableRow => {
                self.lines.push(Line::raw(self.table_row.join(" │ ")));
                self.table_row.clear();
            }
            TagEnd::Table => self.blank(),
            _ => {}
        }
    }
}

/// Where `text` can be cut so the part before is a finished markdown block:
/// the end of the last blank-line paragraph break outside a code fence.
pub fn committable_prefix(text: &str) -> usize {
    let mut in_fence = false;
    let mut last = 0;
    let mut pos = 0;
    let mut prev_blank = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        }
        pos += line.len();
        let complete = line.ends_with('\n');
        let blank = trimmed.is_empty() && complete;
        if blank && !in_fence && !prev_blank && pos > 0 {
            last = pos;
        }
        prev_blank = blank;
    }
    // A cut inside a list or table would split it; only cut where the next
    // block does not continue a list item.
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn renders_blocks() {
        let md = "# Title\n\nSome **bold** and `code`.\n\n- one\n- two\n  1. a\n\n```rust\nfn x() {}\n```\n\n> quoted";
        let out = plain(&render(md));
        assert_eq!(out[0], "Title");
        assert!(out.contains(&"Some bold and code.".to_string()));
        assert!(out.contains(&"• one".to_string()));
        assert!(out.contains(&"  1. a".to_string()));
        assert!(out.contains(&"  fn x() {}".to_string()));
        assert!(out
            .iter()
            .any(|l| l.starts_with("│ ") && l.contains("quoted")));
        let lines = render(md);
        let bold = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content == "bold")
            .unwrap();
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn committable_prefix_stops_outside_fences() {
        assert_eq!(committable_prefix("para one\n\npara two"), 10);
        assert_eq!(
            committable_prefix("```\na\n\nb\n"),
            0,
            "inside an open fence"
        );
        assert_eq!(committable_prefix("no break yet"), 0);
        let t = "a\n\n```\nx\n\ny\n```\n\nz";
        assert_eq!(&t[..committable_prefix(t)], "a\n\n```\nx\n\ny\n```\n\n");
    }

    #[test]
    fn tables() {
        let out = plain(&render("| a | b |\n|---|---|\n| 1 | 2 |"));
        assert!(out.contains(&"a │ b".to_string()));
        assert!(out.contains(&"1 │ 2".to_string()));
    }
}
