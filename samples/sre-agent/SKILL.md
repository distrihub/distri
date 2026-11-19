---
name: distri-sre-agent
description: Deploy an SRE agent on distri that the user can ask from Telegram, with a scheduled health check that alerts them once when their service goes down and investigates. Use when the user wants on-call monitoring, health checks with alerts, or an SRE/ops agent.
---

# Deploy the distri SRE agent

You are setting up a hosted agent for the user on distri. Ask only for what you cannot find yourself.

1. **CLI.** Run `distri --version`. If it is missing, install it with `curl -fsSL https://distri.dev/install.sh | sh`.
2. **Sign in.** If `DISTRI_API_KEY` is set, use it. Otherwise run `distri login`; it opens a browser.
3. **Find the health URL.** Look in this repository for a health route (`/health`, `/healthz`, `/status`, a `healthcheck` in Docker or k8s files) and the public base URL (deploy config, README, `.env.example`). Confirm the full URL with the user in one question. Also note the GitHub repo (`git remote get-url origin`) and the service name.
4. **Apply:**
   ```bash
   distri apply sre-agent --json \
     --set service_url=<url> --set service_name="<name>" --set repo=<owner/name>
   ```
   If `status` is `needs_params`, ask the user each `missing_params[].ask` and run it again with `--set`.
5. **Finish the steps that need the user.** For each item in `pending` where `done` is false, show the `title` and the `url`. The Telegram link must be opened on the user's phone or desktop. GitHub is optional. Then run `distri apply sre-agent --wait --test` with the same `--set` values. It waits for the links, then sends a test message on Telegram.
6. **Report.** Tell the user what exists now (`items`), that alerts and answers come in Telegram, and that they can ask the agent anything there, for example "is the API healthy?" or "what changed in the last deploy?".

Never put secrets in the command line. If the health URL needs a token, ask the user to add it as a workspace secret in distri instead.
