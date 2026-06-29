# Changelog

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
