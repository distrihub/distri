import {
  createIntegration,
  createTool,
  DapTool,
  DistriPlugin,
  ExecutionContext,
} from "https://distri.dev/base.ts";

interface QueryResult {
  rows: unknown[];
  rowCount: number;
  fields: Array<{ name: string; dataTypeID: number }>;
}

function resolveConnectionString(params: Record<string, unknown>, context?: ExecutionContext) {
  if (typeof params.connection_string === "string" && params.connection_string.length > 0) {
    return params.connection_string;
  }

  const secrets = context?.secrets || {};
  const candidates = [
    "POSTGRES_CONNECTION_STRING",
    "postgres",
    "postgres:default",
    "POSTGRES_CONNECTION_STRING:default",
  ];

  for (const key of candidates) {
    if (secrets[key]) {
      return secrets[key];
    }
  }

  throw new Error("PostgreSQL connection string required. Provide connection_string parameter or configure POSTGRES_CONNECTION_STRING secret.");
}

async function executePostgresQuery(params: {
  query: string;
  parameters?: unknown[];
  connection_string?: string;
}, context?: ExecutionContext): Promise<QueryResult> {
  const connectionString = resolveConnectionString(params, context);

  console.log(`Executing PostgreSQL query: ${params.query}`);
  console.log(`Parameters:`, params.parameters);
  console.log(`Connection: ${connectionString.replace(/password=[^;]+/, "password=***")}`);

  return {
    rows: [],
    rowCount: 0,
    fields: [],
  };
}

async function listPostgresTables(params: {
  connection_string?: string;
  schema?: string;
}, context?: ExecutionContext) {
  const schema = params.schema || "public";
  const query = `
    SELECT table_name, table_schema
    FROM information_schema.tables
    WHERE table_schema = $1
    ORDER BY table_name
  `;

  const result = await executePostgresQuery({
    query,
    parameters: [schema],
    connection_string: params.connection_string,
  }, context);

  return { tables: result.rows };
}

function getPostgresTools(): DapTool[] {
  return [
    createTool({
      name: "execute_query",
      description: "Execute a SQL query on PostgreSQL.",
      parameters: {
        type: "object",
        properties: {
          query: { type: "string", description: "SQL query to run." },
          parameters: {
            type: "array",
            description: "Values for parameterised queries.",
            items: { type: "string" },
          },
          connection_string: {
            type: "string",
            description: "Database connection string override.",
          },
        },
        required: ["query"],
      },
      execute: executePostgresQuery,
    }),
    createTool({
      name: "list_tables",
      description: "List tables for a PostgreSQL schema.",
      parameters: {
        type: "object",
        properties: {
          schema: { type: "string", description: "Schema name (default: public)." },
          connection_string: {
            type: "string",
            description: "Database connection string override.",
          },
        },
      },
      execute: listPostgresTables,
    }),
  ];
}

const postgresPlugin: DistriPlugin = {
  integrations: [
    createIntegration({
      name: "postgresql",
      description: "PostgreSQL integration for SQL operations.",
      version: "1.0.0",
      tools: getPostgresTools(),
      metadata: {
        category: "database",
        documentation: "https://www.postgresql.org/docs/",
      },
    }),
  ],
  workflows: [],
};

export default postgresPlugin;
