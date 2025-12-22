# Distri Releases

![](https://img.shields.io/badge/Platform-macOS%20%7C%20Linux%20%7C%20Windows-blue?style=flat-square) ![](https://img.shields.io/badge/Runtime-Distri%20CLI-orange?style=flat-square)

Distri is A2A compatible agent framework built in Rust. Build agents with simply markdown definition and integrate within your React frontend. Tools can be added as either backend tools via Deno or frontend tools which makes building agents much easier. 

**Learn more in the [official documentation](https://distri.dev/docs/)** and explore product updates at [distri.dev](https://distri.dev/).

Distri is built to work with any A2A agent server (https://a2a-protocol.org/). `distri-server` is one implementation and is available under the Elastic License 2.0 (ELv2).

![Distri CLI demo](https://distri.dev/img/social.png)

## Installation

### Prebuilt binary (recommended)
Use the helper script to grab the latest release for your OS/arch:
```bash
curl -fsSL https://distri.dev/install.sh | sh
```

Pin a specific version or choose an install location (env vars are optional):
```bash
DISTRI_VERSION=0.2.2 DISTRI_INSTALL_DIR=/usr/local/bin sh -c "$(curl -fsSL https://distri.dev/install.sh)"
```

### Direct download (macOS / Linux)
If you prefer a direct download instead of the script, pick the slug for your platform and unpack:
```bash
# darwin-arm64 | darwin-x86_64 | linux-arm64 | linux-x86_64
TARGET=darwin-arm64
curl -L "https://github.com/distrihub/distri/releases/latest/download/distri-${TARGET}.tar.gz" -o distri.tar.gz
sudo tar -xzf distri.tar.gz -C /usr/local/bin distri
```

### Homebrew (macOS / Linux)
```bash
brew tap distrihub/distri
brew install distri
```

### Windows (PowerShell)
```powershell
Invoke-WebRequest https://github.com/distrihub/distri/releases/latest/download/distri-windows-x86_64.zip -OutFile distri.zip
Expand-Archive distri.zip -DestinationPath $Env:LOCALAPPDATA\distri -Force
$Env:Path += ";$Env:LOCALAPPDATA\distri"
```

### Verify & explore
```bash
distri --version
distri help
```

### Run your first workflow
```bash
cd path/to/your/project
distri run
```

## Sample plugins

This repo ships ready-to-run examples that mirror how production Distri plugins are authored:

- `plugins/` – complete integrations and workflows (Slack, Notion, Google, etc.) including their `distri.toml` metadata files.
- `runtime/` – lightweight TypeScript helpers (`registerPlugin`, `createTool`, `callTool`, etc.) that emulate the executor runtime so you can iterate locally without booting the Rust host.
- `docs/` – integration checklists, plugin conventions, and guidance for crafting LLM-friendly workflows.

Clone the repo, open any plugin directory, and run the Deno snippets in the README to experiment, or copy the structure into your own repo when building new integrations.

## Releases & updates

Each tagged release in this repo corresponds to a shipped Distri build. Check the [GitHub Releases](https://github.com/distrihub/distri/releases) page for the latest binaries, changelog notes, and signing artifacts.

## Licensing

Everything in the repository root and common components is licensed under the [MIT License](LICENSE). Anything under `server/` is licensed under the [Elastic License 2.0 – Distri Edition](server/LICENSE). `distri-server` follows [fair-code principles](https://faircode.io/) and will always remain free to use.

## Support & feedback

Questions or ideas? Open an issue in this repository or reach out through [distri.dev/contact](https://distri.dev/contact/). The team actively monitors bug reports and feature requests from the community.
