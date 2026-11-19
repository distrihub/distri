# Use case: SRE agent from one sentence

**As:** a fresh sign-up (`fresh` account).

## Web chat

1. Open the web app and start a new chat with distri.
2. Type: *Create an SRE agent for `<fixture>/health` that I can ask from Telegram; check /health every 5 minutes, and if it fails, alert me and start investigating.*
3. Confirm the card that lists `sre_agent`, `sre_health_check` and Telegram.
4. Open the Telegram link and press Start.
5. Wait for the test message.

**Expect:**
- the agent `sre_agent` and the workflow `sre_health_check` exist, with the schedule on;
- Telegram says "Your SRE agent for … is connected";
- when the fixture returns 500, exactly one 🔴 alert arrives, then a 🔎 report;
- when it returns 200 again, one ✅ "recovered" arrives.

## CLI / coding agent

```bash
distri apply sre-agent --set service_url=<fixture>/health --wait --test
```

**Expect:** nothing is left pending, and the test message arrives on Telegram.
