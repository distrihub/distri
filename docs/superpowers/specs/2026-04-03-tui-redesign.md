# Distri TUI Redesign

## Goal

Persistent sticky-bottom input area that stays visible during agent streaming, with scrollable output above — matching the UX of Claude Code.

## What We Have Now

- `rustyline` for readline input (history, completion)
- `EventPrinter` writes directly to stdout via `println!`
- Input prompt only appears after streaming completes — invisible during agent turns
- Context status shown on separator line before each prompt

## What We Want

```
┌──────────────────────────────────────────────┐
│ ⏺ Bash("npm run build")                      │ ← scrolls up
│   ⎿ exit 0                                   │
│                                              │
│ I've built the project. The output is...     │ ← agent text
│                                              │
│   ↳ 20K in, 480 out · $0.0031               │ ← already done ✓
├──────────────────────────────────────────────┤
│ Context: 12% (24K/200K) · claude-sonnet-4-6 │ ← status bar
│ > █                                          │ ← always visible
└──────────────────────────────────────────────┘
```

## Architecture

### Option A: ANSI Scroll Region + Raw Mode (Recommended)

No new dependencies. Uses terminal primitives already available via crossterm.

**How it works:**
1. Set scroll region once at session start: `\x1b[1;{rows-2}r` — rows above the bottom 2 are the scrollable area
2. Raw mode toggled: ON during input reading, OFF during streaming
3. Streaming output via `println!` works normally in cooked mode, scrolls within the scroll region
4. Bottom 2 rows (status bar + input) live outside the scroll region — never scrolled away
5. Status bar updated via cursor save/restore (`\x1b7`/`\x1b8`) without disrupting scroll output

**Replace rustyline with a custom `StickyInput` reader** (crossterm events):
- Character input, backspace, delete
- Left/right cursor movement within line
- Up/down history navigation
- Ctrl+A/E (home/end), Ctrl+C (interrupt), Ctrl+D (exit)
- Retain rustyline for history persistence to disk

**Scope:**
- New file: `distri-cli/src/sticky_input.rs` (~200 lines)
- Modify: `distri-cli/src/chat.rs` — replace readline loop, remove separator logic (~100 line change)
- No changes to printer, no new dependencies

**Risk:** Some terminals don't support scroll regions (rare, <1%). Fallback: detect and degrade gracefully to current behavior.

### Option B: Ratatui (Full TUI Framework)

Proper retained-mode TUI. More capability, more complexity.

**How it works:**
- `ratatui` manages the full terminal as a frame buffer
- Layout: `Constraint::Fill(1)` (output) + `Constraint::Length(3)` (input)
- Streaming output buffered in `Vec<Line>` → render on each frame tick
- Custom key handler replaces rustyline

**Requires:**
- Add `ratatui = "0.30"` dependency
- Add `tui-input` or custom text field widget
- Modify `EventPrinter` to write to a `tokio::sync::mpsc` channel instead of stdout (biggest change)
- New `TuiApp` state struct + event loop replacing current chat.rs loop

**Scope:** ~500 lines new code, significant printer refactor.

**Benefit over Option A:** Mouse support, rich widgets (tables, borders, colors via ratatui palette), easier to extend later.

## Recommendation

Start with **Option A**. It's ~1 day of work, no new dependencies, same visual result for the core feature. If we later need mouse support or richer widgets, migrate to ratatui at that point.

## Context Status Bar Content

Status bar (row rows-1) shows, right-aligned, in gray:
```
Context: 12% (24K/200K) · 20K in, 480 out · $0.003 · claude-sonnet-4-6
```
- Green when <70% context used
- Yellow when 70–90%
- Red when >90%

During streaming, replace with:
```
⏳  20K in, 480 out · $0.003  (updating live every 500ms)
```

## Input Row Content

Bottom row always shows:
```
> user_input_here█
```

Cursor position tracked within the input buffer. During streaming, shows:
```
  (streaming...)
```
or is blank/dimmed to indicate not accepting input.

## Files to Change

| File | Change |
|------|--------|
| `distri-cli/src/sticky_input.rs` | New — StickyTerminal + StickyInput reader |
| `distri-cli/src/chat.rs` | Replace rustyline loop with StickyTerminal |
| `distri-cli/src/main.rs` | Add `mod sticky_input` |
| `distri-cli/Cargo.toml` | No new deps for Option A |
| `distri/src/printer.rs` | No change (already writes to stdout) |

## What's Already Done

- `ContextHealth` struct with `format_status_line()` ✓
- `RwLock<ContextHealth>` shared between printer and chat loop ✓
- Turn-end token summary printed via `↳ 20K in, 480 out · $0.003` ✓
- Context health updated on `RunFinished` and `ContextBudgetUpdate` events ✓
