import { SlackAPIClient } from "https://deno.land/x/slack_web_api_client@1.1.7/mod.ts";
import {
  createIntegration,
  createTool,
  DapTool,
  DistriPlugin,
  ExecutionContext,
} from "jsr:@distri/runtime@0.1.0";

interface SlackConfig {
  token?: string;
  defaultChannel?: string;
}

interface SlackMessageOptions {
  channel: string;
  text: string;
  username?: string;
  icon_emoji?: string;
  blocks?: unknown[];
  attachments?: unknown[];
}

interface SlackFileUploadOptions {
  channels: string;
  file: Uint8Array | string;
  filename?: string;
  title?: string;
  initial_comment?: string;
}

class SlackIntegration {
  name = "slack";
  version = "1.0.0";
  description = "Slack integration using the official Slack Web API client";

  private client: SlackAPIClient | null = null;
  private config: SlackConfig;

  constructor(config: SlackConfig = {}) {
    this.config = config;
  }

  async initialize(config: SlackConfig): Promise<void> {
    this.config = { ...this.config, ...config };
    const token = this.config.token;

    if (!token) {
      throw new Error("Slack token missing. Pass a token parameter or configure secrets.");
    }

    try {
      this.client = new SlackAPIClient(token, {
        throwSlackAPIError: false,
      });
    } catch (error) {
      throw new Error(`Failed to initialize Slack client: ${(error as Error).message}`);
    }
  }

  static getTokenFromContext(context?: ExecutionContext): string | undefined {
    const secrets = context?.secrets || {};
    const candidates = [
      "SLACK_BOT_TOKEN",
      "slack",
      "SLACK_BOT_TOKEN:default",
      "slack:default",
    ];

    for (const key of candidates) {
      if (secrets[key]) {
        return secrets[key];
      }
    }

    return undefined;
  }

  private getClient(): SlackAPIClient {
    if (!this.client) {
      throw new Error("Slack client not initialized. Call initialize() with a valid token.");
    }

    return this.client;
  }

  async sendMessage(options: SlackMessageOptions) {
    const client = this.getClient();
    const response = await client.chat.postMessage({
      channel: options.channel,
      text: options.text,
      username: options.username,
      icon_emoji: options.icon_emoji,
      blocks: options.blocks,
      attachments: options.attachments,
    });

    if (!response.ok) {
      throw new Error(`Slack API error: ${response.error}`);
    }

    return response;
  }

  async listChannels() {
    const client = this.getClient();
    const response = await client.conversations.list({
      types: "public_channel,private_channel",
    });

    if (!response.ok) {
      throw new Error(`Slack API error: ${response.error}`);
    }

    return response.channels;
  }

  async getUserInfo(userId: string) {
    const client = this.getClient();
    const response = await client.users.info({ user: userId });

    if (!response.ok) {
      throw new Error(`Slack API error: ${response.error}`);
    }

    return response.user;
  }

  async getChannelInfo(channelId: string) {
    const client = this.getClient();
    const response = await client.conversations.info({ channel: channelId });

    if (!response.ok) {
      throw new Error(`Slack API error: ${response.error}`);
    }

    return response.channel;
  }

  async uploadFile(options: SlackFileUploadOptions) {
    const client = this.getClient();
    const response = await client.files.upload({
      channels: options.channels,
      file: options.file,
      filename: options.filename,
      title: options.title,
      initial_comment: options.initial_comment,
    });

    if (!response.ok) {
      throw new Error(`Slack API error: ${response.error}`);
    }

    return response;
  }

  async updateMessage(channel: string, ts: string, text: string, extra?: Record<string, unknown>) {
    const client = this.getClient();
    const response = await client.chat.update({
      channel,
      ts,
      text,
      ...extra,
    });

    if (!response.ok) {
      throw new Error(`Slack API error: ${response.error}`);
    }

    return response;
  }

  async deleteMessage(channel: string, ts: string) {
    const client = this.getClient();
    const response = await client.chat.delete({ channel, ts });

    if (!response.ok) {
      throw new Error(`Slack API error: ${response.error}`);
    }

    return response;
  }

  async testConnection() {
    const client = this.getClient();
    const response = await client.auth.test();

    if (!response.ok) {
      throw new Error(`Slack API error: ${response.error}`);
    }

    return response;
  }
}

async function initializeSlack(params: Record<string, unknown>, context?: ExecutionContext) {
  const token = (params.token as string | undefined) || SlackIntegration.getTokenFromContext(context);

  if (!token) {
    throw new Error("Slack token missing. Provide token parameter or configure SLACK_BOT_TOKEN secret.");
  }

  const slack = new SlackIntegration({ token });
  await slack.initialize({ token });
  return slack;
}

function getSlackTools(): DapTool[] {
  return [
    createTool({
      name: "send_message",
      description: "Send a message to a Slack channel.",
      parameters: {
        type: "object",
        properties: {
          channel: { type: "string", description: "Target channel ID or name." },
          text: { type: "string", description: "Message text." },
          username: { type: "string", description: "Display name for the bot." },
          icon_emoji: { type: "string", description: "Emoji icon (e.g. :robot_face:)." },
          blocks: { type: "array", description: "Optional Block Kit payload." },
          attachments: { type: "array", description: "Legacy attachments array." },
          token: { type: "string", description: "Override Slack bot token." },
        },
        required: ["channel", "text"],
      },
      execute: async (params, context) => {
        const slack = await initializeSlack(params, context);
        const { token: _token, ...message } = params;
        return await slack.sendMessage(message as SlackMessageOptions);
      },
    }),
    createTool({
      name: "list_channels",
      description: "List available Slack channels the bot can access.",
      parameters: {
        type: "object",
        properties: {
          token: { type: "string", description: "Override Slack bot token." },
        },
      },
      execute: async (params, context) => {
        const slack = await initializeSlack(params, context);
        return await slack.listChannels();
      },
    }),
    createTool({
      name: "get_user_info",
      description: "Fetch profile information for a Slack user.",
      parameters: {
        type: "object",
        properties: {
          user_id: { type: "string", description: "Slack user ID (e.g. U123)." },
          token: { type: "string", description: "Override Slack bot token." },
        },
        required: ["user_id"],
      },
      execute: async (params, context) => {
        const slack = await initializeSlack(params, context);
        return await slack.getUserInfo(params.user_id);
      },
    }),
    createTool({
      name: "get_channel_info",
      description: "Fetch metadata for a Slack channel.",
      parameters: {
        type: "object",
        properties: {
          channel_id: { type: "string", description: "Channel ID (e.g. C123)." },
          token: { type: "string", description: "Override Slack bot token." },
        },
        required: ["channel_id"],
      },
      execute: async (params, context) => {
        const slack = await initializeSlack(params, context);
        return await slack.getChannelInfo(params.channel_id);
      },
    }),
    createTool({
      name: "upload_file",
      description: "Upload a file to Slack.",
      parameters: {
        type: "object",
        properties: {
          channels: { type: "string", description: "Comma separated list of channel IDs." },
          file: { description: "File content as string or Uint8Array." },
          filename: { type: "string", description: "File name, default: generated." },
          title: { type: "string", description: "Display title." },
          initial_comment: { type: "string", description: "Comment posted with the file." },
          token: { type: "string", description: "Override Slack bot token." },
        },
        required: ["channels", "file"],
      },
      execute: async (params, context) => {
        const slack = await initializeSlack(params, context);
        const { token: _token, ...filePayload } = params;
        return await slack.uploadFile(filePayload as SlackFileUploadOptions);
      },
    }),
    createTool({
      name: "update_message",
      description: "Update an existing Slack message.",
      parameters: {
        type: "object",
        properties: {
          channel: { type: "string", description: "Channel ID containing the message." },
          ts: { type: "string", description: "Message timestamp." },
          text: { type: "string", description: "New message text." },
          token: { type: "string", description: "Override Slack bot token." },
          extra: { type: "object", description: "Additional fields forwarded to chat.update." },
        },
        required: ["channel", "ts", "text"],
      },
      execute: async (params, context) => {
        const slack = await initializeSlack(params, context);
        const { token: _token, extra, channel, ts, text } = params;
        return await slack.updateMessage(channel, ts, text, extra);
      },
    }),
    createTool({
      name: "delete_message",
      description: "Delete a Slack message.",
      parameters: {
        type: "object",
        properties: {
          channel: { type: "string", description: "Channel ID containing the message." },
          ts: { type: "string", description: "Message timestamp." },
          token: { type: "string", description: "Override Slack bot token." },
        },
        required: ["channel", "ts"],
      },
      execute: async (params, context) => {
        const slack = await initializeSlack(params, context);
        const { token: _token, channel, ts } = params;
        return await slack.deleteMessage(channel, ts);
      },
    }),
    createTool({
      name: "test_connection",
      description: "Verify the Slack API token is valid via auth.test.",
      parameters: {
        type: "object",
        properties: {
          token: { type: "string", description: "Override Slack bot token." },
        },
      },
      execute: async (params, context) => {
        const slack = await initializeSlack(params, context);
        return await slack.testConnection();
      },
    }),
  ];
}

const slackPlugin: DistriPlugin = {
  integrations: [
    createIntegration({
      name: "slack",
      description: "Slack messaging integration.",
      version: "1.0.0",
      tools: getSlackTools(),
      metadata: {
        category: "messaging",
        documentation: "https://api.slack.com/",
      },
    }),
  ],
  workflows: [],
};

export default slackPlugin;
