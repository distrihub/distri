# Tool Result Storage: Switch to Azure Blob

## Current approach

Large tool results (>8KB) are persisted to the **session store (Postgres)** under the namespace
`tool-results:{thread_id}` with key `{step_id}_{part_index}`. The compact preview is kept in the
scratchpad. This works for the current single-replica deployment.

See: `server/distri-core/src/agent/context.rs` — `persist_large_parts` / `load_persisted_result`.

## Why switch to Azure Blob

- Postgres `session_entries.value` is `JSONB` — fine for tool results up to ~100KB but becomes a
  problem for large shell outputs, file reads, or web scrapes that can hit 500KB–5MB.
- Azure Blob has no practical size limit, costs a fraction of Postgres I/O, and offloads binary
  data from the DB connection pool.

## What needs to happen

1. **Add `AzureBlob` variant to `ObjectStorageConfig`** in `distri-types/src/configuration/config.rs`:
   ```rust
   AzureBlob {
       account: String,
       container: String,
       sas_token: Option<String>,
       access_key: Option<String>,
   }
   ```

2. **Implement it in `distri-filesystem/src/object_store.rs`** using `object_store::azure::MicrosoftAzureBuilder`.
   The `object_store` crate already has this — just wire it up.

3. **Add a dedicated `session_filesystem` config** to `DistriServerConfig` (or read from env vars
   `AZURE_STORAGE_ACCOUNT`, `AZURE_STORAGE_KEY`, `AZURE_TOOL_RESULTS_CONTAINER`).

4. **Update `persist_large_parts` / `load_persisted_result`** in `context.rs` to write to
   `session_filesystem` instead of the session store, reverting to the filesystem path approach
   but now backed by Azure Blob.

5. **Keep session store as fallback** if blob storage is not configured (current behaviour).

## Threshold to trigger this work

When tool results regularly exceed ~100KB (large file reads, big API responses, long shell outputs),
or when Postgres storage cost / query latency becomes noticeable.
