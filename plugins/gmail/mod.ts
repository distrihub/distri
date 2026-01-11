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
    "gmail_access_token",
    "google",
    "gmail",
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

  throw new Error("Gmail requires authentication. Provide access_token parameter, configure google_access_token secret, or use OAuth flow.");
}

async function listEmails(params: {
  query?: string;
  maxResults?: number;
  access_token?: string;
}, context?: ExecutionContext) {
  const accessToken = resolveAccessToken(params, context);
  const query = params.query || "";
  const maxResults = params.maxResults ?? 10;

  const response = await fetch(
    `https://gmail.googleapis.com/gmail/v1/users/me/messages?q=${encodeURIComponent(query)}&maxResults=${maxResults}`,
    {
      headers: {
        Authorization: `Bearer ${accessToken}`,
        "Content-Type": "application/json",
      },
    },
  );

  if (!response.ok) {
    throw new Error(`Gmail API error: ${response.status} ${response.statusText}`);
  }

  const data = await response.json();
  return { messages: data.messages || [] };
}

async function sendEmail(params: {
  to: string;
  subject: string;
  body: string;
  from?: string;
  access_token?: string;
}, context?: ExecutionContext) {
  const accessToken = resolveAccessToken(params, context);

  const message = [
    `To: ${params.to}`,
    `Subject: ${params.subject}`,
    params.from ? `From: ${params.from}` : "",
    "",
    params.body,
  ].join("\n");

  const encoded = btoa(message).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

  const response = await fetch("https://gmail.googleapis.com/gmail/v1/users/me/messages/send", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${accessToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ raw: encoded }),
  });

  if (!response.ok) {
    throw new Error(`Gmail API error: ${response.status} ${response.statusText}`);
  }

  const data = await response.json();
  return { messageId: data.id };
}

function getGmailTools(): DapTool[] {
  return [
    createTool({
      name: "list_emails",
      description: "List messages from Gmail matching an optional query.",
      parameters: {
        type: "object",
        properties: {
          query: { type: "string", description: "Gmail search query." },
          maxResults: { type: "number", description: "Maximum results (default: 10)." },
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
      },
      execute: listEmails,
    }),
    createTool({
      name: "send_email",
      description: "Send an email via Gmail.",
      parameters: {
        type: "object",
        properties: {
          to: { type: "string", description: "Recipient email address." },
          subject: { type: "string", description: "Email subject." },
          body: { type: "string", description: "Email body." },
          from: { type: "string", description: "Optional custom from address." },
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
        required: ["to", "subject", "body"],
      },
      execute: sendEmail,
    }),
  ];
}

const gmailPlugin: DistriPlugin = {
  integrations: [
    createIntegration({
      name: "gmail",
      description: "Gmail integration for reading and sending email.",
      version: "1.0.0",
      tools: getGmailTools(),
      auth: {
        type: "oauth2",
        provider: "google",
        authorizationUrl: "https://accounts.google.com/o/oauth2/v2/auth",
        tokenUrl: "https://oauth2.googleapis.com/token",
        scopes: [
          "https://www.googleapis.com/auth/gmail.readonly",
          "https://www.googleapis.com/auth/gmail.send",
        ],
      },
      metadata: {
        category: "communication",
        documentation: "https://developers.google.com/gmail/api",
      },
    }),
  ],
  workflows: [],
};

export default gmailPlugin;
