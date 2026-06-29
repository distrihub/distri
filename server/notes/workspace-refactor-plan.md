# Workspace Refactor Plan

## Objectives

1. Run Distri inside an explicit working directory (default `CURRENT_WORKING_DIR=examples`) that exposes `agents/`, `src/mod.ts`, and `plugins/`.
2. Remove legacy `.distri` assumptions from the CLI, backend, and frontend.
3. Replace the old "skills" UX with an **Agents + Files** workspace powered by the filesystem APIs, while leaving room to add the future "skills = partial agents" feature on top of the same file-backed infrastructure.
4. Keep Distri embeddable: discovery/registration happens in `distri-cli`, while `distri` itself only consumes pre-registered assets.

## Workstreams & Tasks

### 1. Environment & Workspace Management
- Add `CURRENT_WORKING_DIR` handling to `distri-cli` (default `.`; override via env/flag) and thread it through all commands.
- Ensure helper functions (`load_agents_dir`, `load_plugins_dir`, `load_workspace_mod`) accept workspace-relative paths and emit useful logs.
- Update documentation and samples to place agents/plugins under `examples/` for local testing.

### 2. CLI Registration Flow
- On startup, resolve `CURRENT_WORKING_DIR` and register:
  - Markdown agents under `${cwd}/agents`.
  - Programmatic registrations from `${cwd}/src/mod.ts` (if present).
  - Plugins under `${cwd}/plugins/**`.
- Remove any `.distri` fallbacks; surface warnings if required folders are missing.

### 3. Backend (distri-server)
- Replace the `skills` endpoint with new handlers that expose:
  - Agent metadata (from orchestrator registry).
  - File browsing/editing APIs backed by `${cwd}` and the filesystem object store.
- Ensure orchestrator loads exactly one workspace prefix at a time and shares the same filesystem instance as the file APIs.
- Keep filesystem storage rooted at `${cwd}/filesystem` so tool artifacts and user files stay co-located.
- Document how a future partial-agent "skills" API could be layered atop the same workspace abstraction once the files endpoint is stable.

### 4. Filesystem & Storage
- Point `distri-filesystem` base paths to `${cwd}/filesystem` (already configurable) and provide helpers for the server to translate relative paths.
- Confirm that object-store adapters can swap in (cloud deployments) without API changes.
- Extend artifact listing/search endpoints so the UI can show raw files side by side with generated artifacts.

### 5. Frontend (distrijs)
- Update Skill Designer / Workspace routes to show **Agents** and **Files** tabs.
- Wire `FileWorkspace.tsx` and `FileWorkspaceWithChat.tsx` to:
  - Load file trees from the new backend endpoints.
  - Cache edits in IndexedDB on every keystroke.
  - Send deltas to the server only when the user hits **Save**, then refresh IndexedDB with the server response.
- Remove UI assumptions about "skills" = plugins; display plugins as part of the files tree when applicable.

### 6. Testing & Validation
- Create fixtures under `examples/` exercising agents, `src/mod.ts`, and plugins simultaneously.
- Add integration tests that set `CURRENT_WORKING_DIR=examples` for both CLI and server flows.
- Verify frontend save flow by mocking IndexedDB writes and ensuring server diffs are applied.

### 7. Rollout & Compatibility
- Provide migration notes for existing users (move `.distri` contents into the new workspace layout, set `CURRENT_WORKING_DIR`).
- Keep telemetry/metrics so we can confirm new endpoints are exercised before removing the old skills APIs entirely.

## Open Questions

1. Do we need a helper to scaffold `src/mod.ts` for users who only have markdown agents?
2. Should `distri-cli` expose a `--workspace` flag in addition to the env variable for scriptability?
3. How do we migrate existing cached artifacts from `.distri/filesystem` to `${cwd}/filesystem` automatically?
4. What metadata/interface will the upcoming "skills = partial agents" feature need so we can align it with the new file endpoints ahead of time?

Capturing answers to these will unblock the implementation workstreams.
