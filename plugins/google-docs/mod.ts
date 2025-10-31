import {
  createIntegration,
  createTool,
  DapTool,
  DistriPlugin,
  ExecutionContext,
} from "https://distri.dev/base.ts";

function resolveAccessToken(context?: ExecutionContext) {
  const token = context?.auth_session?.access_token;
  if (!token) {
    throw new Error("Google Docs requires OAuth authentication. Configure auth_session.access_token in the execution context.");
  }

  return token as string;
}

async function createDocument(params: {
  title: string;
  content?: string;
}, context?: ExecutionContext) {
  const accessToken = resolveAccessToken(context);

  const response = await fetch("https://docs.googleapis.com/v1/documents", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${accessToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ title: params.title }),
  });

  if (!response.ok) {
    throw new Error(`Google Docs API error: ${response.status} ${response.statusText}`);
  }

  const doc = await response.json();

  if (params.content) {
    await fetch(`https://docs.googleapis.com/v1/documents/${doc.documentId}:batchUpdate`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${accessToken}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        requests: [
          {
            insertText: {
              location: { index: 1 },
              text: params.content,
            },
          },
        ],
      }),
    });
  }

  return {
    documentId: doc.documentId,
    documentUrl: `https://docs.google.com/document/d/${doc.documentId}/edit`,
  };
}

function getDocsTools(): DapTool[] {
  return [
    createTool({
      name: "create_document",
      description: "Create a Google Doc with optional body content.",
      parameters: {
        type: "object",
        properties: {
          title: { type: "string", description: "Title for the document." },
          content: { type: "string", description: "Optional body text." },
        },
        required: ["title"],
      },
      execute: createDocument,
    }),
  ];
}

const googleDocsPlugin: DistriPlugin = {
  integrations: [
    createIntegration({
      name: "google_docs",
      description: "Google Docs integration for document creation.",
      version: "1.0.0",
      tools: getDocsTools(),
      auth: {
        type: "oauth2",
        provider: "google",
        authorizationUrl: "https://accounts.google.com/o/oauth2/v2/auth",
        tokenUrl: "https://oauth2.googleapis.com/token",
        scopes: ["https://www.googleapis.com/auth/documents"],
      },
      metadata: {
        category: "productivity",
        documentation: "https://developers.google.com/docs/api",
      },
    }),
  ],
  workflows: [],
};

export default googleDocsPlugin;
