/**
 * Database Plugin for Distri
 * Integrates PostgreSQL and ClickHouse with connection string authentication
 */

import { DistriPlugin, createTool, DapTool } from "https://distri.dev/base.ts";

// PostgreSQL Integration
interface QueryResult {
    rows: any[];
    rowCount: number;
    fields: Array<{ name: string; dataTypeID: number }>;
}

async function executePostgresQuery(params: {
    query: string;
    parameters?: any[];
    connection_string?: string;
}, context: any): Promise<QueryResult> {
    // Try to get connection string from context secrets or parameters
    const connectionString = params.connection_string || 
                           context?.secrets?.POSTGRES_CONNECTION_STRING ||
                           context?.secrets?.['postgres:default'] ||
                           context?.secrets?.postgres;

    if (!connectionString) {
        throw new Error("PostgreSQL connection string required. Provide via POSTGRES_CONNECTION_STRING secret or connection_string parameter.");
    }

    // Note: In a real implementation, you'd use a proper PostgreSQL client
    // For now, this is a placeholder that shows the structure
    console.log(`Executing PostgreSQL query: ${params.query}`);
    console.log(`Parameters:`, params.parameters);
    console.log(`Connection: ${connectionString.replace(/password=[^;]+/, 'password=***')}`);

    // Placeholder response
    return {
        rows: [],
        rowCount: 0,
        fields: []
    };
}

async function listPostgresTables(params: {
    connection_string?: string;
    schema?: string;
}, context: any): Promise<{ tables: Array<{ table_name: string; table_schema: string }> }> {
    const schema = params.schema || 'public';
    const query = `
        SELECT table_name, table_schema 
        FROM information_schema.tables 
        WHERE table_schema = $1 
        ORDER BY table_name
    `;
    
    const result = await executePostgresQuery({
        query,
        parameters: [schema],
        connection_string: params.connection_string
    }, context);

    return { tables: result.rows };
}

// ClickHouse Integration
async function executeClickHouseQuery(params: {
    query: string;
    format?: string;
    host?: string;
    port?: number;
    username?: string;
    password?: string;
}, context: any): Promise<{ data: any; rows?: number }> {
    // Try to get connection details from context secrets or parameters
    const host = params.host || 
                context?.secrets?.CLICKHOUSE_HOST || 
                context?.secrets?.['clickhouse_host:default'] ||
                'localhost';
                
    const port = params.port || 
                parseInt(context?.secrets?.CLICKHOUSE_PORT || '8123');
                
    const username = params.username || 
                    context?.secrets?.CLICKHOUSE_USERNAME ||
                    context?.secrets?.['clickhouse_username:default'] ||
                    'default';
                    
    const password = params.password || 
                    context?.secrets?.CLICKHOUSE_PASSWORD ||
                    context?.secrets?.['clickhouse_password:default'] ||
                    '';

    const format = params.format || 'JSON';
    
    const url = `http://${host}:${port}/?query=${encodeURIComponent(params.query)}&default_format=${format}`;
    
    const authHeader = username && password ? 
        `Basic ${btoa(`${username}:${password}`)}` : 
        undefined;

    console.log(`Executing ClickHouse query on ${host}:${port}`);
    console.log(`Query: ${params.query}`);

    try {
        const response = await fetch(url, {
            method: 'POST',
            headers: {
                ...(authHeader ? { 'Authorization': authHeader } : {}),
                'Content-Type': 'text/plain'
            }
        });

        if (!response.ok) {
            throw new Error(`ClickHouse error: ${response.status} ${response.statusText}`);
        }

        const data = await response.json();
        return { data, rows: Array.isArray(data) ? data.length : 1 };
    } catch (error) {
        throw new Error(`ClickHouse connection failed: ${error.message}`);
    }
}

async function showClickHouseTables(params: {
    database?: string;
    host?: string;
    port?: number;
    username?: string;
    password?: string;
}, context: any): Promise<{ tables: string[] }> {
    const database = params.database || 'default';
    const query = `SHOW TABLES FROM ${database}`;
    
    const result = await executeClickHouseQuery({
        query,
        host: params.host,
        port: params.port,
        username: params.username,
        password: params.password
    }, context);

    return { tables: result.data?.data?.map((row: any) => row.name) || [] };
}

// Create tools for each integration
function getPostgreSQLTools(): DapTool[] {
    return [
        createTool({
            name: "execute_query",
            description: "Execute a SQL query on PostgreSQL database",
            parameters: {
                type: "object",
                properties: {
                    query: { type: "string", description: "SQL query to execute" },
                    parameters: { 
                        type: "array", 
                        description: "Query parameters for prepared statements (optional)",
                        items: { type: "string" }
                    },
                    connection_string: { type: "string", description: "PostgreSQL connection string (optional if POSTGRES_CONNECTION_STRING secret is set)" }
                },
                required: ["query"]
            },
            execute: executePostgresQuery
        }),
        createTool({
            name: "list_tables",
            description: "List all tables in a PostgreSQL schema",
            parameters: {
                type: "object",
                properties: {
                    schema: { type: "string", description: "Schema name (default: 'public')" },
                    connection_string: { type: "string", description: "PostgreSQL connection string (optional if POSTGRES_CONNECTION_STRING secret is set)" }
                }
            },
            execute: listPostgresTables
        })
    ];
}

function getClickHouseTools(): DapTool[] {
    return [
        createTool({
            name: "execute_query",
            description: "Execute a SQL query on ClickHouse database",
            parameters: {
                type: "object",
                properties: {
                    query: { type: "string", description: "SQL query to execute" },
                    format: { type: "string", description: "Output format (default: JSON)" },
                    host: { type: "string", description: "ClickHouse host (optional if CLICKHOUSE_HOST secret is set)" },
                    port: { type: "number", description: "ClickHouse port (default: 8123)" },
                    username: { type: "string", description: "Username (optional if CLICKHOUSE_USERNAME secret is set)" },
                    password: { type: "string", description: "Password (optional if CLICKHOUSE_PASSWORD secret is set)" }
                },
                required: ["query"]
            },
            execute: executeClickHouseQuery
        }),
        createTool({
            name: "show_tables",
            description: "Show all tables in a ClickHouse database",
            parameters: {
                type: "object",
                properties: {
                    database: { type: "string", description: "Database name (default: 'default')" },
                    host: { type: "string", description: "ClickHouse host (optional if CLICKHOUSE_HOST secret is set)" },
                    port: { type: "number", description: "ClickHouse port (default: 8123)" },
                    username: { type: "string", description: "Username (optional if CLICKHOUSE_USERNAME secret is set)" },
                    password: { type: "string", description: "Password (optional if CLICKHOUSE_PASSWORD secret is set)" }
                }
            },
            execute: showClickHouseTables
        })
    ];
}

// Main plugin export
const plugin: DistriPlugin = {
    integrations: [
        {
            name: 'postgresql',
            description: 'PostgreSQL database integration for SQL queries and operations',
            tools: getPostgreSQLTools(),
        },
        {
            name: 'clickhouse',
            description: 'ClickHouse database integration for analytics queries',
            tools: getClickHouseTools(),
        }
    ],
    workflows: []
};

export default plugin;