//! Limits and secret scrubbing applied to everything memory stores (security review of #304).
//!
//! - Caps: an item body is at most [`MAX_BODY_BYTES`]; a signal's `before`, `after` and
//!   `context` at most [`MAX_SIGNAL_FIELD_BYTES`] each; ids at most [`MAX_ID_CHARS`]; one
//!   request posts at most [`MAX_SIGNALS_PER_REQUEST`] signals.
//! - Scrub: token-like values (bearer tokens, `sk-…`, `ghp_…`, `xox…-`, long hex or base64)
//!   are replaced by `[redacted]` before a signal or an item is stored, whoever wrote it.

use std::sync::OnceLock;

use regex::Regex;

use super::MemorySignal;

pub const MAX_BODY_BYTES: usize = 32 * 1024;
pub const MAX_SIGNAL_FIELD_BYTES: usize = 8 * 1024;
pub const MAX_ID_CHARS: usize = 200;
pub const MAX_SIGNALS_PER_REQUEST: usize = 100;
/// What reflection reads of one signal's `before`/`after`, and of its context.
pub const REFLECTION_FIELD_CHARS: usize = 2000;
pub const REDACTED: &str = "[redacted]";

fn patterns() -> &'static [Regex] {
    static P: OnceLock<Vec<Regex>> = OnceLock::new();
    P.get_or_init(|| {
        [
            r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{8,}",
            r"\bsk-[A-Za-z0-9_-]{16,}",
            r"\bgh[pousr]_[A-Za-z0-9]{20,}",
            r"\bgithub_pat_[A-Za-z0-9_]{20,}",
            r"\bxox[abposr]-[A-Za-z0-9-]{10,}",
            r"\bAKIA[0-9A-Z]{16}\b",
            r"\b[0-9a-fA-F]{32,}\b",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("scrub pattern"))
        .collect()
    })
}

fn base64_run() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"[A-Za-z0-9+/_=]{40,}").expect("base64 pattern"))
}

/// `text` with token-like values replaced by `[redacted]`.
pub fn scrub_secrets(text: &str) -> String {
    let mut out = text.to_string();
    for p in patterns() {
        if p.is_match(&out) {
            out = p.replace_all(&out, REDACTED).into_owned();
        }
    }
    // A long run of base64 characters that mixes cases and digits reads as a key, not prose.
    base64_run()
        .replace_all(&out, |c: &regex::Captures<'_>| {
            let s = &c[0];
            let mixed = s.chars().any(|c| c.is_ascii_digit())
                && s.chars().any(|c| c.is_ascii_uppercase())
                && s.chars().any(|c| c.is_ascii_lowercase());
            if mixed {
                REDACTED.to_string()
            } else {
                s.to_string()
            }
        })
        .into_owned()
}

/// Every string inside a JSON value, scrubbed.
pub fn scrub_value(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::String(s) => serde_json::Value::String(scrub_secrets(&s)),
        serde_json::Value::Array(a) => {
            serde_json::Value::Array(a.into_iter().map(scrub_value).collect())
        }
        serde_json::Value::Object(o) => {
            serde_json::Value::Object(o.into_iter().map(|(k, v)| (k, scrub_value(v))).collect())
        }
        other => other,
    }
}

/// Refuse a signal over the caps (`too_large: …`) or with a bad id (`invalid: …`).
pub fn check_signal(s: &MemorySignal) -> Result<(), String> {
    if s.id.is_empty() || s.id.chars().count() > MAX_ID_CHARS {
        return Err(format!(
            "invalid: a signal id is 1 to {MAX_ID_CHARS} characters"
        ));
    }
    for (name, len) in [
        ("before", s.before.as_ref().map(|t| t.len()).unwrap_or(0)),
        ("after", s.after.as_ref().map(|t| t.len()).unwrap_or(0)),
        ("subject", s.subject.as_ref().map(|t| t.len()).unwrap_or(0)),
        ("context", s.context.to_string().len()),
    ] {
        if len > MAX_SIGNAL_FIELD_BYTES {
            return Err(format!(
                "too_large: signal {name} is {len} bytes (at most {MAX_SIGNAL_FIELD_BYTES})"
            ));
        }
    }
    Ok(())
}

/// The signal with its text fields scrubbed.
pub fn scrub_signal(mut s: MemorySignal) -> MemorySignal {
    s.subject = s.subject.map(|t| scrub_secrets(&t));
    s.before = s.before.map(|t| scrub_secrets(&t));
    s.after = s.after.map(|t| scrub_secrets(&t));
    s.context = scrub_value(s.context);
    s
}

/// `text` cut to at most `max` characters, marking the cut.
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str(" …[cut]");
    out
}
