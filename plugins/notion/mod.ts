import {
  createIntegration,
  createTool,
  DapTool,
  DistriPlugin,
  ExecutionContext,
} from "https://distri.dev/base.ts";

interface NotionPage {
  id: string;
  title: string;
  url: string;
}

function resolveNotionKey(params: Record<string, unknown>, context?: ExecutionContext) {
  const token = params.api_key as string | undefined;
  if (token) {
    return token;
  }

  const secrets = context?.secrets || {};
  const candidates = [
    "NOTION_API_KEY",
    "notion",
    "notion:default",
    "NOTION_API_KEY:default",
  ];

  for (const key of candidates) {
    if (secrets[key]) {
      return secrets[key];
    }
  }

  throw new Error("Notion API key required. Provide api_key parameter or configure NOTION_API_KEY secret.");
}

async function searchNotionPages(params: {
  query: string;
  api_key?: string;
}, context?: ExecutionContext): Promise<{ pages: NotionPage[] }> {
  const apiKey = resolveNotionKey(params, context);

  const response = await fetch("https://api.notion.com/v1/search", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${apiKey}`,
      "Notion-Version": "2022-06-28",
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ query: params.query }),
  });

  if (!response.ok) {
    throw new Error(`Notion API error: ${response.status} ${response.statusText}`);
  }

  const data = await response.json();
  const pages: NotionPage[] = (data.results || []).map((page: any) => ({
    id: page.id,
    title: page.properties?.title?.title?.[0]?.plain_text || page.properties?.Name?.title?.[0]?.plain_text || "Untitled",
    url: page.url,
  }));

  return { pages };
}

async function createNotionPage(params: {
  parent_database_id?: string;
  parent_page_id?: string;
  title: string;
  content?: string;
  api_key?: string;
}, context?: ExecutionContext): Promise<{ page_id: string; url: string }> {
  const apiKey = resolveNotionKey(params, context);

  if (!params.parent_database_id && !params.parent_page_id) {
    throw new Error("Either parent_database_id or parent_page_id must be provided.");
  }

  const parent = params.parent_database_id
    ? { database_id: params.parent_database_id }
    : { page_id: params.parent_page_id };

  const properties: Record<string, unknown> = params.parent_database_id
    ? {
      Name: {
        title: [{ text: { content: params.title } }],
      },
    }
    : {
      title: {
        title: [{ text: { content: params.title } }],
      },
    };

  const children = params.content
    ? [
      {
        object: "block",
        type: "paragraph",
        paragraph: {
          rich_text: [{ text: { content: params.content } }],
        },
      },
    ]
    : [];

  const response = await fetch("https://api.notion.com/v1/pages", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${apiKey}`,
      "Notion-Version": "2022-06-28",
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      parent,
      properties,
      children,
    }),
  });

  if (!response.ok) {
    throw new Error(`Notion API error: ${response.status} ${response.statusText}`);
  }

  const page = await response.json();
  return {
    page_id: page.id,
    url: page.url,
  };
}

function getNotionTools(): DapTool[] {
  return [
    createTool({
      name: "search_pages",
      description: "Search for pages in a Notion workspace.",
      parameters: {
        type: "object",
        properties: {
          query: { type: "string", description: "Text to search for." },
          api_key: { type: "string", description: "Notion API key override." },
        },
        required: ["query"],
      },
      execute: (params, context) => searchNotionPages(params, context),
    }),
    createTool({
      name: "create_page",
      description: "Create a page in a Notion database or under a parent page.",
      parameters: {
        type: "object",
        properties: {
          parent_database_id: { type: "string", description: "Database ID for the new page." },
          parent_page_id: { type: "string", description: "Parent page ID for the new page." },
          title: { type: "string", description: "Title of the page." },
          content: { type: "string", description: "Optional paragraph content." },
          api_key: { type: "string", description: "Notion API key override." },
        },
        required: ["title"],
      },
      execute: (params, context) => createNotionPage(params, context),
    }),
  ];
}

const notionPlugin: DistriPlugin = {
  integrations: [
    createIntegration({
      name: "notion",
      description: "Notion integration for knowledge management.",
      version: "1.0.0",
      tools: getNotionTools(),
      auth: {
        type: "secret",
        provider: "notion",
        fields: [{ key: "api_key" }],
      },
      metadata: {
        category: "knowledge",
        documentation: "https://developers.notion.com/",
      },
    }),
  ],
  workflows: [],
};

export default notionPlugin;
