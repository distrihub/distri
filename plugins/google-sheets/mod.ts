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
    throw new Error("Google Sheets requires OAuth authentication. Configure auth_session.access_token in the execution context.");
  }

  return token as string;
}

async function createSpreadsheet(params: {
  title: string;
  sheets?: Array<{ title: string }>;
}, context?: ExecutionContext) {
  const accessToken = resolveAccessToken(context);

  const response = await fetch("https://sheets.googleapis.com/v4/spreadsheets", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${accessToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      properties: { title: params.title },
      sheets: params.sheets || [{ properties: { title: "Sheet1" } }],
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
}, context?: ExecutionContext) {
  const accessToken = resolveAccessToken(context);

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
        },
        required: ["spreadsheetId", "range", "values"],
      },
      execute: writeToSheet,
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
