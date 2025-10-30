import {
  createIntegration,
  createTool,
  DapTool,
  DistriPlugin,
  ExecutionContext,
} from "jsr:@distri/runtime@0.1.0";

interface ClickHouseQueryParams {
  query: string;
  format?: string;
  host?: string;
  port?: number;
  username?: string;
  password?: string;
}

function resolveClickHouseConfig(params: ClickHouseQueryParams, context?: ExecutionContext) {
  const secrets = context?.secrets || {};

  const host = params.host
    || secrets.CLICKHOUSE_HOST
    || secrets["clickhouse_host:default"]
    || "localhost";

  const port = params.port
    || (secrets.CLICKHOUSE_PORT ? parseInt(secrets.CLICKHOUSE_PORT, 10) : undefined)
    || 8123;

  const username = params.username
    || secrets.CLICKHOUSE_USERNAME
    || secrets["clickhouse_username:default"]
    || "default";

  const password = params.password
    || secrets.CLICKHOUSE_PASSWORD
    || secrets["clickhouse_password:default"]
    || "";

  return { host, port, username, password };
}

async function executeClickHouseQuery(params: ClickHouseQueryParams, context?: ExecutionContext) {
  if (!params.query) {
    throw new Error("ClickHouse query is required.");
  }

  const { host, port, username, password } = resolveClickHouseConfig(params, context);
  const format = params.format || "JSON";
  const url = `http://${host}:${port}/?query=${encodeURIComponent(params.query)}&default_format=${format}`;

  const headers: Record<string, string> = {
    "Content-Type": "text/plain",
  };

  if (username) {
    headers.Authorization = `Basic ${btoa(`${username}:${password}`)}`;
  }

  console.log(`Executing ClickHouse query on ${host}:${port}`);
  console.log(`Query: ${params.query}`);

  const response = await fetch(url, {
    method: "POST",
    headers,
  });

  if (!response.ok) {
    throw new Error(`ClickHouse error: ${response.status} ${response.statusText}`);
  }

  const data = await response.json();
  return {
    data,
    rows: Array.isArray(data?.data) ? data.data.length : undefined,
  };
}

async function showClickHouseTables(params: {
  database?: string;
  host?: string;
  port?: number;
  username?: string;
  password?: string;
}, context?: ExecutionContext) {
  const database = params.database || "default";
  const query = `SHOW TABLES FROM ${database}`;
  const result = await executeClickHouseQuery({
    query,
    host: params.host,
    port: params.port,
    username: params.username,
    password: params.password,
  }, context);

  return {
    tables: Array.isArray(result.data?.data) ? result.data.data.map((row: any) => row.name) : [],
  };
}

function getClickHouseTools(): DapTool[] {
  return [
    createTool({
      name: "execute_query",
      description: "Execute a SQL query on ClickHouse.",
      parameters: {
        type: "object",
        properties: {
          query: { type: "string", description: "SQL query to run." },
          format: { type: "string", description: "Response format (default: JSON)." },
          host: { type: "string", description: "ClickHouse host override." },
          port: { type: "number", description: "ClickHouse port override." },
          username: { type: "string", description: "Username override." },
          password: { type: "string", description: "Password override." },
        },
        required: ["query"],
      },
      execute: executeClickHouseQuery,
    }),
    createTool({
      name: "show_tables",
      description: "List tables in a ClickHouse database.",
      parameters: {
        type: "object",
        properties: {
          database: { type: "string", description: "Database name (default: default)." },
          host: { type: "string", description: "ClickHouse host override." },
          port: { type: "number", description: "ClickHouse port override." },
          username: { type: "string", description: "Username override." },
          password: { type: "string", description: "Password override." },
        },
      },
      execute: showClickHouseTables,
    }),
  ];
}

const clickhousePlugin: DistriPlugin = {
  integrations: [
    createIntegration({
      name: "clickhouse",
      description: "ClickHouse integration for analytics queries.",
      version: "1.0.0",
      tools: getClickHouseTools(),
      metadata: {
        category: "database",
        documentation: "https://clickhouse.com/docs",
      },
    }),
  ],
  workflows: [],
};

export default clickhousePlugin;
