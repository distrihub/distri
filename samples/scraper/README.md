# Distri Scraper Sample

This sample demonstrates how to use the **Distri Scraper** agent to programmatically scrape websites and extract structured data.

## Quick Start with Distri Cloud

1. **Install Distri CLI**:
   ```bash
   curl -fsSL https://distri.dev/install.sh | bash
   ```

2. **Push the Agent to Distri Cloud**:
   ```bash
   distri push
   ```

3. **Run Scraping Tasks**:
   ```bash
   distri run --agent distri-scraper --task "Extract the top 5 headlines from news.ycombinator.com"
   ```

## Local Development (Optional)

If you prefer to run a local distri server:

1. **Start the Local Server**:
   ```bash
   distri serve
   ```

2. **Push the Agent Locally**:
   ```bash
   distri push --local
   ```

3. **Run Tasks**:
   ```bash
   distri run --agent distri-scraper --task "Your scraping task here"
   ```

## Features

- Web scraping with intelligent content extraction
- Structured data output (JSON, Markdown)
- Search integration via Tavily
- Rate limiting and session management

## Example Tasks

```bash
# Basic headline extraction
distri run --agent distri-scraper --task "Get the top 5 headlines from cnn.com"

# Product scraping
distri run --agent distri-scraper --task "Extract product names and prices from https://example-store.com"

# Research queries
distri run --agent distri-scraper --task "Research recent AI news and summarize top 3 stories"
```

## Configuration

Environment variables (optional):
- `TAVILY_API_KEY`: Enable enhanced search capabilities

The agent configuration is in `definition.yaml`.
