//! ANSI color constants for terminal rendering.
//! Shared across all crates that need terminal colors.

pub const COLOR_RESET: &str = "\x1b[0m";
pub const COLOR_RED: &str = "\x1b[31m";
pub const COLOR_GREEN: &str = "\x1b[32m";
pub const COLOR_YELLOW: &str = "\x1b[33m";
pub const COLOR_CYAN: &str = "\x1b[36m";
pub const COLOR_GRAY: &str = "\x1b[90m";
pub const COLOR_BRIGHT_CYAN: &str = "\x1b[96m";
