/**
 * GSuite Plugin for Distri
 * Integrates Gmail, Google Docs, and Google Sheets with OAuth2
 */

import { DistriPlugin, createTool, DapTool } from "https://distri.dev/base.ts";

// Gmail Integration
interface GmailMessage {
    id: string;
    threadId: string;
    snippet: string;
    payload: {
        headers: Array<{ name: string; value: string }>;
    };
}

async function listEmails(params: {
    query?: string;
    maxResults?: number;
}, context: any): Promise<{ messages: GmailMessage[] }> {
    const accessToken = context?.auth_session?.access_token;
    if (!accessToken) {
        throw new Error("Gmail requires OAuth authentication. Please authenticate with Google first.");
    }

    const query = params.query || '';
    const maxResults = params.maxResults || 10;
    
    const url = `https://gmail.googleapis.com/gmail/v1/users/me/messages?q=${encodeURIComponent(query)}&maxResults=${maxResults}`;
    
    const response = await fetch(url, {
        headers: {
            'Authorization': `Bearer ${accessToken}`,
            'Content-Type': 'application/json'
        }
    });

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
}, context: any): Promise<{ messageId: string }> {
    const accessToken = context?.auth_session?.access_token;
    if (!accessToken) {
        throw new Error("Gmail requires OAuth authentication. Please authenticate with Google first.");
    }

    const message = [
        `To: ${params.to}`,
        `Subject: ${params.subject}`,
        params.from ? `From: ${params.from}` : '',
        '',
        params.body
    ].join('\n');

    const encodedMessage = btoa(message).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');

    const response = await fetch('https://gmail.googleapis.com/gmail/v1/users/me/messages/send', {
        method: 'POST',
        headers: {
            'Authorization': `Bearer ${accessToken}`,
            'Content-Type': 'application/json'
        },
        body: JSON.stringify({
            raw: encodedMessage
        })
    });

    if (!response.ok) {
        throw new Error(`Gmail API error: ${response.status} ${response.statusText}`);
    }

    const data = await response.json();
    return { messageId: data.id };
}

// Google Docs Integration
async function createDocument(params: {
    title: string;
    content?: string;
}, context: any): Promise<{ documentId: string; documentUrl: string }> {
    const accessToken = context?.auth_session?.access_token;
    if (!accessToken) {
        throw new Error("Google Docs requires OAuth authentication. Please authenticate with Google first.");
    }

    const response = await fetch('https://docs.googleapis.com/v1/documents', {
        method: 'POST',
        headers: {
            'Authorization': `Bearer ${accessToken}`,
            'Content-Type': 'application/json'
        },
        body: JSON.stringify({
            title: params.title
        })
    });

    if (!response.ok) {
        throw new Error(`Google Docs API error: ${response.status} ${response.statusText}`);
    }

    const doc = await response.json();
    
    // Add content if provided
    if (params.content) {
        await fetch(`https://docs.googleapis.com/v1/documents/${doc.documentId}:batchUpdate`, {
            method: 'POST',
            headers: {
                'Authorization': `Bearer ${accessToken}`,
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({
                requests: [{
                    insertText: {
                        location: { index: 1 },
                        text: params.content
                    }
                }]
            })
        });
    }

    return {
        documentId: doc.documentId,
        documentUrl: `https://docs.google.com/document/d/${doc.documentId}/edit`
    };
}

// Google Sheets Integration
async function createSpreadsheet(params: {
    title: string;
    sheets?: Array<{ title: string }>;
}, context: any): Promise<{ spreadsheetId: string; spreadsheetUrl: string }> {
    const accessToken = context?.auth_session?.access_token;
    if (!accessToken) {
        throw new Error("Google Sheets requires OAuth authentication. Please authenticate with Google first.");
    }

    const response = await fetch('https://sheets.googleapis.com/v4/spreadsheets', {
        method: 'POST',
        headers: {
            'Authorization': `Bearer ${accessToken}`,
            'Content-Type': 'application/json'
        },
        body: JSON.stringify({
            properties: {
                title: params.title
            },
            sheets: params.sheets || [{ properties: { title: 'Sheet1' } }]
        })
    });

    if (!response.ok) {
        throw new Error(`Google Sheets API error: ${response.status} ${response.statusText}`);
    }

    const sheet = await response.json();
    return {
        spreadsheetId: sheet.spreadsheetId,
        spreadsheetUrl: sheet.spreadsheetUrl
    };
}

async function writeToSheet(params: {
    spreadsheetId: string;
    range: string;
    values: string[][];
}, context: any): Promise<{ updatedCells: number }> {
    const accessToken = context?.auth_session?.access_token;
    if (!accessToken) {
        throw new Error("Google Sheets requires OAuth authentication. Please authenticate with Google first.");
    }

    const response = await fetch(
        `https://sheets.googleapis.com/v4/spreadsheets/${params.spreadsheetId}/values/${params.range}?valueInputOption=USER_ENTERED`,
        {
            method: 'PUT',
            headers: {
                'Authorization': `Bearer ${accessToken}`,
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({
                values: params.values
            })
        }
    );

    if (!response.ok) {
        throw new Error(`Google Sheets API error: ${response.status} ${response.statusText}`);
    }

    const result = await response.json();
    return { updatedCells: result.updatedCells || 0 };
}

// Create tools for each integration
function getGmailTools(): DapTool[] {
    return [
        createTool({
            name: "list_emails",
            description: "List emails from Gmail",
            parameters: {
                type: "object",
                properties: {
                    query: { type: "string", description: "Gmail search query (optional)" },
                    maxResults: { type: "number", description: "Maximum number of results (default: 10)" }
                }
            },
            execute: listEmails
        }),
        createTool({
            name: "send_email",
            description: "Send an email via Gmail",
            parameters: {
                type: "object",
                properties: {
                    to: { type: "string", description: "Recipient email address" },
                    subject: { type: "string", description: "Email subject" },
                    body: { type: "string", description: "Email body" },
                    from: { type: "string", description: "Sender email (optional)" }
                },
                required: ["to", "subject", "body"]
            },
            execute: sendEmail
        })
    ];
}

function getGoogleDocsTools(): DapTool[] {
    return [
        createTool({
            name: "create_document",
            description: "Create a new Google Doc",
            parameters: {
                type: "object",
                properties: {
                    title: { type: "string", description: "Document title" },
                    content: { type: "string", description: "Initial document content (optional)" }
                },
                required: ["title"]
            },
            execute: createDocument
        })
    ];
}

function getGoogleSheetsTools(): DapTool[] {
    return [
        createTool({
            name: "create_spreadsheet",
            description: "Create a new Google Spreadsheet",
            parameters: {
                type: "object",
                properties: {
                    title: { type: "string", description: "Spreadsheet title" },
                    sheets: { 
                        type: "array", 
                        description: "Sheet definitions (optional)",
                        items: { 
                            type: "object",
                            properties: {
                                title: { type: "string" }
                            }
                        }
                    }
                },
                required: ["title"]
            },
            execute: createSpreadsheet
        }),
        createTool({
            name: "write_to_sheet",
            description: "Write data to a Google Spreadsheet",
            parameters: {
                type: "object",
                properties: {
                    spreadsheetId: { type: "string", description: "Spreadsheet ID" },
                    range: { type: "string", description: "Cell range (e.g., 'A1:C3')" },
                    values: { 
                        type: "array", 
                        description: "2D array of values to write",
                        items: {
                            type: "array",
                            items: { type: "string" }
                        }
                    }
                },
                required: ["spreadsheetId", "range", "values"]
            },
            execute: writeToSheet
        })
    ];
}

// Main plugin export
const plugin: DistriPlugin = {
    integrations: [
        {
            name: 'gmail',
            description: 'Gmail integration for reading and sending emails',
            tools: getGmailTools(),
            authProvider: {
                type: 'oauth',
                provider: 'google',
                authorization_url: 'https://accounts.google.com/o/oauth2/v2/auth',
                token_url: 'https://oauth2.googleapis.com/token',
                scopes: ['https://www.googleapis.com/auth/gmail.readonly', 'https://www.googleapis.com/auth/gmail.send']
            }
        },
        {
            name: 'google_docs',
            description: 'Google Docs integration for creating and editing documents',
            tools: getGoogleDocsTools(),
            authProvider: {
                type: 'oauth',
                provider: 'google',
                authorization_url: 'https://accounts.google.com/o/oauth2/v2/auth',
                token_url: 'https://oauth2.googleapis.com/token',
                scopes: ['https://www.googleapis.com/auth/documents']
            }
        },
        {
            name: 'google_sheets',
            description: 'Google Sheets integration for spreadsheet operations',
            tools: getGoogleSheetsTools(),
            authProvider: {
                type: 'oauth',
                provider: 'google',
                authorization_url: 'https://accounts.google.com/o/oauth2/v2/auth',
                token_url: 'https://oauth2.googleapis.com/token',
                scopes: ['https://www.googleapis.com/auth/spreadsheets']
            }
        }
    ],
    workflows: []
};

export default plugin;