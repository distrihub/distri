//! An inline terminal: finished output goes into the terminal's own
//! scrollback, and only a small live region at the bottom is redrawn.
//!
//! ratatui's `Viewport::Inline` has a fixed height; the live region here
//! grows and shrinks with what it shows (a streaming paragraph, a prompt,
//! the run overlay), so this writes rows itself with crossterm. Rows are
//! ratatui [`Line`]s already wrapped to the width (see [`super::wrap`]).
//!
//! Invariants: `top` is the screen row where the live region starts; the
//! live region is the last thing on screen. History is printed at `top`
//! (overwriting the live region), each row followed by `\r\n`, so rows that
//! scroll off the top of the screen land in native scrollback.

use std::io::{self, Write};

use ratatui::crossterm::{
    cursor::{Hide, MoveTo, Show},
    queue,
    style::{
        Attribute, Color as CColor, Print, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal::{BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate},
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;

pub struct InlineTerm<W: Write> {
    out: W,
    width: u16,
    height: u16,
    /// First screen row of the live region.
    top: u16,
    /// Rows the live region used last time it was drawn.
    live_rows: u16,
}

impl<W: Write> InlineTerm<W> {
    /// `cursor_row`: where the cursor is now; the live region starts there.
    pub fn new(out: W, (width, height): (u16, u16), cursor_row: u16) -> Self {
        Self {
            out,
            width: width.max(10),
            height: height.max(3),
            top: cursor_row.min(height.saturating_sub(1)),
            live_rows: 0,
        }
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    #[cfg(test)]
    pub fn top(&self) -> u16 {
        self.top
    }

    #[cfg(test)]
    pub fn writer(&mut self) -> &mut W {
        &mut self.out
    }

    pub fn resize(&mut self, width: u16, height: u16) -> io::Result<()> {
        // A narrower terminal re-wraps what is on screen; start the live
        // region on a clean screen bottom rather than guess where it went.
        if width < self.width {
            queue!(
                self.out,
                MoveTo(0, self.top),
                Clear(ClearType::FromCursorDown)
            )?;
        }
        self.width = width.max(10);
        self.height = height.max(3);
        self.top = self.top.min(self.height.saturating_sub(1));
        Ok(())
    }

    /// Print `rows` above the live region, into the scrollback. The live
    /// region is erased; draw it again after.
    pub fn insert_history(&mut self, rows: &[Line<'_>]) -> io::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        queue!(
            self.out,
            BeginSynchronizedUpdate,
            MoveTo(0, self.top),
            Clear(ClearType::FromCursorDown)
        )?;
        let mut row = self.top;
        for line in rows {
            write_line(&mut self.out, line)?;
            queue!(self.out, Print("\r\n"))?;
            if row + 1 < self.height {
                row += 1;
            }
        }
        self.top = row;
        self.live_rows = 0;
        queue!(self.out, EndSynchronizedUpdate)?;
        self.out.flush()
    }

    /// Draw the live region. `cursor`: (row, column) inside it to show the
    /// cursor at, or `None` to hide it.
    pub fn draw_live(&mut self, rows: &[Line<'_>], cursor: Option<(u16, u16)>) -> io::Result<()> {
        let rows = &rows[rows.len().saturating_sub(self.height as usize)..];
        let h = rows.len() as u16;
        queue!(self.out, BeginSynchronizedUpdate, Hide)?;
        if self.top + h > self.height {
            // Make room: scroll the screen up from its last row. What
            // scrolls off goes into the scrollback like any output.
            let scroll = self.top + h - self.height;
            queue!(self.out, MoveTo(0, self.height - 1))?;
            for _ in 0..scroll {
                queue!(self.out, Print("\n"))?;
            }
            self.top -= scroll;
        }
        for (i, line) in rows.iter().enumerate() {
            queue!(self.out, MoveTo(0, self.top + i as u16))?;
            write_line(&mut self.out, line)?;
            queue!(self.out, Clear(ClearType::UntilNewLine))?;
        }
        if h < self.live_rows || h == 0 {
            queue!(
                self.out,
                MoveTo(0, self.top + h),
                Clear(ClearType::FromCursorDown)
            )?;
        }
        self.live_rows = h;
        match cursor {
            Some((r, c)) if r < h => {
                queue!(self.out, MoveTo(c.min(self.width - 1), self.top + r), Show)?
            }
            _ => queue!(self.out, MoveTo(0, self.top + h.min(self.height - 1)))?,
        }
        queue!(self.out, EndSynchronizedUpdate)?;
        self.out.flush()
    }

    /// Erase the live region and leave the cursor where it started, so plain
    /// output (a legacy command, the process exiting) continues from there.
    pub fn clear_live(&mut self) -> io::Result<()> {
        queue!(
            self.out,
            MoveTo(0, self.top),
            Clear(ClearType::FromCursorDown),
            Show
        )?;
        self.live_rows = 0;
        self.out.flush()
    }

    /// After plain output ran from `top`: the live region starts at `row`.
    pub fn set_top(&mut self, row: u16) {
        self.top = row.min(self.height.saturating_sub(1));
        self.live_rows = 0;
    }
}

fn write_line<W: Write>(out: &mut W, line: &Line<'_>) -> io::Result<()> {
    for span in &line.spans {
        let style = line.style.patch(span.style);
        apply_style(out, style)?;
        queue!(out, Print(span.content.as_ref()))?;
        queue!(out, SetAttribute(Attribute::Reset))?;
    }
    Ok(())
}

fn apply_style<W: Write>(out: &mut W, style: Style) -> io::Result<()> {
    if let Some(fg) = style.fg {
        queue!(out, SetForegroundColor(to_crossterm(fg)))?;
    }
    if let Some(bg) = style.bg {
        queue!(out, SetBackgroundColor(to_crossterm(bg)))?;
    }
    let m = style.add_modifier;
    if m.contains(Modifier::BOLD) {
        queue!(out, SetAttribute(Attribute::Bold))?;
    }
    if m.contains(Modifier::DIM) {
        queue!(out, SetAttribute(Attribute::Dim))?;
    }
    if m.contains(Modifier::ITALIC) {
        queue!(out, SetAttribute(Attribute::Italic))?;
    }
    if m.contains(Modifier::UNDERLINED) {
        queue!(out, SetAttribute(Attribute::Underlined))?;
    }
    if m.contains(Modifier::REVERSED) {
        queue!(out, SetAttribute(Attribute::Reverse))?;
    }
    if m.contains(Modifier::CROSSED_OUT) {
        queue!(out, SetAttribute(Attribute::CrossedOut))?;
    }
    Ok(())
}

fn to_crossterm(c: Color) -> CColor {
    match c {
        Color::Reset => CColor::Reset,
        Color::Black => CColor::Black,
        Color::Red => CColor::DarkRed,
        Color::Green => CColor::DarkGreen,
        Color::Yellow => CColor::DarkYellow,
        Color::Blue => CColor::DarkBlue,
        Color::Magenta => CColor::DarkMagenta,
        Color::Cyan => CColor::DarkCyan,
        Color::Gray => CColor::Grey,
        Color::DarkGray => CColor::DarkGrey,
        Color::LightRed => CColor::Red,
        Color::LightGreen => CColor::Green,
        Color::LightYellow => CColor::Yellow,
        Color::LightBlue => CColor::Blue,
        Color::LightMagenta => CColor::Magenta,
        Color::LightCyan => CColor::Cyan,
        Color::White => CColor::White,
        Color::Rgb(r, g, b) => CColor::Rgb { r, g, b },
        Color::Indexed(i) => CColor::AnsiValue(i),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &mut InlineTerm<Vec<u8>>) -> String {
        String::from_utf8_lossy(t.writer()).into_owned()
    }

    #[test]
    fn history_rows_advance_top_until_the_screen_bottom() {
        let mut t = InlineTerm::new(Vec::new(), (40, 10), 2);
        t.insert_history(&[Line::raw("a"), Line::raw("b")]).unwrap();
        assert_eq!(t.top(), 4);
        let rows: Vec<Line> = (0..20).map(|i| Line::raw(format!("r{i}"))).collect();
        t.insert_history(&rows).unwrap();
        assert_eq!(
            t.top(),
            9,
            "top sticks to the last row once the screen scrolls"
        );
        assert!(text(&mut t).contains("r19\x1b[0m\r\n"));
    }

    #[test]
    fn a_tall_live_region_scrolls_the_screen_up() {
        let mut t = InlineTerm::new(Vec::new(), (40, 10), 8);
        let rows: Vec<Line> = (0..4).map(|i| Line::raw(format!("live{i}"))).collect();
        t.draw_live(&rows, Some((3, 2))).unwrap();
        assert_eq!(t.top(), 6);
        let out = text(&mut t);
        assert!(out.contains("live3"));
        // Cursor shown on the last live row, column 2 (1-based: row 10, col 3).
        assert!(out.contains("\x1b[10;3H"));
    }

    #[test]
    fn a_shrinking_live_region_clears_what_it_left() {
        let mut t = InlineTerm::new(Vec::new(), (40, 10), 0);
        t.draw_live(&[Line::raw("1"), Line::raw("2"), Line::raw("3")], None)
            .unwrap();
        t.writer().clear();
        t.draw_live(&[Line::raw("1")], None).unwrap();
        // Clear from row 2 (1-based) down.
        assert!(text(&mut t).contains("\x1b[2;1H\x1b[J"));
    }
}
