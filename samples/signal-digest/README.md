# Signal digest

> **"Every weekday at 8am, send me a Telegram digest of the posts worth reading about AI agents for SRE on Hacker News and Reddit."**

## What it sets up

| What | Name | Notes |
|---|---|---|
| Workflow | `signal_digest` | Weekday schedule. A **TypeScript code step** ([`tools/fetch_posts.ts`](tools/fetch_posts.ts)) fetches new posts, `digest_writer` picks the good ones, and the digest goes to you on Telegram. Links already sent are kept in the workflow's state and never repeated. |
| Agent | `digest_writer` | Ranks the posts and writes the digest. No tools. |
| Chat | Telegram | Where the digest arrives. |

The fetch step is a `@distri/tool` module run with Deno. It may only reach `hn.algolia.com` and `www.reddit.com`, as declared in its frontmatter. On a self-hosted distri server it runs with the server's Deno; on distri cloud it runs in a sandbox.

## Run it

```bash
distri apply samples/signal-digest --set topic="AI agents for SRE" --set timezone=Europe/Lisbon --wait --test
```

Try the code step on its own:

```bash
echo '{"topic":"AI agents"}' | deno run --allow-net=hn.algolia.com,www.reddit.com --allow-read \
  --import-map=<(echo '{"imports":{"jsr:@distri/tool":"../../distri-tool/mod.ts"}}') \
  ../../distri-tool/run.ts ./tools/fetch_posts.ts
```
