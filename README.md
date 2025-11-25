# Distri Releases

![](https://img.shields.io/badge/Platform-macOS%20%7C%20Linux%20%7C%20Windows-blue?style=flat-square) ![](https://img.shields.io/badge/Runtime-Distri%20CLI-orange?style=flat-square)

Distri is a programmable automation runtime for shipping AI-native workflows across your stack. This repository is the public home for official Distri releases plus a curated set of sample plugins you can use as starting points for your own automations.

**Learn more in the [official documentation](https://distri.dev/docs/)** and explore product updates at [distri.dev](https://distri.dev/).

![Distri CLI demo](https://distri.dev/img/social.png)

## Get started

1. Install the Distri CLI using your preferred method.
2. Verify the install and explore the built-in help.
3. Point Distri at your repo or workspace and start running workflows.

### Homebrew (macOS / Linux)
```bash
brew tap distrihub/distri
brew install distri
```

### Direct download (macOS / Linux)
Download the latest archive from [GitHub Releases](https://github.com/distrihub/distri-public/releases), then:
```bash
curl -L https://github.com/distrihub/distri-public/releases/latest/download/distri-darwin-arm64.tar.gz -o distri.tar.gz   # pick the right OS/arch
sudo tar -xzf distri.tar.gz -C /usr/local/bin distri
```
For Linux `x86_64`, swap the archive for `distri-linux-x86_64.tar.gz` and adjust the extraction path as needed.

### Windows (PowerShell)
```powershell
Invoke-WebRequest https://github.com/distrihub/distri-public/releases/latest/download/distri-windows-x86_64.zip -OutFile distri.zip
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

Each tagged release in this repo corresponds to a shipped Distri build. Check the [GitHub Releases](https://github.com/distrihub/distri-public/releases) page for the latest binaries, changelog notes, and signing artifacts.

## Support & feedback

Questions or ideas? Open an issue in this repository or reach out through [distri.dev/contact](https://distri.dev/contact/). The team actively monitors bug reports and feature requests from the community.
