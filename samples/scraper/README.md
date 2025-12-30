# Distri Scraper Sample

This sample demonstrates how to use the **Distri Scraper** agent to programmatically scrape websites and extract structured data using the `distri` CLI.

## Prerequisites

- `distri` CLI installed.
- Access to a running Distri server.
- MCP servers enabled (Spider, Search).

## Setup

1. **Start the Distri Server**:
   ```bash
   distri serve
   ```

2. **Push the Agent Definition**:
   Push the scraper agent definition to the server:
   ```bash
   distri agents push agents/scraper.md
   ```

## Usage

Run the scraper agent using the `distri run` command. You can provide a specific task to scrape a website.

**Example 1: Basic Scrape**
```bash
distri run --agent distri-scraper --task "Scrape news from cnn.com and give me the top 5 headlines"
```

**Example 2: Extraction**
```bash
distri run --agent distri-scraper --task "Extract product prices from https://example-store.com and format as JSON"
```
