//! In-memory output of an `EventPrinter`: the exact bytes `distri run`
//! would print for the attached run, ANSI colours included. The TUI turns
//! them into styled lines with `ansi-to-tui`.

use std::io::Write;
use std::sync::{Arc, Mutex};

/// The printer's spinner redraws its line with `\r` + "erase line".
const ERASE_LINE: &[u8] = b"\r\x1b[2K";

/// Keep the last ~512 KiB so a long run doesn't grow without bound.
const MAX_BYTES: usize = 512 * 1024;

#[derive(Clone, Default)]
pub struct Transcript {
    buf: Arc<Mutex<Vec<u8>>>,
}

impl Transcript {
    /// A `Write` sink appending to this transcript.
    pub fn writer(&self) -> TranscriptWriter {
        TranscriptWriter {
            buf: self.buf.clone(),
        }
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.buf.lock().map(|b| b.clone()).unwrap_or_default()
    }

    /// The transcript without ANSI escape sequences.
    #[cfg(test)]
    pub fn plain_text(&self) -> String {
        strip_ansi(&String::from_utf8_lossy(&self.bytes()))
    }
}

pub struct TranscriptWriter {
    buf: Arc<Mutex<Vec<u8>>>,
}

impl Write for TranscriptWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let mut buf = self
            .buf
            .lock()
            .map_err(|_| std::io::Error::other("transcript lock poisoned"))?;
        buf.extend_from_slice(data);
        apply_line_erases(&mut buf);
        if buf.len() > MAX_BYTES {
            let cut = buf.len() - MAX_BYTES;
            // Cut at a line boundary so no escape sequence is split.
            let cut = buf[cut..]
                .iter()
                .position(|b| *b == b'\n')
                .map(|p| cut + p + 1)
                .unwrap_or(cut);
            buf.drain(..cut);
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Do in the buffer what a terminal does with `\r\x1b[2K`: drop the current
/// line's contents.
fn apply_line_erases(buf: &mut Vec<u8>) {
    while let Some(pos) = find(buf, ERASE_LINE) {
        let line_start = buf[..pos]
            .iter()
            .rposition(|b| *b == b'\n')
            .map(|p| p + 1)
            .unwrap_or(0);
        buf.drain(line_start..pos + ERASE_LINE.len());
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Remove CSI escape sequences (`ESC [ … final-byte`).
#[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_redraws_leave_no_trace() {
        let t = Transcript::default();
        let mut w = t.writer();
        writeln!(w, "line one").unwrap();
        write!(w, "\r\x1b[2K ●●●● thinking…").unwrap();
        write!(w, "\r\x1b[2K ●●●● still thinking…").unwrap();
        write!(w, "\r\x1b[2K").unwrap();
        writeln!(w, "\x1b[96m◆ agent:\x1b[0m hello").unwrap();
        assert_eq!(t.plain_text(), "line one\n◆ agent: hello\n");
    }

    #[test]
    fn caps_size_at_a_line_boundary() {
        let t = Transcript::default();
        let mut w = t.writer();
        let line = "x".repeat(1023) + "\n";
        for _ in 0..600 {
            w.write_all(line.as_bytes()).unwrap();
        }
        let bytes = t.bytes();
        assert!(bytes.len() <= MAX_BYTES);
        assert_eq!(bytes.len() % 1024, 0);
    }
}
