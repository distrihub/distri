/**
 * Business Tools Plugin for Distri
 * Integrates Slack and Notion for workplace productivity
 */

import { SlackAPIClient } from "https://deno.land/x/slack_web_api_client@1.1.7/mod.ts";
import { DistriPlugin, createTool, DapTool } from "https://distri.dev/base.ts";

// Slack Integration (reused from existing implementation)
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

class SlackIntegration {
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

        const token = this.config.token;
        if (token) {
            try {
                this.client = new SlackAPIClient(token, {
                    throwSlackAPIError: false,
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
        return context?.secrets?.SLACK_BOT_TOKEN || 
               context?.secrets?.slack || 
               context?.secrets?.['SLACK_BOT_TOKEN:default'] ||
               context?.secrets?.['slack:default'];
    }

    private getClient(): SlackAPIClient {
        if (!this.client) {
            throw new Error("Slack client not initialized. Please provide a valid Slack bot token.");
        }
        return this.client;
    }

    async sendMessage(options: SlackMessageOptions): Promise<any> {
        const client = this.getClient();
        
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
    }

    async listChannels(): Promise<any> {
        const client = this.getClient();
        
        const response = await client.conversations.list({
            types: 'public_channel,private_channel'
        });

        if (response.ok) {
            return response.channels;
        } else {
            throw new Error(`Slack API error: ${response.error}`);
        }
    }
}

// Notion Integration
interface NotionPage {
    id: string;
    title: string;
    url: string;
}

async function searchNotionPages(params: {
    query: string;
    api_key?: string;
}, context: any): Promise<{ pages: NotionPage[] }> {
    const apiKey = params.api_key || 
                  context?.secrets?.NOTION_API_KEY ||
                  context?.secrets?.['notion:default'] ||
                  context?.secrets?.notion;

    if (!apiKey) {
        throw new Error("Notion API key required. Provide via NOTION_API_KEY secret or api_key parameter.");
    }

    const response = await fetch('https://api.notion.com/v1/search', {
        method: 'POST',
        headers: {
            'Authorization': `Bearer ${apiKey}`,
            'Notion-Version': '2022-06-28',
            'Content-Type': 'application/json'
        },
        body: JSON.stringify({
            query: params.query
        })
    });

    if (!response.ok) {
        throw new Error(`Notion API error: ${response.status} ${response.statusText}`);
    }

    const data = await response.json();
    const pages = data.results.map((page: any) => ({
        id: page.id,
        title: page.properties?.title?.title?.[0]?.plain_text || 'Untitled',
        url: page.url
    }));

    return { pages };
}

async function createNotionPage(params: {
    parent_database_id?: string;
    parent_page_id?: string;
    title: string;
    content?: string;
    api_key?: string;
}, context: any): Promise<{ page_id: string; url: string }> {
    const apiKey = params.api_key || 
                  context?.secrets?.NOTION_API_KEY ||
                  context?.secrets?.['notion:default'] ||
                  context?.secrets?.notion;

    if (!apiKey) {
        throw new Error("Notion API key required. Provide via NOTION_API_KEY secret or api_key parameter.");
    }

    if (!params.parent_database_id && !params.parent_page_id) {
        throw new Error("Either parent_database_id or parent_page_id must be provided");
    }

    const parent = params.parent_database_id ? 
        { database_id: params.parent_database_id } : 
        { page_id: params.parent_page_id };

    const properties: any = {};
    if (params.parent_database_id) {
        // For database pages, assume there's a title property
        properties.Name = {
            title: [{ text: { content: params.title } }]
        };
    } else {
        // For regular pages
        properties.title = {
            title: [{ text: { content: params.title } }]
        };
    }

    const children = params.content ? [{
        object: 'block',
        type: 'paragraph',
        paragraph: {
            rich_text: [{ text: { content: params.content } }]
        }
    }] : [];

    const response = await fetch('https://api.notion.com/v1/pages', {
        method: 'POST',
        headers: {
            'Authorization': `Bearer ${apiKey}`,
            'Notion-Version': '2022-06-28',
            'Content-Type': 'application/json'
        },
        body: JSON.stringify({
            parent,
            properties,
            children
        })
    });

    if (!response.ok) {
        throw new Error(`Notion API error: ${response.status} ${response.statusText}`);
    }

    const page = await response.json();
    return {
        page_id: page.id,
        url: page.url
    };
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

// Create tools for each integration
function getSlackTools(): DapTool[] {
    return [
        createTool({
            name: "send_message",
            description: "Send a message to a Slack channel",
            parameters: {
                type: "object",
                properties: {
                    channel: { type: "string", description: "Channel to send message to" },
                    text: { type: "string", description: "Message text" },
                    username: { type: "string", description: "Bot username (optional)" },
                    icon_emoji: { type: "string", description: "Bot icon emoji (optional)" },
                    token: { type: "string", description: "Slack bot token (optional if SLACK_BOT_TOKEN secret is set)" }
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
            name: "list_channels",
            description: "List all Slack channels",
            parameters: {
                type: "object",
                properties: {
                    token: { type: "string", description: "Slack bot token (optional if SLACK_BOT_TOKEN secret is set)" }
                }
            },
            execute: async (params: { token?: string }, context?: any) => {
                const slack = await initializeSlack(params, context);
                return await slack.listChannels();
            }
        })
    ];
}

function getNotionTools(): DapTool[] {
    return [
        createTool({
            name: "search_pages",
            description: "Search for pages in Notion",
            parameters: {
                type: "object",
                properties: {
                    query: { type: "string", description: "Search query" },
                    api_key: { type: "string", description: "Notion API key (optional if NOTION_API_KEY secret is set)" }
                },
                required: ["query"]
            },
            execute: searchNotionPages
        }),
        createTool({
            name: "create_page",
            description: "Create a new page in Notion",
            parameters: {
                type: "object",
                properties: {
                    parent_database_id: { type: "string", description: "Parent database ID (use this OR parent_page_id)" },
                    parent_page_id: { type: "string", description: "Parent page ID (use this OR parent_database_id)" },
                    title: { type: "string", description: "Page title" },
                    content: { type: "string", description: "Initial page content (optional)" },
                    api_key: { type: "string", description: "Notion API key (optional if NOTION_API_KEY secret is set)" }
                },
                required: ["title"]
            },
            execute: createNotionPage
        })
    ];
}

// Main plugin export
const plugin: DistriPlugin = {
    integrations: [
        {
            name: 'slack',
            description: 'Slack integration for team communication and messaging',
            tools: getSlackTools(),
        },
        {
            name: 'notion',
            description: 'Notion integration for knowledge management and documentation',
            tools: getNotionTools(),
        }
    ],
    workflows: []
};

export default plugin;