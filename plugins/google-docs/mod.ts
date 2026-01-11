import {
  createIntegration,
  createTool,
  DapTool,
  DistriPlugin,
  ExecutionContext,
} from "https://distri.dev/base.ts";

function resolveAccessToken(params: Record<string, unknown>, context?: ExecutionContext): string {
  // 1. Check params.access_token
  if (params.access_token) {
    return params.access_token as string;
  }

  // 2. Check secrets
  const secrets = context?.secrets || {};
  const candidates = [
    "google_access_token",
    "google_docs_access_token",
    "google",
    "google_docs",
  ];

  for (const key of candidates) {
    if (secrets[key]) {
      return secrets[key];
    }
  }

  // 3. Check auth_session (OAuth flow)
  if (context?.auth_session?.access_token) {
    return context.auth_session.access_token as string;
  }

  throw new Error("Google Docs requires authentication. Provide access_token parameter, configure google_access_token secret, or use OAuth flow.");
}

async function createDocument(params: {
  title: string;
  content?: string;
  access_token?: string;
}, context?: ExecutionContext) {
  const accessToken = resolveAccessToken(params, context);

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

async function getDocument(params: {
  documentId: string;
  access_token?: string;
}, context?: ExecutionContext) {
  const accessToken = resolveAccessToken(params, context);

  const response = await fetch(`https://docs.googleapis.com/v1/documents/${params.documentId}`, {
    headers: {
      Authorization: `Bearer ${accessToken}`,
      "Content-Type": "application/json",
    },
  });

  if (!response.ok) {
    throw new Error(`Google Docs API error: ${response.status} ${response.statusText}`);
  }

  return await response.json();
}

async function appendToDocument(params: {
  documentId: string;
  content: string;
  access_token?: string;
}, context?: ExecutionContext) {
  const accessToken = resolveAccessToken(params, context);

  // First get the document to find the end index
  const doc = await getDocument({ documentId: params.documentId, access_token: accessToken }, context);
  const endIndex = doc.body?.content?.[doc.body.content.length - 1]?.endIndex || 1;

  const response = await fetch(`https://docs.googleapis.com/v1/documents/${params.documentId}:batchUpdate`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${accessToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      requests: [
        {
          insertText: {
            location: { index: endIndex - 1 },
            text: params.content,
          },
        },
      ],
    }),
  });

  if (!response.ok) {
    throw new Error(`Google Docs API error: ${response.status} ${response.statusText}`);
  }

  return { success: true, documentId: params.documentId };
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
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
        required: ["title"],
      },
      execute: createDocument,
    }),
    createTool({
      name: "get_document",
      description: "Retrieve a Google Doc by ID.",
      parameters: {
        type: "object",
        properties: {
          documentId: { type: "string", description: "The document ID." },
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
        required: ["documentId"],
      },
      execute: getDocument,
    }),
    createTool({
      name: "append_to_document",
      description: "Append text to the end of a Google Doc.",
      parameters: {
        type: "object",
        properties: {
          documentId: { type: "string", description: "The document ID." },
          content: { type: "string", description: "Text to append." },
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
        required: ["documentId", "content"],
      },
      execute: appendToDocument,
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
