import {
  createIntegration,
  createTool,
  DapTool,
  DistriPlugin,
  ExecutionContext,
} from "jsr:@distri/runtime@0.1.0";

function resolveAccessToken(context?: ExecutionContext) {
  const token = context?.auth_session?.access_token;
  if (!token) {
    throw new Error("Gmail requires OAuth authentication. Configure auth_session.access_token in the execution context.");
  }

  return token as string;
}

async function listEmails(params: {
  query?: string;
  maxResults?: number;
}, context?: ExecutionContext) {
  const accessToken = resolveAccessToken(context);
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
}, context?: ExecutionContext) {
  const accessToken = resolveAccessToken(context);

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
      authProvider: {
        type: "oauth",
        provider: "google",
        authorization_url: "https://accounts.google.com/o/oauth2/v2/auth",
        token_url: "https://oauth2.googleapis.com/token",
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
