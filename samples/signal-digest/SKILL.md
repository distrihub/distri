---
name: distri-signal-digest
description: Deploy a daily Telegram digest on distri of the Hacker News and Reddit posts worth reading on a topic (customers, competitors, a category). Use when the user wants a news, signal or social-listening digest.
---

# Deploy the distri signal digest

1. Check `distri --version`; if it is missing, install it with `curl -fsSL https://distri.dev/install.sh | sh`. Sign in with `distri login`, or use `DISTRI_API_KEY`.
2. Ask the user for the **topic** in one question. Infer the time zone from the system (`date +%Z`, or the `TZ` environment variable), and use 8am unless they said otherwise.
3. Run `distri apply signal-digest --json --set topic="<topic>" --set hour=<h> --set timezone=<IANA zone>`.
4. Show the Telegram link from `pending` and ask the user to open it. Then run `distri apply signal-digest --wait --test` with the same `--set` values. A first digest arrives right away.
5. Tell the user it arrives on weekdays at that hour and never repeats a link.
