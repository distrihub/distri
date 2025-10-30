# Slack Poet Workflow

Sample workflow that pairs a creative agent with the Slack plugin to deliver poems in real time. It demonstrates how to compose tools using the lightweight Deno runtime.

## Dependencies

- Slack plugin registered in the runtime (provides `send_message`)
- Agent handler capable of fulfilling the `poet_agent` requests

## Usage

```ts
import slackPlugin from "../slack/mod.ts";
import workflowPackage from "./mod.ts";
import { registerPlugin, registerAgentHandler, callWorkflow } from "jsr:@distri/runtime@0.1.0";

registerPlugin(slackPlugin);
registerPlugin(workflowPackage);
registerAgentHandler(async ({ task }) => `Poem about ${task}`);

await callWorkflow({
  workflow_name: "slack_poet",
  input: { message: "coffee in the rain", channel: "#poetry" },
  context: { secrets: { SLACK_BOT_TOKEN: "xoxb-..." } },
});
```
