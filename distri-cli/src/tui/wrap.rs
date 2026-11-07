//! Wrap styled lines to a width, keeping span styles. Breaks after the last
//! space that fits, else mid-word. Width is measured in terminal cells.

use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// Wrap every line to at most `width` cells. An empty line stays one row.
pub fn wrap_lines(lines: &[Line<'_>], width: u16) -> Vec<Line<'static>> {
    let width = width.max(4) as usize;
    let mut rows = Vec::new();
    for line in lines {
        wrap_one(line, width, &mut rows);
    }
    rows
}

/// Cells `s` takes on screen.
pub fn display_width(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

fn wrap_one(line: &Line<'_>, width: usize, rows: &mut Vec<Line<'static>>) {
    // Flatten to (char, style) so breaks can fall inside a span.
    let mut cells: Vec<(char, ratatui::style::Style)> = Vec::new();
    for span in &line.spans {
        let style = line.style.patch(span.style);
        for c in span.content.chars() {
            if c == '\t' {
                for _ in 0..4 {
                    cells.push((' ', style));
                }
            } else if c != '\r' && c != '\n' {
                cells.push((c, style));
            }
        }
    }
    if cells.is_empty() {
        rows.push(Line::default());
        return;
    }
    let mut start = 0;
    while start < cells.len() {
        let mut w = 0;
        let mut end = start;
        let mut last_space = None;
        while end < cells.len() {
            let cw = cells[end].0.width().unwrap_or(0);
            if w + cw > width {
                break;
            }
            if cells[end].0 == ' ' {
                last_space = Some(end);
            }
            w += cw;
            end += 1;
        }
        let (cut, next) = if end >= cells.len() {
            (end, end)
        } else if cells[end].0 == ' ' {
            (end, end + 1)
        } else if let Some(sp) = last_space.filter(|sp| *sp > start) {
            (sp, sp + 1)
        } else {
            (end.max(start + 1), end.max(start + 1))
        };
        rows.push(to_line(&cells[start..cut]));
        start = next;
    }
}

fn to_line(cells: &[(char, ratatui::style::Style)]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut cur = String::new();
    let mut style = None;
    for (c, s) in cells {
        if Some(*s) != style {
            if let Some(st) = style {
                spans.push(Span::styled(std::mem::take(&mut cur), st));
            }
            style = Some(*s);
        }
        cur.push(*c);
    }
    if let Some(st) = style {
        spans.push(Span::styled(cur, st));
    }
    Line::from(spans)
}

/// Prefix every row of `rows` with `first` on the first row and `rest` on the
/// others (hanging indent), wrapping the content to `width` minus the prefix.
pub fn prefixed(
    lines: &[Line<'_>],
    width: u16,
    first: Span<'static>,
    rest: Span<'static>,
) -> Vec<Line<'static>> {
    let pw = display_width(&first.content).max(display_width(&rest.content)) as u16;
    let inner = wrap_lines(lines, width.saturating_sub(pw).max(4));
    inner
        .into_iter()
        .enumerate()
        .map(|(i, mut l)| {
            l.spans
                .insert(0, if i == 0 { first.clone() } else { rest.clone() });
            l
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Style, Stylize};

    fn plain(rows: &[Line]) -> Vec<String> {
        rows.iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn wraps_at_spaces_and_keeps_styles() {
        let line = Line::from(vec![
            Span::raw("hello "),
            "brave new".bold(),
            Span::raw(" world"),
        ]);
        let rows = wrap_lines(&[line], 11);
        assert_eq!(plain(&rows), vec!["hello brave", "new world"]);
        assert!(rows[0].spans.iter().any(|s| s.style == Style::new().bold()));
    }

    #[test]
    fn long_words_break_and_wide_chars_count_double() {
        assert_eq!(
            plain(&wrap_lines(&[Line::raw("abcdefghij")], 4)),
            vec!["abcd", "efgh", "ij"]
        );
        assert_eq!(
            plain(&wrap_lines(&[Line::raw("日本語です")], 4)),
            vec!["日本", "語で", "す"]
        );
        assert_eq!(wrap_lines(&[Line::raw("")], 10).len(), 1);
    }

    #[test]
    fn hanging_indent() {
        let rows = prefixed(
            &[Line::raw("one two three")],
            9,
            Span::raw("⏺ "),
            Span::raw("  "),
        );
        assert_eq!(plain(&rows), vec!["⏺ one two", "  three"]);
    }
}
