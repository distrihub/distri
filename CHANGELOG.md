# Changelog

## 0.5.0 - 2026-11-22

The distri CLI and distri-server, released together. `curl -fsSL https://distri.dev/install.sh | sh` installs both on macOS and Linux (x86_64, arm64).

- A2A 1.0: the agent card at `/.well-known/agent-card.json`, 1.0 method names; 0.x method names and the pre-1.0 card at `/.well-known/agent.json` still work for older clients (#187, #190).
- Durable runs: runs park on client tools and resume on any worker (#187).
- `distri apply`: set up agents, workflows and a channel from a blueprint folder; samples in `samples/` (#185).
- Sandbox tools in a thread sandbox (#180), MCP Apps (#183), typed decisions (#137), one usage record per LLM call (#172).
- distri-server is API only; the CLI talks to it or to Distri Cloud (#115).
- Release binaries: one workflow builds the CLI and the server natively for darwin-arm64, darwin-amd64, linux-amd64 and linux-arm64, and publishes `v<version>` (install.sh) and `server-v<version>` (`distri serve`).

## 0.1.0 - 2025-11-25 (First public release)

Say hello to Distri — a composable AI agent runtime you can run as a CLI, a server, or embed as an SDK. Learn more at https://distri.dev.

### Installation
```bash
curl -fsSL https://install.distri.dev/download.sh | sh
```
Installer highlights:
- Auto-detects macOS/Linux and arm64/x86_64, fetches the latest `distrihub/distri` release asset, and installs to `/usr/local/bin` or `~/.local/bin`.
- Overrides: `DISTRI_VERSION`, `DISTRI_INSTALL_DIR`, `DISTRI_REPO`.

### Running Distri as a CLI
- Get help: `distri -h`
- Run an agent: `distri run search_agent --task "who is the prime minister of singapore"`
- Explore commands (not exhaustive): `list`, `list-tools`, `run`, `serve`, `toolcall`, `validate`, `generate-prompt`, `generate-response`, `auth`, `build`, `help`.

### Running Distri as a server
```bash
distri serve
```
Starts `distri-server` (API-only by default). Add `--ui` to enable the web interface. Default API endpoint: `http://localhost:8081/v1`.

### Verify & learn more
- Check install: `distri --version`
- Docs: https://distri.dev/docs/
