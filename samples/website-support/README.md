# Website onboarding and support agent

> **"Create an onboarding agent for my product on example.com that answers questions from our docs and opens a support ticket when it can't help."**

## What it sets up

| What | Name | Notes |
|---|---|---|
| Agent | `support_guide` | Web agent embedded on your site. Answers from your docs. Calls `create_support_ticket` when it can't help. |
| Workflow | `create_support_ticket` | Called **as a tool** by the website agent (`tools.workflows`). Runs on the server with your Linear connection, files the issue (a TypeScript code step) and tells you on Telegram. If Linear fails, you still get the request. |
| Agent | `support_inbox` | The Telegram chat where ticket notices arrive. |
| Embed | public client | Allowed origin `site_url`. Apply prints the script tag. |
| Connection | Linear | Required; tickets go to the team `linear_team_key`. |

Visitors never see a credential: the website agent only calls the workflow, and the workflow uses the workspace's Linear connection on the server.

## Run it

```bash
distri apply samples/website-support \
  --set product_name=Acme --set site_url=https://acme.com \
  --set docs_url=https://acme.com/docs --set linear_team_key=SUP --wait --test
```

Then paste the snippet it prints into your site, before `</body>`. For the React component instead, see [`distri-cloud/samples/restaurant-demo`](https://github.com/distrihub/distri-cloud/tree/main/samples/restaurant-demo).
