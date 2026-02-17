// Integration interface - a group of tools with shared functionality
export interface Integration {
    name: string;
    description: string;
    version?: string;
    tools: DistriTool[];  // Tools only available at integration level
    callbacks?: any;   // Input schema for callbacks (JSON schema)
    auth?: AuthRequirement;
    notifications?: string[];
    metadata?: { [key: string]: any };
}

// DistriPlugin interface - plugins expose integrations and workflows
export interface DistriPlugin {
    integrations: Integration[];  // Required - tools only exist in integrations
    workflows?: DistriWorkflow[];
}

// Part types for rich content support
export interface TextPart {
    part_type: 'text';
    data: string;
}

export interface DataPart {
    part_type: 'data';
    data: any;
}

export interface ImagePart {
    part_type: 'image';
    data: {
        type: 'bytes' | 'url';
        bytes?: string; // base64 encoded
        url?: string;
        mime_type: string;
        name?: string;
    };
}

export interface ArtifactPart {
    part_type: 'artifact';
    data: {
        file_id: string;
        size: number;
        preview?: string;
        mime_type?: string;
        path?: string;
        content_type?: string;
    };
}

export interface ToolCallPart {
    part_type: 'tool_call';
    data: ToolCall;
}

export interface ToolResultPart {
    part_type: 'tool_result';
    data: {
        tool_call_id: string;
        tool_name: string;
        parts: Part[];
    };
}

export type Part = TextPart | DataPart | ImagePart | ArtifactPart | ToolCallPart | ToolResultPart;

// Helper functions for creating parts
export function createTextPart(text: string): TextPart {
    return { part_type: 'text', data: text };
}

export function createDataPart(data: any): DataPart {
    return { part_type: 'data', data };
}

export function createImagePart(image: ImagePart['data']): ImagePart {
    return { part_type: 'image', data: image };
}

export function createArtifactPart(artifact: ArtifactPart['data']): ArtifactPart {
    return { part_type: 'artifact', data: artifact };
}

export interface DistriTool {
    name: string;
    description: string;
    version?: string;
    // Support both old (any) and new (Part[]) return types for backward compatibility
    execute(parameters: any, context: any): Promise<any | Part[]>;
    parameters: any;
    auth?: AuthRequirement;
    integrationName?: string;
}

export interface DistriWorkflow {
    name: string;
    description: string;
    version?: string;
    execute(params: any, context: any): Promise<any>;
    parameters: any;
    examples: any[];
}

export interface Context {
    callId: string;
    call_id?: string;
    agentId: string;
    agent_id?: string;
    sessionId: string;
    session_id?: string;
    taskId: string;
    task_id?: string;
    runId: string;
    run_id?: string;
    params: any;
    secrets: { [key: string]: string };  // Environment secrets and API keys
    env_vars?: { [key: string]: string };  // Environment variables from client
    userId?: string;
    user_id?: string;
}

export interface ToolCall {
    tool_call_id: string;
    tool_name: string;
    parameters: any;
}

export interface WorkflowCall {
    workflow_call_id: string;
    workflow_name: string;
    input: any;
}

// Workflow function parameter types
export interface CallAgentParams {
    agent_name: string;
    task: string;
    session_id: string;
}

export interface CallToolParams {
    session_id: string;
    user_id?: string;
    package_name?: string;
    tool_name: string;
    input: any;  // Now supports JSON object values
}

export interface GetSessionValueParams {
    session_id: string;
    key: string;
}

export interface SetSessionValueParams {
    session_id: string;
    key: string;
    value: any;
}

export interface WorkflowResponse {
    success: boolean;
    result: any;
    error?: string;
}


export interface OAuth2Requirement {
    type: 'oauth2';
    provider: string;
    scopes?: string[];
    authorizationUrl?: string;
    tokenUrl?: string;
    refreshUrl?: string;
    sendRedirectUri?: boolean;
}

export interface SecretFieldRequirement {
    key: string;
    label?: string;
    description?: string;
    optional?: boolean;
}

export interface SecretRequirement {
    type: 'secret';
    provider: string;
    fields?: SecretFieldRequirement[];
}

export type AuthRequirement = OAuth2Requirement | SecretRequirement;



export function createTool(config: {
    name: string;
    description: string;
    parameters: any;
    execute: (parameters: any, context?: any) => Promise<any>;
    auth?: AuthRequirement;
}): DistriTool {
    return {
        name: config.name,
        description: config.description,
        parameters: config.parameters,
        auth: config.auth,
        execute: config.execute
    };
}

export function createIntegration(config: {
    name: string;
    description: string;
    version?: string;
    tools: DistriTool[];
    auth?: AuthRequirement;
    notifications?: string[];
    metadata?: { [key: string]: any };
}): Integration {
    return {
        name: config.name,
        description: config.description,
        version: config.version || "1.0.0",
        tools: config.tools,
        auth: config.auth,
        notifications: config.notifications || [],
        metadata: config.metadata || {}
    };
}

// Workflow runtime functions - these will be injected by the plugin system
declare global {
    function callAgent(params: CallAgentParams): Promise<string>;
    function callTool(params: CallToolParams): Promise<any>;
    function getSessionValue(params: GetSessionValueParams): Promise<any>;
    function setSessionValue(params: SetSessionValueParams): Promise<void>;
}

export async function loadPlugin(plugin_name: string): Promise<any> {
    console.log("About to import:", `./plugin_${plugin_name}.ts`);
    const pluginModule = await import(`./plugin_${plugin_name}.ts`);
    console.log("Import successful!");
    return pluginModule.default || pluginModule;
}

// Make these functions available to the plugin system
export const callAgent = globalThis.rustyscript.async_functions.callAgent as (params: CallAgentParams) => Promise<string>;
export const callTool = globalThis.rustyscript.async_functions.callTool as (params: CallToolParams) => Promise<any>;
export const getSessionValue = globalThis.rustyscript.async_functions.getSessionValue as (params: GetSessionValueParams) => Promise<any>;
export const setSessionValue = globalThis.rustyscript.async_functions.setSessionValue as (params: SetSessionValueParams) => Promise<void>;

/**
 * Process a DistriPlugin to normalize tools and workflows for consistent Rust parsing
 */
export function processPlugin(plugin: DistriPlugin): any {
    const processedIntegrations: any[] = [];

    const normalizeAuth = (value: any): AuthRequirement | undefined => {
        if (!value) {
            return undefined;
        }

        if (typeof value === 'string') {
            return { type: 'oauth2', provider: value };
        }

        if (value.type === 'oauth2') {
            return {
                type: 'oauth2',
                provider: value.provider,
                scopes: value.scopes ?? [],
                authorizationUrl: value.authorizationUrl,
                tokenUrl: value.tokenUrl,
                refreshUrl: value.refreshUrl,
                sendRedirectUri: value.sendRedirectUri,
            };
        }

        if (value.type === 'secret') {
            const fields = Array.isArray(value.fields) ? value.fields : [];
            return {
                type: 'secret',
                provider: value.provider,
                fields: fields.map((field: any) => ({
                    key: field.key,
                    label: field.label,
                    description: field.description,
                    optional: field.optional ?? false,
                })),
            };
        }

        if (value.provider) {
            return {
                type: 'oauth2',
                provider: value.provider,
                scopes: value.scopes ?? [],
                authorizationUrl: value.authorizationUrl,
                tokenUrl: value.tokenUrl,
                refreshUrl: value.refreshUrl,
                sendRedirectUri: value.sendRedirectUri,
            };
        }

        return undefined;
    };

    // Process integrations - return them to Rust for proper tool wrapping
    for (const integration of plugin.integrations) {
        const integrationAuth = normalizeAuth(
            (integration as any).auth ?? (integration as any).requiresAuth,
        );

        const processedTools = integration.tools.map(tool => {
            const toolAuth =
                normalizeAuth((tool as any).auth ?? (tool as any).requiresAuth) ||
                integrationAuth;

            const { ...restTool } = tool as any;

            return {
                ...restTool,
                parameters: tool.parameters ?? {},
                auth: toolAuth ?? undefined,
            };
        });

        const { ...restIntegration } =
            integration as any;

        processedIntegrations.push({
            ...restIntegration,
            tools: processedTools,
            callbacks: integration.callbacks ?? {},
            notifications: integration.notifications ?? [],
            metadata: integration.metadata ?? {},
            auth: integrationAuth ?? undefined,
        });
    }

    // Process workflows
    const workflows = plugin.workflows || [];
    const processedWorkflows = workflows.map(workflow => {
        const parameters = workflow.parameters || {};
        const examples = workflow.examples || [];

        return {
            ...workflow,
            parameters: parameters,
            examples: examples
        };
    });

    // Return clean Plugin structure to Rust
    return {
        integrations: processedIntegrations,
        workflows: processedWorkflows
    };
}


// Make processPlugin available globally for the Rust side to call
(globalThis as any).processPlugin = processPlugin;
