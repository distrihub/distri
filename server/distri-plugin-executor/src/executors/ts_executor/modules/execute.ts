import { Context, WorkflowCall } from "./base.ts";

export function executeTool(pluginPath: string, toolCall: any, context: Context): Promise<any> {
    // Extract tool name and parameters from the ToolCall object
    const toolName = toolCall.tool_name;
    const parameters = toolCall.input;
    return executeToolInner(pluginPath, toolName, parameters, context);
}

// Plugin execution helper function  
export async function executeToolInner(pluginPath: string, toolName: string, parameters: any, context: Context): Promise<any> {
    try {
        const modulePath = pluginPath.startsWith("plugin://")
            ? pluginPath
            : `./${pluginPath}`;
        const plugin = await import(modulePath);
        const pluginDefault = plugin.default || plugin;
        let tool;

        // Search for tool in integrations (new architecture)
        if (!pluginDefault.integrations || pluginDefault.integrations.length === 0) {
            throw new Error("Plugin has no integrations with tools");
        }

        // Search through all integrations for the tool
        for (const integration of pluginDefault.integrations) {
            const actualToolName = toolName.replace(integration.name + "_", "");
            if (integration.tools) {
                tool = integration.tools.find((t: any) => t.name === actualToolName);
                if (tool) {
                    break;
                }
            }
        }

        if (!tool) {
            throw new Error(`Tool ${toolName} not found`);
        }

        if (typeof tool.execute !== 'function') {
            throw new Error("Item does not have execute method");
        }

        // Tools get the parameters directly (not wrapped in toolCall structure)
        return tool.execute(parameters, context);

    } catch (error) {
        console.error("Error executing plugin:", error);
        throw error;
    }
}

export function executeWorkflow(pluginPath: string, workflowCall: WorkflowCall, context: Context): Promise<any> {
    return executeWorkflowInner(pluginPath, workflowCall, context);
}

// Plugin execution helper function  
export async function executeWorkflowInner(pluginPath: string, workflowCall: WorkflowCall, context: Context): Promise<any> {
    try {
        // For loaded plugins, import the module using the correct key format
        const modulePath = pluginPath.startsWith("plugin://")
            ? pluginPath
            : `./${pluginPath}`;
        const plugin = await import(modulePath);
        const pluginDefault = plugin.default || plugin;
        let workflow;

        if (!pluginDefault.workflows) {
            throw new Error("Plugin has no workflows");
        }

        if (Array.isArray(pluginDefault.workflows)) {
            workflow = pluginDefault.workflows.find((w) => w.name === workflowCall.workflow_name);
        } else {
            workflow = pluginDefault.workflows[workflowCall.workflow_name];
        }


        if (!workflow) {
            throw new Error(`Workflow ${workflowCall.workflow_name} not found`);
        }

        if (typeof workflow.execute !== 'function') {
            throw new Error("Workflow does not have execute method");
        }

        // Execute the workflow
        const params = typeof workflowCall.input === 'string'
            ? JSON.parse(workflowCall.input)
            : workflowCall.input || {};

        // Workflows get params and context (not workflowCall structure)
        return workflow.execute(params, context);

    } catch (error) {
        console.error("Error executing plugin:", error);
        throw error;
    }
}
