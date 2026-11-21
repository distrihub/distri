# docs-agent: the use case

1. `distri apply samples/docs-agent --set site_url=http://localhost:5302`.
2. `cd samples/docs-agent/app && npm install`, put `DISTRI_API_KEY` (and `DISTRI_URL` when not on Distri Cloud) in `app/.env`, then `npx vite`.
3. Open http://localhost:5302 and ask "What happens when I go over the rate limit?".
4. The agent calls `search_docs` and `open_page` (both run in the page): the portal switches to **Rate limits** and the answer says the API returns 429 with a Retry-After header.

`test/run.sh` runs 1–4 with Playwright and saves `test/out/1-portal.png`, `test/out/2-answered.png` and the transcript.
