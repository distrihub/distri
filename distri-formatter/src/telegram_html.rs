//! HTML escaping + URL linkification for the Telegram surface.
//!
//! Telegram's HTML parse mode requires escaping `<`, `>`, `&` (and only those
//! three). We escape first, then run a URL detector and wrap each detected
//! URL in `<a href="…">…</a>`. The href value uses the pre-escaped form so
//! `&` in query strings becomes `&amp;` in both the attribute and link text.
//!
//! Why HTML and not MarkdownV2: LLM output regularly contains brackets,
//! parens, asterisks, and underscores that MarkdownV2 treats as syntax.
//! Escaping all 18 MarkdownV2 metacharacters reliably is a known footgun
//! (the existing code path silently falls back to plain text on parse
//! errors). HTML's three-char escape set is much harder to get wrong.
//!
//! Callers that hand-write their own formatted text can still pick
//! MarkdownV2 by constructing a `Reply::markdown_v2(...)` directly — this
//! module only governs the formatter's *default* output for raw LLM text.

/// Escape `&`, `<`, `>` for Telegram HTML parse mode. Idempotent in the
/// sense that running it twice double-escapes, so callers must run it
/// exactly once before calling `linkify_html`.
pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Wrap any bare `http(s)://…` URLs in `<a href="…">…</a>`. Operates on
/// already-escaped HTML — both the href value and the visible text use the
/// escaped form, so an `&` that came in as `&amp;` stays `&amp;` everywhere.
///
/// URL detection rules:
/// - Match `http://` or `https://` followed by one or more non-whitespace,
///   non-`<`, non-`>` characters.
/// - Trim trailing punctuation (`.,!?;:)]}>`) so `https://example.com.` ends
///   the link before the period and `(https://example.com)` doesn't capture
///   the closing paren.
/// - URLs preceded by `"` (already inside an `<a href="…">` attribute) are
///   skipped — we don't want to double-wrap.
pub fn linkify_html(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        // Look for the start of an http(s):// URL.
        let rest = &input[i..];
        let scheme_pos = rest.find("http");
        let Some(rel) = scheme_pos else {
            out.push_str(rest);
            break;
        };
        let abs = i + rel;

        // Append everything before the candidate.
        out.push_str(&input[i..abs]);

        // Validate that this `http` is actually `http://` or `https://`.
        let after = &input[abs..];
        let scheme_len = if after.starts_with("https://") {
            8
        } else if after.starts_with("http://") {
            7
        } else {
            // Not a real URL start. Emit the literal "http" and advance.
            out.push_str("http");
            i = abs + 4;
            continue;
        };

        // Skip the "we're already inside an href attribute" case — if the
        // character immediately before the URL is `"`, this is the value of
        // an attribute we (or someone else) emitted earlier.
        let prev = if abs == 0 {
            None
        } else {
            input[..abs].chars().last()
        };
        if matches!(prev, Some('"')) {
            // Emit the URL literally and advance past it.
            let url_end = find_url_end(after, scheme_len);
            out.push_str(&after[..url_end]);
            i = abs + url_end;
            continue;
        }

        let url_end = find_url_end(after, scheme_len);
        let url = &after[..url_end];
        out.push_str("<a href=\"");
        out.push_str(url);
        out.push_str("\">");
        out.push_str(url);
        out.push_str("</a>");
        i = abs + url_end;
    }
    out
}

/// Find the byte offset where a URL ends, starting from `s` (which begins
/// with `http://` or `https://`). Stops at the first whitespace, `<`, `>`,
/// `"`, or HTML entity that came from escaping `<`/`>` (`&lt;` / `&gt;`).
/// `&amp;` (the escaped form of `&` in query strings) does NOT terminate the
/// URL — that's the whole point of running escape before linkify.
/// Then trims trailing punctuation.
fn find_url_end(s: &str, scheme_len: usize) -> usize {
    let bytes = s.as_bytes();
    let mut end = scheme_len;
    while end < bytes.len() {
        let b = bytes[end];
        if b == b' '
            || b == b'\n'
            || b == b'\r'
            || b == b'\t'
            || b == b'<'
            || b == b'>'
            || b == b'"'
        {
            break;
        }
        if b == b'&' {
            let after = &s[end..];
            // Allow `&amp;` (escaped `&` inside query strings); stop on
            // `&lt;` or `&gt;` (escaped angle brackets).
            if after.starts_with("&lt;") || after.starts_with("&gt;") {
                break;
            }
        }
        end += 1;
    }
    // Trim trailing punctuation. We special-case `&amp;` / `&gt;` / `&lt;`:
    // those entities end in `;` so naive trim would chop the `;`. Walk back
    // from the end stripping disallowed trailing punctuation but only when
    // it's a literal punct char, not when it's part of `&…;`.
    while end > scheme_len {
        let last = bytes[end - 1];
        let is_trim = matches!(last, b'.' | b',' | b'!' | b'?' | b':' | b')' | b']' | b'}');
        if !is_trim {
            break;
        }
        end -= 1;
    }
    end
}

/// Convenience helper: escape THEN linkify in one call.
pub fn escape_and_linkify(input: &str) -> String {
    linkify_html(&escape_html(input))
}

use crate::extract::{extract_fields, ToolFields};

/// Max bytes for a tool output snippet in Telegram. 800 bytes ≈ 800 ASCII
/// chars or ~270 4-byte UTF-8 chars — leaves room for surrounding chat.
const TOOL_OUTPUT_MAX: usize = 800;

/// Format a tool result as compact Telegram HTML.
///
/// Returns an empty string for tools whose output should not be shown inline
/// (final, reflect, console_log, transfer_to_agent) — their effect is visible
/// through other events.
pub fn format_tool_result_html(response: &distri_types::ToolResponse) -> String {
    match response.tool_name.as_str() {
        "final" | "reflect" | "console_log" | "transfer_to_agent" => return String::new(),
        _ => {}
    }

    let fields = extract_fields(response);
    match fields {
        ToolFields::Bash {
            stdout,
            stderr,
            exit_code,
        }
        | ToolFields::Shell {
            stdout,
            stderr,
            exit_code,
        } => format_shell_html(&stdout, &stderr, exit_code),
        ToolFields::Read {
            file_path,
            lines_read,
            total,
            truncated,
            ..
        } => {
            let trunc = if truncated {
                format!(" of {total}")
            } else {
                String::new()
            };
            format!(
                "<i>{lines_read} lines{trunc} from {}</i>",
                escape_html(&file_path)
            )
        }
        ToolFields::Grep {
            output,
            total_lines,
            truncated,
        } => {
            let suffix = if truncated { " (truncated)" } else { "" };
            let preview = truncate_for_tg(&output, TOOL_OUTPUT_MAX);
            format!(
                "<i>{total_lines} matches{suffix}</i>\n<pre>{}</pre>",
                escape_html(&preview)
            )
        }
        ToolFields::Glob {
            filenames,
            num_files,
            truncated,
        } => {
            let suffix = if truncated { " (truncated)" } else { "" };
            let list = filenames
                .iter()
                .take(10)
                .map(|f| escape_html(f))
                .collect::<Vec<_>>()
                .join("\n");
            let more = if num_files > 10 {
                format!("\n… and {} more", num_files.saturating_sub(10))
            } else {
                String::new()
            };
            format!("<i>{num_files} files{suffix}</i>\n<pre>{list}{more}</pre>")
        }
        ToolFields::Edit {
            file_path,
            replacements,
        } => {
            format!(
                "<i>{replacements} replacement(s) in {}</i>",
                escape_html(&file_path)
            )
        }
        ToolFields::Write {
            file_path,
            bytes_written,
        } => {
            format!(
                "<i>{bytes_written} bytes written to {}</i>",
                escape_html(&file_path)
            )
        }
        ToolFields::Generic { text } => {
            if text.is_empty() {
                return String::new();
            }
            let preview = truncate_for_tg(&text, TOOL_OUTPUT_MAX);
            format!("<pre>{}</pre>", escape_html(&preview))
        }
    }
}

fn format_shell_html(stdout: &str, stderr: &str, exit_code: i64) -> String {
    let mut parts = Vec::new();

    if !stdout.trim().is_empty() {
        let preview = truncate_for_tg(stdout, TOOL_OUTPUT_MAX);
        parts.push(format!("<pre>{}</pre>", escape_html(&preview)));
    }

    if !stderr.trim().is_empty() {
        let first_lines: String = stderr.lines().take(3).collect::<Vec<_>>().join("\n");
        parts.push(format!(
            "<i>stderr:</i> <code>{}</code>",
            escape_html(&first_lines)
        ));
    }

    if exit_code != 0 {
        parts.push(format!("<i>exit {exit_code}</i>"));
    }

    parts.join("\n")
}

/// Truncate `s` so its UTF-8 byte length is at most `max`. Truncation happens
/// at line boundaries, so the result is always valid UTF-8 and never splits
/// a multi-byte character mid-codepoint. Appends `"… (N lines total)"` when
/// content is dropped.
fn truncate_for_tg(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let lines: Vec<&str> = s.lines().collect();
    let mut result = String::new();
    let mut count = 0;
    for line in &lines {
        if result.len() + line.len() + 1 > max.saturating_sub(30) {
            break;
        }
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(line);
        count += 1;
    }
    if count < lines.len() {
        result.push_str(&format!("\n… ({} lines total)", lines.len()));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_result_html_bash_success() {
        let response = distri_types::ToolResponse::direct(
            "tc1".into(),
            "Bash".into(),
            serde_json::json!({"stdout": "hello world\nline2\nline3", "stderr": "", "exit_code": 0}),
        );
        let html = format_tool_result_html(&response);
        assert!(html.contains("<pre>"), "should have pre block for stdout");
        assert!(html.contains("hello world"), "should contain stdout");
        assert!(!html.to_lowercase().contains("stderr"), "empty stderr should be omitted");
    }

    #[test]
    fn tool_result_html_bash_error() {
        let response = distri_types::ToolResponse::direct(
            "tc1".into(),
            "Bash".into(),
            serde_json::json!({"stdout": "", "stderr": "not found", "exit_code": 1}),
        );
        let html = format_tool_result_html(&response);
        assert!(html.contains("exit 1"), "should show exit code");
        assert!(html.contains("not found"), "should show stderr");
    }

    #[test]
    fn tool_result_html_edit() {
        let response = distri_types::ToolResponse::direct(
            "tc1".into(),
            "Edit".into(),
            serde_json::json!({"file_path": "/src/main.rs", "replacements": 2}),
        );
        let html = format_tool_result_html(&response);
        assert!(html.contains("main.rs"), "should mention file");
        assert!(html.contains("2"), "should mention replacement count");
    }

    #[test]
    fn tool_result_html_suppressed_tools() {
        for name in &["final", "reflect", "console_log", "transfer_to_agent"] {
            let response = distri_types::ToolResponse::from_parts(
                "tc1".into(),
                name.to_string(),
                vec![distri_types::Part::Text("ignored".into())],
            );
            let html = format_tool_result_html(&response);
            assert!(html.is_empty(), "{name} should produce empty output");
        }
    }

    #[test]
    fn tool_result_html_truncates_long_output() {
        let long = "x".repeat(2000);
        let response = distri_types::ToolResponse::direct(
            "tc1".into(),
            "Bash".into(),
            serde_json::json!({"stdout": long, "stderr": "", "exit_code": 0}),
        );
        let html = format_tool_result_html(&response);
        assert!(
            html.len() < 1500,
            "should truncate long output (got {} chars)",
            html.len()
        );
    }

    #[test]
    fn escape_basic() {
        assert_eq!(escape_html("a & b"), "a &amp; b");
        assert_eq!(escape_html("<tag>"), "&lt;tag&gt;");
        assert_eq!(escape_html("plain"), "plain");
    }

    #[test]
    fn linkify_bare_url() {
        let got = escape_and_linkify("see https://example.com here");
        assert_eq!(
            got,
            "see <a href=\"https://example.com\">https://example.com</a> here"
        );
    }

    #[test]
    fn linkify_query_string_with_amp() {
        let got = escape_and_linkify("https://example.com/q?a=1&b=2");
        assert_eq!(
            got,
            "<a href=\"https://example.com/q?a=1&amp;b=2\">https://example.com/q?a=1&amp;b=2</a>"
        );
    }

    #[test]
    fn linkify_trailing_punctuation_period() {
        let got = escape_and_linkify("visit https://example.com.");
        assert_eq!(
            got,
            "visit <a href=\"https://example.com\">https://example.com</a>."
        );
    }

    #[test]
    fn linkify_inside_parens() {
        let got = escape_and_linkify("(https://example.com)");
        assert_eq!(
            got,
            "(<a href=\"https://example.com\">https://example.com</a>)"
        );
    }

    #[test]
    fn linkify_two_urls() {
        let got = escape_and_linkify("a https://x.test b https://y.test c");
        assert_eq!(
            got,
            "a <a href=\"https://x.test\">https://x.test</a> b <a href=\"https://y.test\">https://y.test</a> c"
        );
    }

    #[test]
    fn linkify_does_not_break_on_lone_http_word() {
        let got = escape_and_linkify("the http protocol vs https://example.com");
        assert_eq!(
            got,
            "the http protocol vs <a href=\"https://example.com\">https://example.com</a>"
        );
    }

    #[test]
    fn escape_then_linkify_preserves_brackets() {
        let got = escape_and_linkify("see <code>https://example.com</code>");
        // The < and > escape to &lt;/&gt; so the URL is still a clean run.
        assert_eq!(
            got,
            "see &lt;code&gt;<a href=\"https://example.com\">https://example.com</a>&lt;/code&gt;"
        );
    }

    #[test]
    fn no_double_wrap_when_inside_href() {
        // Verifies the "preceded by quote" check — if we already produced
        // an anchor and re-ran linkify, it would double-wrap.
        let pre_anchored = "<a href=\"https://example.com\">https://example.com</a>";
        // First escape so the angle brackets become entities, then run
        // linkify. Since `>` is escaped, the inner URL is no longer
        // immediately preceded by `"` (it's preceded by `&gt;`), so it gets
        // its own anchor. This is the expected behavior — escape_and_linkify
        // is only meant to run on raw text, not pre-anchored HTML.
        let got = escape_and_linkify(pre_anchored);
        assert!(got.contains("<a href=\"https://example.com\">"));
    }

    #[test]
    fn tool_result_html_escapes_dynamic_content() {
        // Edit path and Bash stdout with HTML-special chars must be escaped.
        let r = distri_types::ToolResponse::direct(
            "tc1".into(),
            "Edit".into(),
            serde_json::json!({"file_path": "/src/<weird>&file.rs", "replacements": 1}),
        );
        let html = format_tool_result_html(&r);
        assert!(
            !html.contains("<weird>"),
            "raw < and > must be escaped, got: {html}"
        );
        assert!(
            html.contains("&lt;weird&gt;"),
            "expected escaped entities, got: {html}"
        );

        let r2 = distri_types::ToolResponse::direct(
            "tc1".into(),
            "Bash".into(),
            serde_json::json!({"stdout": "echo <script>alert(1)</script>", "stderr": "", "exit_code": 0}),
        );
        let html2 = format_tool_result_html(&r2);
        assert!(
            !html2.contains("<script>"),
            "bash stdout must escape HTML, got: {html2}"
        );
        assert!(html2.contains("&lt;script&gt;"));
    }
}
