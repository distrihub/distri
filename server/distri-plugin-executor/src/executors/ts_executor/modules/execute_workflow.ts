import { Context, WorkflowCall, loadPlugin } from "./base";

export default function executeWorkflow(plugin_name: string, workflowCall: WorkflowCall, context: Context): Promise<any> {
    return executeInner(plugin_name, workflowCall, context);
}

// Plugin execution helper function  
export async function executeInner(plugin_name: string, workflowCall: WorkflowCall, context: Context): Promise<any> {
    try {

        const plugin = await loadPlugin(plugin_name);
        let workflow;

        if (!plugin.workflows) {
            throw new Error("Plugin has no tools");
        }

        if (Array.isArray(plugin.workflows)) {
            workflow = plugin.workflows.find((w) => w.name === workflowCall.workflow_name);
        } else {
            workflow = plugin.workflows[workflowCall.workflow_name];
        }


        if (!workflow) {
            throw new Error(`Workflow ${workflowCall.workflow_name} not found`);
        }

        if (typeof workflow.execute !== 'function') {
            throw new Error("Workflow does not have execute method");
        }

        // Execute the item
        const params = typeof workflowCall.input === 'string'
            ? JSON.parse(workflowCall.input)
            : workflowCall.input || {};
        // Tools get toolCall structure
        return workflow.execute(workflowCall, context);

    } catch (error) {
        console.error("Error executing plugin:", error);
        throw error;
    }
}
