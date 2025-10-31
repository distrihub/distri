# distri-plugins

Workspace for building and testing distri TypeScript plugins in isolation. Each package follows the conventions used by the main [`blinklogic/distri`](../distri) repository’s `ts-executor`: plugins export a `DistriPlugin` object that exposes integrations (collections of tools) and workflows. This repo adds a Deno-based harness so you can prototype locally without booting the Rust runtime.

## What’s Inside

- `runtime/` – lightweight TypeScript helpers that mirror the executor runtime (`callTool`, `callWorkflow`, `registerPlugin`, `createTool`, `createIntegration`). They emulate the behaviour of `distri-plugin-executor/src/executors/ts_executor/modules/base.ts` by normalising tool names, injecting execution context, and supporting agent callbacks.
- `plugins/` – one directory per integration or workflow package (Slack, Notion, Google, Slack poet demo, etc.). Each package owns its `README.md` and `distri.toml` for discovery metadata.
- `docs/` – guides and reference material, including an LLM-friendly integration checklist.

## Quick Start (Deno)

```ts
import { registerPlugin, registerAgentHandler, callTool } from "https://distri.dev/base.ts";
import slackPlugin from "./plugins/slack/mod.ts";

registerPlugin(slackPlugin);
registerAgentHandler(async ({ task }) => `Echo: ${task}`);

await callTool({
  integration: "slack",
  tool_name: "send_message",
  input: { channel: "#general", text: "Hello distri" },
  context: { secrets: { slack_bot_token: "xoxb-..." } },
});
```

- Tools are registered under multiple aliases (e.g. `slack_send_message`, `slack.send_message`, `send_message`).
- `callWorkflow` works the same way and delegates to the workflow’s `execute` method.
- Provide an agent handler to unblock workflows that invoke `callAgent`.

## Packaging Conventions

1. **Structure** – export `default` as a `DistriPlugin` with `integrations` and `workflows` arrays. Tools are created with `createTool`, workflows with `DapWorkflow` objects.
2. **Metadata** – capture registry-ready information in `distri.toml` (name, tags, tool list, auth hints). This keeps parity with the manifest processing performed by the Rust executor.
3. **Auth & Secrets** – tools read from `context.secrets` and `context.auth_session`, matching the context shape passed by `ts-executor` inside `distri-plugin-executor`.
4. **Logging** – prefer concise `console.log` messages that help during local testing; swap them for `debug!`/structured logging inside the real runtime if needed.

## Learning Resources

- `docs/developing-integrations.md` – step-by-step integration guide tailored for LLM co-pilots, including prompts, checklists, and testing hints.
- Parent repo references:
  - `distri-plugin-executor/src/executors/ts_executor/modules/base.ts` – definitions of `createTool`, `createIntegration`, `callTool`, and plugin processing.
  - `notes/typescript-plugins.md` – canonical structure for TypeScript plugins and guidance on agent configuration.
  - `notes/PLUGIN_ARCHITECTURE.md` – broader overview of plugin loading, validation, and planned WASM component support.

## Contributing Workflow

1. Create a new folder in `plugins/` for your integration or workflow package.
2. Implement tools/workflows with the local runtime helpers.
3. Document behaviour in a package README and capture metadata in `distri.toml`.
4. Run scripts or snippets with Deno to validate your tool invocations.
5. When satisfied, port the module into the main `distri` repository or publish your package.

Maintaining parity with the core executor ensures the code written here drops into production with minimal changes.
