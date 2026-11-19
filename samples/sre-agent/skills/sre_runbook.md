---
name: sre_runbook
description: "How the SRE agent investigates a failed health check"
---

# Failed health check: runbook

1. **Reproduce.** Call the health URL twice, a few seconds apart. Record the HTTP status, latency and the first 300 characters of the body. A `0` status means the connection failed (DNS, TLS, refused, timeout).
2. **Classify.**
   - 5xx with a body: the service is up but failing; look at recent changes.
   - Timeout or `0`: network, DNS, TLS or the host is down; check DNS resolution and the certificate expiry.
   - 4xx (401/403/404): the health route moved or now needs auth; probably a deploy.
3. **Recent changes.** With GitHub: commits and merged PRs in the last 6 hours, and the latest workflow runs. Name the commit and author closest before the first failure.
4. **Upstream.** Search for an ongoing incident at the hosting or database provider.
5. **Report** what is broken, since when, the likely cause with evidence, and one recommended next step. Never take an action yourself.
