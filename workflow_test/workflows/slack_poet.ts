/**
 * Poet Slack Workflow
 * Takes a user message, generates a poem using a poet agent, and sends it to Slack
 */

// Import from distri/base.ts (provided by the runtime)
import { DapWorkflow, callAgent, callTool } from "https://distri.dev/base.ts";

async function run(input: any, context: any): Promise<any> {
    console.log('🎭 Starting Poet Slack Workflow');
    console.log(`📝 Input message: "${input.message}"`);

    const channel = input.channel || "#poetry";


    const poetPrompt = `Please write a beautiful, creative poem inspired by this message: "${input.message}". 
        Make it meaningful, artistic, and capture the essence of the message. 
        Use appropriate poetic devices like metaphor, imagery, and rhythm.`;

    console.log('🤖 Running poet agent...');
    const poem = await callAgent({
        agent_name: 'poet_agent',
        task: poetPrompt,
        session_id: context.session_id
    });

    console.log('✅ Poem generated successfully by agent');

    console.log(`🎨 Generated poem (${poem.length} characters)`);

    // Use callTool to send Slack message - let the system discover and execute the tool
    console.log(`📤 Sending poem to Slack channel: ${channel}`);
    const slackResponse = await callTool({
        tool_name: 'slack_send_message',
        input: {
            channel: channel,
            text: poem,
            username: "Poetry Bot",
            icon_emoji: ":scroll:",
            token: input.slackToken  // Let the tool handle token fallbacks
        },
        session_id: context.session_id
    });

    console.log(slackResponse);
    console.log('✅ Successfully sent poem to Slack');
    return {
        success: true,
        slack_response: slackResponse,
    }

}



// Export the workflow configuration as DapWorkflow
const slackPoetWorkflow: DapWorkflow = {
    name: "slack_poet",
    description: "Takes a message, generates a poem using a poet agent, and sends it to Slack",
    version: "1.0.0",

    async execute(params: any, context: any): Promise<any> {
        return await run(params, context);
    },

    parameters: {
        type: "object",
        properties: {
            message: {
                type: "string",
                description: "Message to turn into a poem"
            },
            channel: {
                type: "string",
                description: "Slack channel (defaults to #poetry)",
                default: "#poetry"
            },
            slackToken: {
                type: "string",
                description: "Slack bot token (can use env var)"
            }
        },
        required: ["message"]
    },

    examples: [
        {
            description: "Simple poem generation",
            input: {
                message: "coffee in the morning",
                channel: "#poetry"
            },
            expected_output: "Generates and sends a poem about coffee to Slack"
        },
        {
            description: "Poem with custom channel",
            input: {
                message: "sunset over mountains",
                channel: "#general"
            },
            expected_output: "Generates and sends a poem about sunsets to #general"
        }
    ]
};

export default slackPoetWorkflow;