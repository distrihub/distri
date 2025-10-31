import {
  callAgent,
  callTool,
  DapWorkflow,
  DistriPlugin,
} from "https://distri.dev/base.ts";

async function run(input: any, context: any) {
  const channel = input.channel || "#poetry";
  const poetPrompt = `Write a thoughtful poem inspired by: "${input.message}". Use imagery and keep it concise.`;

  const poem = await callAgent({
    agent_name: "poet_agent",
    task: poetPrompt,
    session_id: context.session_id,
    context,
  });

  const slackResponse = await callTool({
    tool_name: "send_message",
    integration: "slack",
    input: {
      channel,
      text: poem,
      username: "Poetry Bot",
      icon_emoji: ":scroll:",
      token: input.slackToken,
    },
    session_id: context.session_id,
    context,
  });

  return {
    success: true,
    poem,
    slack_response: slackResponse,
  };
}

const slackPoetWorkflow: DapWorkflow = {
  name: "slack_poet",
  description: "Generate a poem with an agent and send it to Slack.",
  version: "1.0.0",
  parameters: {
    type: "object",
    properties: {
      message: {
        type: "string",
        description: "Message to turn into a poem.",
      },
      channel: {
        type: "string",
        description: "Slack channel (default: #poetry).",
      },
      slackToken: {
        type: "string",
        description: "Optional Slack bot token override.",
      },
    },
    required: ["message"],
  },
  examples: [
    {
      description: "Generate a poem about coffee",
      input: {
        message: "coffee in the morning",
        channel: "#poetry",
      },
      expected_output: "Poem delivered to Slack",
    },
    {
      description: "Custom channel",
      input: {
        message: "sunset over mountains",
        channel: "#general",
      },
      expected_output: "Poem delivered to #general",
    },
  ],
  execute: async (params, context) => run(params, context),
};

const workflowPackage: DistriPlugin = {
  integrations: [],
  workflows: [slackPoetWorkflow],
};

export default workflowPackage;
