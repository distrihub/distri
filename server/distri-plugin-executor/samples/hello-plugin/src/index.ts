/**
 * js_samples DAP Package - JavaScript Entry Point
 * 
 * Self-contained implementation without TypeScript syntax for rustyscript compatibility
 */

import { createTool, DistriTool, DistriPlugin, createIntegration } from "https://distri.dev/base.ts";

/**
 * Hello Tool - Simple greeting tool
 */
const hellTool = createTool({
    name: 'hello',
    description: 'Simple greeting tool that says hello with optional personalization',
    version: '1.0.0',
    execute: async (params: any, context: any) => {
        const name = params.name || 'World';
        const message = params.message || 'Hello';

        return {
            greeting: `${message}, ${name}!`,
            timestamp: new Date().toISOString()
        };
    },
    parameters: {
        type: 'object',

        properties: {
            name: {
                type: 'string',
                description: 'Name to greet (default: World)',
                default: 'World'
            },
            message: {
                type: 'string',
                description: 'Custom greeting message (default: Hello)',
                default: 'Hello'
            }
        },
        additionalProperties: false
    }
});

const randomApiTool = createTool({
    name: 'random_api',
    description: 'Fetches random data from various public APIs (quotes, facts, jokes, UUIDs)',
    version: '1.0.0',
    execute: async (params: any, context: any) => {
        const apiType = params.apiType || 'quote';

        const responses = {
            quote: { text: "The only impossible journey is the one you never begin.", author: "Tony Robbins" },
            fact: { text: "Honey never spoils. Archaeologists have found pots of honey in ancient Egyptian tombs that are over 3,000 years old and still perfectly edible." },
            joke: { text: "Why don't scientists trust atoms? Because they make up everything!" },
            uuid: { uuid: "550e8400-e29b-41d4-a716-446655440000" }
        };

        return {
            apiType: apiType,
            data: responses[apiType] || responses.quote,
            timestamp: new Date().toISOString()
        };
    },
    parameters: {
        type: 'object',
        properties: {
            apiType: {
                type: 'string',
                enum: ['quote', 'fact', 'joke', 'uuid'],
                description: 'Type of API to call',
                default: 'quote'
            }
        },
        additionalProperties: false
    }
});


const sampleIntegration = createIntegration({
    name: 'sample',
    description: 'Sample integration',
    version: '1.0.0',
    tools: [hellTool, randomApiTool],
    notifications: [],
    metadata: {
        category: 'development'
    }
});
const plugin: DistriPlugin = {
    integrations: [sampleIntegration],
    workflows: [],

};
export default plugin;