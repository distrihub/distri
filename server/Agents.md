# Distri Agents Guide

This repository now bundles both core backend services and the DistriJS frontend
stack. Use this guide as the starting point when coding agents need to
familiarise themselves with the merged layout or hand off work between the Rust
and TypeScript projects.

## Monorepo Layout

- `distri-server/`, `distri-stores/`, `distri-types/`, … – Rust backend crates
  that power the API, orchestration runtime, and data stores.
- `distrijs/` – The DistriJS monorepo containing all frontend apps, shared UI,
  and TypeScript client packages (see `distrijs/agents.md` for deep details).
- `agents/` – Prompt packs for autonomous agents that run inside the backend or
  CLI.
- `distri-coder/` – Agent runtime used by `distri` for on-demand code changes.

When in doubt: backend logic (Rust) lives under the top-level `distri-*`
crates, while any React UI, demos, or JS packages now live under `distrijs/`.

## Working With Agents

- Start with the agent specs in `agents/` to understand available personas such
  as `agents/distri.md` (platform orchestrator) and `agents/scripter.md`
  (TypeScript workflow authoring).
- The UI embeds these agents: for example, the Skill Designer route in
  `distrijs/apps/distri-ui` brokers between the frontend workspace and
  `agents/scripter.md`.
- Backend endpoints for agent orchestration, workflow storage, and threads live
  in `distri-server/src/routes.rs`.

## Additional Resources

- `README.md` – Platform overview and setup instructions.
- `distrijs/agents.md` – Frontend-specific tasks, build commands, and
  conventions.
- `notes/` – Scratch documents used during ongoing feature work.

Keep this document updated when the repository structure changes so agents can
quickly find the right subsystem.
