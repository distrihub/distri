# Distri Samples

This directory contains sample applications demonstrating various capabilities of the Distri framework.

## Samples Overview

| Sample | Type | Description | Live Demo |
|--------|------|-------------|-----------|
| [maps-demo](./maps-demo) | React/Vite | Interactive Google Maps with AI chat | [View](https://distrihub.github.io/distri/samples/maps) |
| [coder](./coder) | CLI/Rust | Code generation assistant showcasing distri CLI | - |
| [scraper](./scraper) | CLI/Rust | Web scraping and data extraction agent | - |

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
