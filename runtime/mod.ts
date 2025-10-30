export interface AuthRequirement {
  provider: string;
  scopes?: string[];
}

export interface AuthProviderConfig {
  type: string;
  provider: string;
  authorization_url?: string;
  token_url?: string;
  refresh_url?: string;
  scopes?: string[];
  redirect_uri?: string;
  description?: string;
}

export interface ExecutionContext {
  session_id?: string;
  task_id?: string;
  run_id?: string;
  agent_id?: string;
  secrets?: Record<string, string>;
  auth_session?: Record<string, unknown>;
  [key: string]: unknown;
}

export interface DapTool {
  name: string;
  description: string;
  parameters: unknown;
  requiresAuth?: AuthRequirement;
  execute: (parameters: any, context?: ExecutionContext) => Promise<any>;
}

export interface Integration {
  name: string;
  description: string;
  version?: string;
  tools: DapTool[];
  authProvider?: AuthProviderConfig;
  requiresAuth?: AuthRequirement;
  notifications?: string[];
  metadata?: Record<string, unknown>;
}

export interface DapWorkflow {
  name: string;
  description: string;
  version?: string;
  parameters?: unknown;
  examples?: unknown[];
  execute: (parameters: any, context: ExecutionContext) => Promise<any>;
}

export interface DistriPlugin {
  integrations: Integration[];
  workflows: DapWorkflow[];
}

export interface RegisterPluginOptions {
  namespace?: string;
}

type ToolRegistryEntry = {
  integration: string;
  tool: DapTool;
};

type WorkflowRegistryEntry = {
  namespace: string;
  workflow: DapWorkflow;
};

const toolRegistry = new Map<string, ToolRegistryEntry>();
const workflowRegistry = new Map<string, WorkflowRegistryEntry>();

let agentHandler: ((params: CallAgentParams) => Promise<any>) | null = null;

export interface CallAgentParams {
  agent_name: string;
  task: string;
  session_id?: string;
  context?: ExecutionContext;
}

export interface CallToolParams {
  tool_name: string;
  input?: any;
  session_id?: string;
  context?: ExecutionContext;
  integration?: string;
}

export interface CallWorkflowParams {
  workflow_name: string;
  input?: any;
  session_id?: string;
  context?: ExecutionContext;
  namespace?: string;
}

export function createTool(config: {
  name: string;
  description: string;
  parameters: unknown;
  execute: (parameters: any, context?: ExecutionContext) => Promise<any>;
  requiresAuth?: AuthRequirement;
}): DapTool {
  return {
    name: config.name,
    description: config.description,
    parameters: config.parameters,
    requiresAuth: config.requiresAuth,
    execute: config.execute,
  };
}

export function createIntegration(config: {
  name: string;
  description: string;
  version?: string;
  tools: DapTool[];
  authProvider?: AuthProviderConfig;
  requiresAuth?: AuthRequirement;
  notifications?: string[];
  metadata?: Record<string, unknown>;
}): Integration {
  return {
    name: config.name,
    description: config.description,
    version: config.version || "1.0.0",
    tools: config.tools,
    authProvider: config.authProvider,
    requiresAuth: config.requiresAuth,
    notifications: config.notifications || [],
    metadata: config.metadata || {},
  };
}

function normalize(value: string): string {
  return value
    .trim()
    .replace(/\s+/g, "_")
    .replace(/[^a-zA-Z0-9_\.\-]/g, "_")
    .toLowerCase();
}

function registerToolAliases(integrationName: string, tool: DapTool) {
  const aliases = new Set<string>();
  const toolName = normalize(tool.name);
  const integration = normalize(integrationName);

  aliases.add(toolName);
  aliases.add(`${integration}_${toolName}`);
  aliases.add(`${integration}.${toolName}`);

  for (const alias of aliases) {
    toolRegistry.set(alias, {
      integration,
      tool,
    });
  }
}

function registerWorkflowAliases(namespace: string | null, workflow: DapWorkflow) {
  const workflowName = normalize(workflow.name);
  const aliases = new Set<string>([workflowName]);

  if (namespace) {
    const ns = normalize(namespace);
    aliases.add(`${ns}_${workflowName}`);
    aliases.add(`${ns}.${workflowName}`);
  }

  for (const alias of aliases) {
    workflowRegistry.set(alias, {
      namespace: namespace ? normalize(namespace) : "",
      workflow,
    });
  }
}

export function registerPlugin(plugin: DistriPlugin, options: RegisterPluginOptions = {}) {
  const namespace = options.namespace ? normalize(options.namespace) : null;

  for (const integration of plugin.integrations) {
    const integrationName = namespace ? `${namespace}.${integration.name}` : integration.name;
    for (const tool of integration.tools) {
      registerToolAliases(integrationName, tool);
    }
  }

  for (const workflow of plugin.workflows) {
    registerWorkflowAliases(namespace, workflow);
  }
}

export function clearRuntime() {
  toolRegistry.clear();
  workflowRegistry.clear();
}

export function registerAgentHandler(handler: (params: CallAgentParams) => Promise<any>) {
  agentHandler = handler;
}

export async function callAgent(params: CallAgentParams): Promise<any> {
  if (!agentHandler) {
    throw new Error("No agent handler registered. Use registerAgentHandler() to wire callAgent.");
  }

  return await agentHandler(params);
}

function mergeContext(sessionId?: string, context?: ExecutionContext): ExecutionContext {
  return {
    ...(context || {}),
    session_id: sessionId || context?.session_id,
  };
}

function findToolName(params: CallToolParams): ToolRegistryEntry | null {
  const candidates = new Set<string>();
  const requested = params.tool_name ? normalize(params.tool_name) : "";
  if (requested) {
    candidates.add(requested);
  }

  if (params.integration) {
    const integration = normalize(params.integration);
    if (requested) {
      candidates.add(`${integration}_${requested}`);
      candidates.add(`${integration}.${requested}`);
    }
  }

  for (const name of candidates) {
    const entry = toolRegistry.get(name);
    if (entry) {
      return entry;
    }
  }

  return null;
}

export async function callTool(params: CallToolParams): Promise<any> {
  const entry = findToolName(params);
  if (!entry) {
    const attempted = [
      params.tool_name,
      params.integration && `${params.integration}_${params.tool_name}`,
    ]
      .filter(Boolean)
      .join(", ");
    throw new Error(`Tool not registered: ${attempted}`);
  }

  const context = mergeContext(params.session_id, params.context);
  const input = params.input ?? {};
  return await entry.tool.execute(input, context);
}

function findWorkflowName(params: CallWorkflowParams): WorkflowRegistryEntry | null {
  const candidates = new Set<string>();
  const requested = params.workflow_name ? normalize(params.workflow_name) : "";
  if (requested) {
    candidates.add(requested);
  }

  if (params.namespace) {
    const namespace = normalize(params.namespace);
    if (requested) {
      candidates.add(`${namespace}_${requested}`);
      candidates.add(`${namespace}.${requested}`);
    }
  }

  for (const name of candidates) {
    const entry = workflowRegistry.get(name);
    if (entry) {
      return entry;
    }
  }

  return null;
}

export async function callWorkflow(params: CallWorkflowParams): Promise<any> {
  const entry = findWorkflowName(params);
  if (!entry) {
    const attempted = [
      params.workflow_name,
      params.namespace && `${params.namespace}_${params.workflow_name}`,
    ]
      .filter(Boolean)
      .join(", ");
    throw new Error(`Workflow not registered: ${attempted}`);
  }

  const context = mergeContext(params.session_id, params.context);
  const input = params.input ?? {};
  return await entry.workflow.execute(input, context);
}
