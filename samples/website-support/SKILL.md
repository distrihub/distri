---
name: distri-website-support
description: Deploy an onboarding and support agent on the user's website with distri. It answers from their docs and files Linear tickets when it can't help. Use when the user wants a website chat assistant, support bot or onboarding guide.
---

# Deploy the distri website support agent

1. Check `distri --version`; if it is missing, install it with `curl -fsSL https://distri.dev/install.sh | sh`. Sign in with `distri login`, or use `DISTRI_API_KEY`.
2. **Gather from the repository:** the product name (package.json, README), the production origin (deploy config, `NEXT_PUBLIC_SITE_URL`, a CNAME) and the docs URL. Ask the user only for the Linear team key, plus anything you could not find.
3. Run `distri apply website-support --json --set product_name=… --set site_url=… --set docs_url=… --set linear_team_key=…`.
4. Show the user each open `pending` link: **Connect Linear** (required) and **Connect Telegram** (where ticket notices arrive).
5. **Install the widget yourself:** take the `snippet` of the `embed:snippet` pending item and add it to the site's root layout (`app/layout.tsx`, `pages/_document.tsx`, `index.html` or the base template) just before `</body>`. Commit it on a branch.
6. Run `distri apply website-support --wait --test` with the same `--set` values. It files a test ticket and sends the 🎫 notice on Telegram.
7. Tell the user where the widget will show once deployed, and that new tickets arrive in Linear and on Telegram.
