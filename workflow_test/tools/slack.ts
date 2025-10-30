/**
 * Slack Integration for Distri
 * Uses deno.land/x/slack_web_api_client for Slack API interactions
 */

import { SlackAPIClient } from "https://deno.land/x/slack_web_api_client@1.1.7/mod.ts";
import { createTool, DapTool } from "https://distri.dev/base.ts";

interface SlackConfig {
    token?: string;
    defaultChannel?: string;
}

interface SlackMessageOptions {
    channel: string;
    text: string;
    username?: string;
    icon_emoji?: string;
    blocks?: any[];
    attachments?: any[];
}

interface SlackFileUploadOptions {
    channels: string;
    file: Uint8Array | string;
    filename?: string;
    title?: string;
    initial_comment?: string;
}

export class SlackIntegration {
    name = "slack";
    version = "1.0.0";
    description = "Slack integration using official Slack Web API client";

    private client: SlackAPIClient | null = null;
    private config: SlackConfig;

    constructor(config: SlackConfig = {}) {
        this.config = config;
    }

    async initialize(config: SlackConfig): Promise<void> {
        this.config = { ...this.config, ...config };

        // Initialize client if token is available
        const token = this.config.token;
        if (token) {
            try {
                this.client = new SlackAPIClient(token, {
                    throwSlackAPIError: false, // for compatibility and better error handling
                });
                console.log("✅ Slack client initialized successfully");
            } catch (error) {
                console.warn("⚠️  Failed to initialize Slack client:", error.message);
            }
        } else {
            throw new Error("No Slack token provided. Token must be provided through context or parameters.");
        }
    }

    static getTokenFromContext(context: any): string | undefined {
        // Try multiple possible token keys
        return context?.secrets?.SLACK_BOT_TOKEN ||
            context?.secrets?.['SLACK_BOT_TOKEN:default']

    }

    private getClient(): SlackAPIClient {
        if (!this.client) {
            throw new Error("Slack client not initialized. Please provide a valid Slack bot token.");
        }
        return this.client;
    }

    /**
     * Send a message to a Slack channel
     */
    async sendMessage(options: SlackMessageOptions): Promise<any> {
        const client = this.getClient();

        console.log(`🔧 SlackIntegration.sendMessage called with options:`, JSON.stringify(options, null, 2));
        console.log(`🔧 options.text type:`, typeof options.text, `value:`, options.text);

        const textPreview = options.text ? options.text.substring(0, 100) : 'undefined';
        console.log(`📤 Sending Slack message to ${options.channel}: ${textPreview}...`);

        try {
            const response = await client.chat.postMessage({
                channel: options.channel,
                text: options.text,
                username: options.username,
                icon_emoji: options.icon_emoji,
                blocks: options.blocks,
                attachments: options.attachments,
            });

            if (response.ok) {
                console.log("✅ Message sent successfully");
                return response;
            } else {
                throw new Error(`Slack API error: ${response.error}`);
            }
        } catch (error) {
            console.error("❌ Failed to send Slack message:", error.message);
            throw error;
        }
    }

    /**
     * Get information about a user
     */
    async getUserInfo(userId: string): Promise<any> {
        const client = this.getClient();

        try {
            const response = await client.users.info({ user: userId });

            if (response.ok) {
                return response.user;
            } else {
                throw new Error(`Slack API error: ${response.error}`);
            }
        } catch (error) {
            console.error("❌ Failed to get user info:", error.message);
            throw error;
        }
    }

    /**
     * Get information about a channel
     */
    async getChannelInfo(channelId: string): Promise<any> {
        const client = this.getClient();

        try {
            const response = await client.conversations.info({ channel: channelId });

            if (response.ok) {
                return response.channel;
            } else {
                throw new Error(`Slack API error: ${response.error}`);
            }
        } catch (error) {
            console.error("❌ Failed to get channel info:", error.message);
            throw error;
        }
    }

    /**
     * Upload a file to Slack
     */
    async uploadFile(options: SlackFileUploadOptions): Promise<any> {
        const client = this.getClient();

        console.log(`📎 Uploading file to Slack: ${options.filename || 'unnamed file'}`);

        try {
            const response = await client.files.upload({
                channels: options.channels,
                file: options.file,
                filename: options.filename,
                title: options.title,
                initial_comment: options.initial_comment,
            });

            if (response.ok) {
                console.log("✅ File uploaded successfully");
                return response;
            } else {
                throw new Error(`Slack API error: ${response.error}`);
            }
        } catch (error) {
            console.error("❌ Failed to upload file:", error.message);
            throw error;
        }
    }

    /**
     * Update a message
     */
    async updateMessage(channel: string, ts: string, text: string, options?: any): Promise<any> {
        const client = this.getClient();

        try {
            const response = await client.chat.update({
                channel,
                ts,
                text,
                ...options,
            });

            if (response.ok) {
                console.log("✅ Message updated successfully");
                return response;
            } else {
                throw new Error(`Slack API error: ${response.error}`);
            }
        } catch (error) {
            console.error("❌ Failed to update message:", error.message);
            throw error;
        }
    }

    /**
     * Delete a message
     */
    async deleteMessage(channel: string, ts: string): Promise<any> {
        const client = this.getClient();

        try {
            const response = await client.chat.delete({
                channel,
                ts,
            });

            if (response.ok) {
                console.log("✅ Message deleted successfully");
                return response;
            } else {
                throw new Error(`Slack API error: ${response.error}`);
            }
        } catch (error) {
            console.error("❌ Failed to delete message:", error.message);
            throw error;
        }
    }

    /**
     * Test the connection by calling auth.test
     */
    async testConnection(): Promise<any> {
        const client = this.getClient();

        try {
            const response = await client.auth.test();

            if (response.ok) {
                console.log("✅ Slack connection test successful");
                console.log(`   Connected as: ${response.user} (${response.user_id})`);
                console.log(`   Team: ${response.team} (${response.team_id})`);
                return response;
            } else {
                throw new Error(`Slack API error: ${response.error}`);
            }
        } catch (error) {
            console.error("❌ Slack connection test failed:", error.message);
            throw error;
        }
    }
}

// Helper function to initialize Slack client
async function initializeSlack(params: any, context?: any): Promise<SlackIntegration> {
    const token = params.token || SlackIntegration.getTokenFromContext(context);
    if (!token) {
        throw new Error("No Slack token available. Token must be provided in parameters or through authenticated context.");
    }

    const slack = new SlackIntegration({ token });
    await slack.initialize({ token });
    return slack;
}

/**
     * Get all available tools for this integration
     */
export function getTools(): DapTool[] {
    return [
        createTool({
            name: "send_message",
            description: "Send a message to a Slack channel",
            parameters: {
                type: "object",
                properties: {
                    channel: { type: "string", description: "Channel to send message to" },
                    text: { type: "string", description: "Message text" },
                    username: { type: "string", description: "Bot username" },
                    icon_emoji: { type: "string", description: "Bot icon emoji" },
                    token: { type: "string", description: "Slack bot token (optional if SLACK_BOT_TOKEN env var is set)" }
                },
                required: ["channel", "text"]
            },
            execute: async (params: {
                channel: string;
                text: string;
                username?: string;
                icon_emoji?: string;
                token?: string;
                blocks?: any;
                attachments?: any;
            }, context?: any) => {
                console.log(`🔧 sendMessage tool called with params:`, JSON.stringify(params, null, 2));
                console.log(`🔧 params.text:`, params.text, `typeof:`, typeof params.text);
                console.log(`🔧 context:`, context);

                const slack = await initializeSlack(params, context);
                return await slack.sendMessage({
                    channel: params.channel,
                    text: params.text,
                    username: params.username,
                    icon_emoji: params.icon_emoji,
                    blocks: params.blocks,
                    attachments: params.attachments
                });
            }
        }),

        createTool({
            name: "getUserInfo",
            description: "Get information about a Slack user",
            parameters: {
                type: "object",
                properties: {
                    userId: { type: "string", description: "User ID to get info for" },
                    token: { type: "string", description: "Slack bot token (optional if SLACK_BOT_TOKEN env var is set)" }
                },
                required: ["userId"]
            },
            execute: async (params: { userId: string; token?: string }, context?: any) => {
                const slack = await initializeSlack(params, context);
                return await slack.getUserInfo(params.userId);
            }
        }),

        createTool({
            name: "getChannelInfo",
            description: "Get information about a Slack channel",
            parameters: {
                type: "object",
                properties: {
                    channelId: { type: "string", description: "Channel ID to get info for" },
                    token: { type: "string", description: "Slack bot token (optional if SLACK_BOT_TOKEN env var is set)" }
                },
                required: ["channelId"]
            },
            execute: async (params: { channelId: string; token?: string }, context?: any) => {
                const slack = await initializeSlack(params, context);
                return await slack.getChannelInfo(params.channelId);
            }
        }),

        createTool({
            name: "uploadFile",
            description: "Upload a file to Slack",
            parameters: {
                type: "object",
                properties: {
                    channels: { type: "string", description: "Channels to upload to" },
                    file: { description: "File content" },
                    filename: { type: "string", description: "File name" },
                    title: { type: "string", description: "File title" },
                    token: { type: "string", description: "Slack bot token (optional if SLACK_BOT_TOKEN env var is set)" }
                },
                required: ["channels", "file"]
            },
            execute: async (params: {
                channels: string;
                file: any;
                filename?: string;
                title?: string;
                initial_comment?: string;
                token?: string;
            }, context?: any) => {
                const slack = await initializeSlack(params, context);
                return await slack.uploadFile({
                    channels: params.channels,
                    file: params.file,
                    filename: params.filename,
                    title: params.title,
                    initial_comment: params.initial_comment
                });
            }
        }),

        createTool({
            name: "testConnection",
            description: "Test the Slack API connection",
            parameters: {
                type: "object",
                properties: {
                    token: { type: "string", description: "Slack bot token (optional if SLACK_BOT_TOKEN env var is set)" }
                },
                required: []
            },
            execute: async (params: { token?: string }, context?: any) => {
                const slack = await initializeSlack(params, context);
                return await slack.testConnection();
            }
        })
    ];
}

export default {
    getTools
}
