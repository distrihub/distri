# observability

`https://distri.dev/a2a/extensions/observability/v1`

- **Request metadata:** `tags` (searchable key/values on the run's spans and thread),
  `trace_context` (W3C trace parent).
- **Events:** `llm_call`, `diagnostic_log` (verbose servers).
- **params:** `metadata`, `events`.
