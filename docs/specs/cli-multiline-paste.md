# CLI Multi-line Paste Support

## Problem

In `distri-cli/src/main.rs`, the interactive chat uses `inquire::Text` for input. When users paste multi-line text, each newline is treated as Enter (submit), causing the paste to be split into multiple separate messages.

## Goal

- Support multi-line text input (pasted or typed)
- Pasted newlines must not trigger submit
- Keep the same look and feel: prompt `> `, placeholder hint, slash command autocomplete, history

## Solution: Replace `inquire::Text` with `rustyline`

**Crate**: `rustyline = { version = "17", features = ["derive"] }`

rustyline has native **bracket paste mode** — terminals wrap pasted content in `\x1b[200~` ... `\x1b[201~` escape sequences. rustyline detects this and treats newlines within a paste as literal `\n` characters, not submit.

### What to replace

**Remove**: `inquire` dependency (only `Select` is used elsewhere — keep that).

**Add**: `rustyline` dependency.

### Implementation

#### 1. New `DistriHelper` struct (replaces `DistriAutocomplete`)

Implements rustyline's `Helper` trait (which combines `Completer`, `Hinter`, `Highlighter`, `Validator`):

```rust
use rustyline::completion::{Completer, Pair};
use rustyline::hint::{Hint, Hinter};
use rustyline::highlight::Highlighter;
use rustyline::validate::Validator;
use rustyline::{Config, Context, Editor, Helper, error::ReadlineError};

#[derive(Helper, Highlighter, Validator)]
struct DistriHelper {
    slash_commands: Vec<String>,
    matcher: SkimMatcherV2,
}
```

**Completer**: Port existing `get_suggestions` logic — fuzzy match against slash commands. Only complete when input starts with `/`.

```rust
impl Completer for DistriHelper {
    type Candidate = Pair;
    fn complete(&self, line: &str, pos: usize, _ctx: &Context<'_>)
        -> rustyline::Result<(usize, Vec<Pair>)>
    {
        if !line.starts_with('/') {
            return Ok((0, vec![]));
        }
        // fuzzy match slash_commands against line[..pos]
        // return (0, vec![Pair { display, replacement }])
    }
}
```

**Hinter**: Show placeholder when input is empty.

```rust
impl Hinter for DistriHelper {
    type Hint = String;
    fn hint(&self, line: &str, _pos: usize, _ctx: &Context<'_>) -> Option<String> {
        if line.is_empty() {
            Some("/help for commands... Ask me anything".to_string())
        } else {
            None
        }
    }
}
```

#### 2. Editor setup

```rust
let config = Config::builder()
    .auto_add_history(true)
    .bracket_paste(true)  // KEY: enables multi-line paste support
    .build();
let mut rl = Editor::with_config(config)?;
rl.set_helper(Some(DistriHelper::new()));

// Load history from existing history file
let history_path = distri_home_dir().join("history.txt");
let _ = rl.load_history(&history_path);
```

#### 3. Chat loop replacement

Replace the `Text::new("> ").prompt()` block in `run_interactive_chat` (~line 876):

```rust
loop {
    print_context_status();
    print_separator_line();

    let input = match rl.readline("> ") {
        Ok(line) => {
            print_help_options();
            line
        }
        Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => {
            println!("\nExiting...");
            break;
        }
        Err(err) => {
            eprintln!("Error reading input: {}", err);
            continue;
        }
    };

    let input = input.trim();
    if input.is_empty() {
        continue;
    }

    // ... rest of loop unchanged
}

// Save history on exit
let _ = rl.save_history(&history_path);
```

#### 4. History

rustyline manages history natively with `auto_add_history(true)`. Remove manual history management (`load_history`, `save_history`, `history` vec, `autocomplete.update_history`). Use rustyline's `rl.load_history()` / `rl.save_history()` with the same `~/.distri/history.txt` file.

#### 5. Other `Text::new` usages

There are a few other `Text::new` calls in `handle_slash_command` (model name input, etc.). These can stay as `inquire::Text` since they're simple single-line prompts where paste isn't an issue. Keep `inquire` as a dependency for `Select` and these.

### Files to modify

- `distri-cli/Cargo.toml` — add `rustyline = { version = "17", features = ["derive"] }`
- `distri-cli/src/main.rs`:
  - Add rustyline imports
  - Replace `DistriAutocomplete` with `DistriHelper` (Completer + Hinter)
  - Replace chat loop input with `rl.readline("> ")`
  - Replace manual history with rustyline's built-in history
  - Keep `inquire` for `Select` and other simple prompts

### Behavior

| Action | Before | After |
|--------|--------|-------|
| Type + Enter | Submit | Submit (same) |
| Paste multi-line | 3 separate messages | 1 message with newlines |
| Ctrl+C | Exit | Exit (same) |
| Ctrl+D | Exit | Exit (same) |
| `/help` + Tab | Autocomplete | Autocomplete (same) |
| Empty prompt | Shows placeholder | Shows hint text (same look) |
| Up arrow | No history nav | History navigation (improvement) |
