# Use case: signal digest

**As:** a signed-up user (`user` account) with Telegram connected.

1. Run `distri apply signal-digest --set topic="AI agents" --wait --test`.
2. Run `distri apply signal-digest --set topic="AI agents" --test` again.

**Expect:**
- the first test sends a 📰 digest with at least 3 links about the topic to Telegram, or "Nothing new…" on a quiet day;
- the workflow `signal_digest` has a weekday schedule at the chosen hour;
- after a scheduled run, the next run does not repeat a link.
