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
    "google_sheets_access_token",
    "google",
    "google_sheets",
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

  throw new Error("Google Sheets requires authentication. Provide access_token parameter, configure google_access_token secret, or use OAuth flow.");
}

async function createSpreadsheet(params: {
  title: string;
  sheets?: Array<{ title: string }>;
  access_token?: string;
}, context?: ExecutionContext) {
  const accessToken = resolveAccessToken(params, context);

  const response = await fetch("https://sheets.googleapis.com/v4/spreadsheets", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${accessToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      properties: { title: params.title },
      sheets: params.sheets?.map(s => ({ properties: { title: s.title } })) || [{ properties: { title: "Sheet1" } }],
    }),
  });

  if (!response.ok) {
    throw new Error(`Google Sheets API error: ${response.status} ${response.statusText}`);
  }

  const sheet = await response.json();
  return {
    spreadsheetId: sheet.spreadsheetId,
    spreadsheetUrl: sheet.spreadsheetUrl,
  };
}

async function writeToSheet(params: {
  spreadsheetId: string;
  range: string;
  values: string[][];
  access_token?: string;
}, context?: ExecutionContext) {
  const accessToken = resolveAccessToken(params, context);

  const response = await fetch(
    `https://sheets.googleapis.com/v4/spreadsheets/${params.spreadsheetId}/values/${params.range}?valueInputOption=USER_ENTERED`,
    {
      method: "PUT",
      headers: {
        Authorization: `Bearer ${accessToken}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ values: params.values }),
    },
  );

  if (!response.ok) {
    throw new Error(`Google Sheets API error: ${response.status} ${response.statusText}`);
  }

  const result = await response.json();
  return { updatedCells: result.updatedCells || 0 };
}

async function readFromSheet(params: {
  spreadsheetId: string;
  range: string;
  access_token?: string;
}, context?: ExecutionContext) {
  const accessToken = resolveAccessToken(params, context);

  const response = await fetch(
    `https://sheets.googleapis.com/v4/spreadsheets/${params.spreadsheetId}/values/${params.range}`,
    {
      headers: {
        Authorization: `Bearer ${accessToken}`,
        "Content-Type": "application/json",
      },
    },
  );

  if (!response.ok) {
    throw new Error(`Google Sheets API error: ${response.status} ${response.statusText}`);
  }

  const result = await response.json();
  return { values: result.values || [] };
}

async function appendToSheet(params: {
  spreadsheetId: string;
  range: string;
  values: string[][];
  access_token?: string;
}, context?: ExecutionContext) {
  const accessToken = resolveAccessToken(params, context);

  const response = await fetch(
    `https://sheets.googleapis.com/v4/spreadsheets/${params.spreadsheetId}/values/${params.range}:append?valueInputOption=USER_ENTERED&insertDataOption=INSERT_ROWS`,
    {
      method: "POST",
      headers: {
        Authorization: `Bearer ${accessToken}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ values: params.values }),
    },
  );

  if (!response.ok) {
    throw new Error(`Google Sheets API error: ${response.status} ${response.statusText}`);
  }

  const result = await response.json();
  return { updatedRange: result.updates?.updatedRange };
}

function getSheetsTools(): DapTool[] {
  return [
    createTool({
      name: "create_spreadsheet",
      description: "Create a Google Spreadsheet.",
      parameters: {
        type: "object",
        properties: {
          title: { type: "string", description: "Spreadsheet title." },
          sheets: {
            type: "array",
            description: "Optional sheet definitions.",
            items: {
              type: "object",
              properties: {
                title: { type: "string" },
              },
            },
          },
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
        required: ["title"],
      },
      execute: createSpreadsheet,
    }),
    createTool({
      name: "write_to_sheet",
      description: "Write data to a range in Google Sheets.",
      parameters: {
        type: "object",
        properties: {
          spreadsheetId: { type: "string", description: "Spreadsheet ID." },
          range: { type: "string", description: "Target range (e.g. Sheet1!A1:C3)." },
          values: {
            type: "array",
            description: "2D array of cell values.",
            items: {
              type: "array",
              items: { type: "string" },
            },
          },
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
        required: ["spreadsheetId", "range", "values"],
      },
      execute: writeToSheet,
    }),
    createTool({
      name: "read_from_sheet",
      description: "Read data from a range in Google Sheets.",
      parameters: {
        type: "object",
        properties: {
          spreadsheetId: { type: "string", description: "Spreadsheet ID." },
          range: { type: "string", description: "Range to read (e.g. Sheet1!A1:C10)." },
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
        required: ["spreadsheetId", "range"],
      },
      execute: readFromSheet,
    }),
    createTool({
      name: "append_to_sheet",
      description: "Append rows to a Google Sheet.",
      parameters: {
        type: "object",
        properties: {
          spreadsheetId: { type: "string", description: "Spreadsheet ID." },
          range: { type: "string", description: "Range to append to (e.g. Sheet1!A:C)." },
          values: {
            type: "array",
            description: "2D array of row values to append.",
            items: {
              type: "array",
              items: { type: "string" },
            },
          },
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
        required: ["spreadsheetId", "range", "values"],
      },
      execute: appendToSheet,
    }),
  ];
}

const googleSheetsPlugin: DistriPlugin = {
  integrations: [
    createIntegration({
      name: "google_sheets",
      description: "Google Sheets integration for spreadsheet operations.",
      version: "1.0.0",
      tools: getSheetsTools(),
      auth: {
        type: "oauth2",
        provider: "google",
        authorizationUrl: "https://accounts.google.com/o/oauth2/v2/auth",
        tokenUrl: "https://oauth2.googleapis.com/token",
        scopes: ["https://www.googleapis.com/auth/spreadsheets"],
      },
      metadata: {
        category: "productivity",
        documentation: "https://developers.google.com/sheets/api",
      },
    }),
  ],
  workflows: [],
};

export default googleSheetsPlugin;
