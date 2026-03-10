# Default Agents

## fast_search — Quick lookups

```bash
distri-server run fast_search --task "What is the population of Tokyo?"
```

## search — Search + scrape

```bash
distri-server run search --task "What are the top 3 programming languages in 2026?"
```

## web — Browser automation + scraping

```bash
distri-server run web --task "Scrape https://news.ycombinator.com and extract the top 5 story titles and links"
```

## code — Sandboxed code execution

```bash
distri-server run code --task "Calculate the sum of all prime numbers below 1000 using Python"
```

## deepresearch — Multi-phase research with sub-agent delegation

```bash
distri-server run deepresearch --task "Research the current state of quantum computing. What are the top 3 companies and their latest breakthroughs?"
```

## distri — Master orchestrator

```bash
distri-server run distri --task "Find the latest SpaceX launch date and calculate how many days from now"
```

## agent_designer — Design new agents

```bash
distri-server run agent_designer --task "Design an agent that monitors stock prices and sends alerts when they cross a threshold"
```

## Environment

Requires `BROWSR_BASE_URL` and `BROWSR_API_KEY` for search, scrape, browser, and shell tools.
