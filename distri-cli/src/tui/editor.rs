//! The input editor: multi-line text with a cursor, bracketed paste folded to
//! `[Pasted text #n +L lines]`, history (↑/↓ on the first/last row, Ctrl+R
//! search), and the word/line editing keys of a readline.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// A paste this long (or with a newline) is folded to a placeholder.
const FOLD_PASTE_CHARS: usize = 400;

#[derive(Debug, Clone, Default)]
pub struct Editor {
    lines: Vec<Vec<char>>,
    row: usize,
    col: usize,
    history: Vec<String>,
    hist_pos: Option<usize>,
    draft: Option<String>,
    pastes: Vec<(String, String)>,
    pub search: Option<String>,
}

impl Editor {
    pub fn new(history: Vec<String>) -> Self {
        Self {
            lines: vec![Vec::new()],
            history,
            ..Default::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The text with folded pastes expanded.
    pub fn expanded_text(&self) -> String {
        let mut t = self.text();
        for (placeholder, content) in &self.pastes {
            t = t.replace(placeholder, content);
        }
        t
    }

    pub fn set_text(&mut self, text: &str) {
        self.lines = text.split('\n').map(|l| l.chars().collect()).collect();
        if self.lines.is_empty() {
            self.lines.push(Vec::new());
        }
        self.row = self.lines.len() - 1;
        self.col = self.lines[self.row].len();
    }

    pub fn clear(&mut self) {
        self.lines = vec![Vec::new()];
        self.row = 0;
        self.col = 0;
        self.pastes.clear();
        self.hist_pos = None;
        self.draft = None;
        self.search = None;
    }

    /// Take the text (pastes expanded) and remember it in history.
    pub fn submit(&mut self) -> String {
        let text = self.expanded_text();
        let shown = self.text();
        if !shown.trim().is_empty() && self.history.last() != Some(&shown) {
            self.history.push(shown);
        }
        self.clear();
        text
    }

    pub fn history(&self) -> &[String] {
        &self.history
    }

    pub fn insert_char(&mut self, c: char) {
        if c == '\n' {
            self.newline();
            return;
        }
        self.lines[self.row].insert(self.col, c);
        self.col += 1;
    }

    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            if c != '\r' {
                self.insert_char(c);
            }
        }
    }

    pub fn paste(&mut self, s: &str) {
        let s = s.replace("\r\n", "\n").replace('\r', "\n");
        let lines = s.lines().count();
        if lines > 1 || s.chars().count() > FOLD_PASTE_CHARS {
            let n = self.pastes.len() + 1;
            let placeholder = if lines > 1 {
                format!("[Pasted text #{n} +{lines} lines]")
            } else {
                format!("[Pasted text #{n} {} chars]", s.chars().count())
            };
            self.insert_str(&placeholder);
            self.pastes.push((placeholder, s));
        } else {
            self.insert_str(&s);
        }
    }

    pub fn newline(&mut self) {
        let rest = self.lines[self.row].split_off(self.col);
        self.lines.insert(self.row + 1, rest);
        self.row += 1;
        self.col = 0;
    }

    pub fn backspace(&mut self) {
        if self.col > 0 {
            // A folded paste goes as a whole.
            let before: String = self.lines[self.row][..self.col].iter().collect();
            if let Some((ph, _)) = self
                .pastes
                .iter()
                .find(|(ph, _)| before.ends_with(ph.as_str()))
            {
                let n = ph.chars().count();
                let ph = ph.clone();
                self.lines[self.row].drain(self.col - n..self.col);
                self.col -= n;
                self.pastes.retain(|(p, _)| *p != ph);
                return;
            }
            self.col -= 1;
            self.lines[self.row].remove(self.col);
        } else if self.row > 0 {
            let line = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.lines[self.row].len();
            self.lines[self.row].extend(line);
        }
    }

    pub fn delete(&mut self) {
        if self.col < self.lines[self.row].len() {
            self.lines[self.row].remove(self.col);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].extend(next);
        }
    }

    pub fn left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.lines[self.row].len();
        }
    }

    pub fn right(&mut self) {
        if self.col < self.lines[self.row].len() {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    pub fn home(&mut self) {
        self.col = 0;
    }

    pub fn end(&mut self) {
        self.col = self.lines[self.row].len();
    }

    pub fn word_left(&mut self) {
        let l = &self.lines[self.row];
        while self.col > 0 && l[self.col - 1].is_whitespace() {
            self.col -= 1;
        }
        while self.col > 0 && !l[self.col - 1].is_whitespace() {
            self.col -= 1;
        }
    }

    pub fn word_right(&mut self) {
        let l = &self.lines[self.row];
        while self.col < l.len() && l[self.col].is_whitespace() {
            self.col += 1;
        }
        while self.col < l.len() && !l[self.col].is_whitespace() {
            self.col += 1;
        }
    }

    /// Ctrl+W: delete the word before the cursor.
    pub fn delete_word(&mut self) {
        let end = self.col;
        self.word_left();
        self.lines[self.row].drain(self.col..end);
    }

    /// Ctrl+U: delete to the start of the line.
    pub fn kill_to_start(&mut self) {
        self.lines[self.row].drain(..self.col);
        self.col = 0;
    }

    /// Ctrl+K: delete to the end of the line.
    pub fn kill_to_end(&mut self) {
        self.lines[self.row].truncate(self.col);
    }

    /// ↑: move up a row, or recall older history from the first row.
    pub fn up(&mut self) {
        if self.row > 0 {
            self.row -= 1;
            self.col = self.col.min(self.lines[self.row].len());
            return;
        }
        if self.history.is_empty() {
            return;
        }
        let pos = match self.hist_pos {
            None => {
                self.draft = Some(self.text());
                self.history.len() - 1
            }
            Some(0) => return,
            Some(p) => p - 1,
        };
        self.hist_pos = Some(pos);
        let entry = self.history[pos].clone();
        self.set_text(&entry);
        self.row = 0;
        self.col = self.col.min(self.lines[0].len());
    }

    /// ↓: move down a row, or recall newer history from the last row.
    pub fn down(&mut self) {
        if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = self.col.min(self.lines[self.row].len());
            return;
        }
        let Some(pos) = self.hist_pos else { return };
        if pos + 1 < self.history.len() {
            self.hist_pos = Some(pos + 1);
            let entry = self.history[pos + 1].clone();
            self.set_text(&entry);
        } else {
            self.hist_pos = None;
            let draft = self.draft.take().unwrap_or_default();
            self.set_text(&draft);
        }
    }

    /// Ctrl+R: the newest history entry containing `query`.
    pub fn search_match(&self) -> Option<&str> {
        let q = self.search.as_deref()?;
        if q.is_empty() {
            return None;
        }
        self.history
            .iter()
            .rev()
            .find(|h| h.contains(q))
            .map(String::as_str)
    }

    pub fn accept_search(&mut self) {
        if let Some(m) = self.search_match().map(str::to_string) {
            self.set_text(&m);
        }
        self.search = None;
    }

    /// The word under the cursor (for `@file` and `/command` completion):
    /// its start column and text.
    pub fn current_token(&self) -> (usize, String) {
        let l = &self.lines[self.row];
        let mut start = self.col;
        while start > 0 && !l[start - 1].is_whitespace() {
            start -= 1;
        }
        (start, l[start..self.col].iter().collect())
    }

    /// Replace the current token with `with`.
    pub fn replace_token(&mut self, with: &str) {
        let (start, _) = self.current_token();
        self.lines[self.row].drain(start..self.col);
        self.col = start;
        self.insert_str(with);
    }

    pub fn is_single_line(&self) -> bool {
        self.lines.len() == 1
    }

    /// Rows to draw, wrapped to `width` after a 2-cell gutter (`> ` on the
    /// first row), and the cursor's (row, column) within them.
    pub fn render(&self, width: u16, placeholder: &str) -> (Vec<Line<'static>>, (u16, u16)) {
        let gutter = 2usize;
        let inner = (width as usize).saturating_sub(gutter + 1).max(4);
        let mut rows: Vec<Line<'static>> = Vec::new();
        let mut cursor = (0u16, gutter as u16);
        if self.is_empty() {
            rows.push(Line::from(vec![
                Span::styled("> ", Style::new().add_modifier(Modifier::BOLD)),
                Span::styled(placeholder.to_string(), Style::new().fg(Color::DarkGray)),
            ]));
            return (rows, cursor);
        }
        for (li, line) in self.lines.iter().enumerate() {
            let mut cur = String::new();
            let mut w = 0usize;
            let mut first_row_of_line = true;
            let push = |rows: &mut Vec<Line<'static>>, s: String, first: bool| {
                let lead = if rows.is_empty() && first { "> " } else { "  " };
                rows.push(Line::from(vec![
                    Span::styled(lead, Style::new().add_modifier(Modifier::BOLD)),
                    Span::raw(s),
                ]));
            };
            for (ci, c) in line.iter().enumerate() {
                let cw = c.width().unwrap_or(0);
                if w + cw > inner {
                    push(&mut rows, std::mem::take(&mut cur), first_row_of_line);
                    first_row_of_line = false;
                    w = 0;
                }
                if li == self.row && ci == self.col {
                    cursor = (rows.len() as u16, (gutter + w) as u16);
                }
                cur.push(*c);
                w += cw;
            }
            if li == self.row && self.col == line.len() {
                if w >= inner {
                    push(&mut rows, std::mem::take(&mut cur), first_row_of_line);
                    first_row_of_line = false;
                    w = 0;
                }
                cursor = (rows.len() as u16, (gutter + w) as u16);
            }
            push(&mut rows, cur, first_row_of_line);
        }
        (rows, cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_editing_and_submit() {
        let mut e = Editor::new(vec![]);
        e.insert_str("hello");
        e.newline();
        e.insert_str("world");
        assert_eq!(e.text(), "hello\nworld");
        e.left();
        e.left();
        e.left();
        e.left();
        e.left();
        e.backspace();
        assert_eq!(e.text(), "helloworld");
        e.delete_word();
        assert_eq!(e.text(), "world");
        assert_eq!(e.submit(), "world");
        assert!(e.is_empty());
        assert_eq!(e.history(), &["world".to_string()]);
    }

    #[test]
    fn pasted_blocks_fold_and_expand_on_submit() {
        let mut e = Editor::new(vec![]);
        e.insert_str("see ");
        e.paste("a\nb\nc");
        assert_eq!(e.text(), "see [Pasted text #1 +3 lines]");
        assert_eq!(e.submit(), "see a\nb\nc");
        e.paste("x\ny");
        e.backspace();
        assert!(e.is_empty(), "backspace removes the whole placeholder");
        e.paste("one line");
        assert_eq!(e.text(), "one line");
    }

    #[test]
    fn history_recall_keeps_the_draft() {
        let mut e = Editor::new(vec!["first".into(), "second".into()]);
        e.insert_str("draft");
        e.up();
        assert_eq!(e.text(), "second");
        e.up();
        assert_eq!(e.text(), "first");
        e.down();
        e.down();
        assert_eq!(e.text(), "draft");
        e.search = Some("fir".into());
        assert_eq!(e.search_match(), Some("first"));
        e.accept_search();
        assert_eq!(e.text(), "first");
    }

    #[test]
    fn render_wraps_and_places_the_cursor() {
        let mut e = Editor::new(vec![]);
        e.insert_str("abcdefghij");
        let (rows, cursor) = e.render(9, "");
        // inner width 9-3=6: "abcdef" / "ghij"
        assert_eq!(rows.len(), 2);
        assert_eq!(cursor, (1, 6));
        let (_, start) = Editor::new(vec![]).render(20, "type");
        assert_eq!(start, (0, 2));
    }

    #[test]
    fn tokens_for_completion() {
        let mut e = Editor::new(vec![]);
        e.insert_str("look at @src/ma");
        assert_eq!(e.current_token(), (8, "@src/ma".to_string()));
        e.replace_token("@src/main.rs ");
        assert_eq!(e.text(), "look at @src/main.rs ");
    }
}
