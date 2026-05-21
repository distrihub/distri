# Distri Samples

This directory contains sample applications demonstrating various capabilities of the Distri framework.

## Samples Overview

| Sample | Type | Description | Live Demo |
|--------|------|-------------|-----------|
| [maps-demo](./maps-demo) | React/Vite | Interactive Google Maps with AI chat | [View](https://distrihub.github.io/distri/samples/maps) |
| [coder](./coder) | CLI/Rust | Code generation assistant showcasing distri CLI | - |
| [scraper](./scraper) | CLI/Rust | Web scraping and data extraction agent | - |
| [hello_webhook.json](./hello_webhook.json) | Workflow agent | `Webhook` trigger — POST `/v1/workflows/webhook/hello` fires a 2-step run | - |
| [heartbeat_schedule.json](./heartbeat_schedule.json) | Workflow agent | `Schedule` trigger — every-minute cron fires from the scheduler tick | - |

## Workflow trigger demos

Push the workflow samples to your workspace:

```bash
distri push samples/hello_webhook.json
distri push samples/heartbeat_schedule.json
```

Then test the webhook (no auth — see the trigger's `auth: none`):

```bash
curl -X POST https://your-cloud-host/v1/workflows/webhook/hello \
  -H 'content-type: application/json' \
  -d '{"hello":"world"}'
# → HTTP 202 { "task_id": "...", "thread_id": "..." }
```

Watch the run via `GET /v1/tasks/{task_id}/resubscribe` (SSE) — you'll see
two `StepCompleted` events. The second `checkpoint` message echoes your
payload back, proving the input flowed through `WorkflowInput` →
`run.context.input` → `{input}` template resolution.

`heartbeat_schedule.json` needs no manual trigger — it auto-fires every
minute via the scheduler tick. Tail the server log for
`💓 heartbeat fired` lines, or watch new task rows appearing on
`/v1/tasks` once a minute. Disable with `DISTRI_SCHEDULER_DISABLE=1`
in the server's env if you don't want it running in dev.

## Quick Start

All samples follow the same workflow:

```bash
# Navigate to sample directory
cd samples/<sample-name>

# Push to Distri Cloud
distri push

# Run tasks
distri run --agent <agent-name> --task "Your task"
```

## Iframe Embedding

React samples are deployed to GitHub Pages and can be embedded as iframes:

```html
<iframe 
  src="https://distrihub.github.io/distri/samples/maps" 
  width="100%" 
  height="600"
  frameborder="0">
</iframe>
```

## Local Development

Each sample includes a README with local development instructions. General pattern:

### React/Vite Samples
```bash
pnpm install
pnpm dev
```

### CLI/Rust Samples
```bash
cargo build
./target/debug/<binary-name> --help
```

## Deploying to Distri Cloud

1. Install the Distri CLI:
   ```bash
   curl -fsSL https://distri.dev/install.sh | bash
   ```

2. Navigate to sample and push:
   ```bash
   cd samples/<sample-name>
   distri push
   ```

## Contributing

Each sample should:
- Include a clear README with setup instructions
- Use `distri push` as the primary deployment method
- Include example tasks demonstrating the agent's capabilities
