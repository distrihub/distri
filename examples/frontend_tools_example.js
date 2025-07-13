// Frontend Tools Example - JavaScript Client

class FrontendToolsClient {
    constructor(baseUrl = 'http://localhost:8080') {
        this.baseUrl = baseUrl;
    }

    // Register a frontend tool
    async registerTool(tool, agentId = null) {
        const response = await fetch(`${this.baseUrl}/api/v1/tools/frontend`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
            },
            body: JSON.stringify({
                tool,
                agent_id: agentId
            })
        });

        if (!response.ok) {
            throw new Error(`Failed to register tool: ${response.statusText}`);
        }

        return await response.json();
    }

    // List all frontend tools
    async listTools(agentId = null) {
        const url = agentId 
            ? `${this.baseUrl}/api/v1/tools/frontend?agent_id=${agentId}`
            : `${this.baseUrl}/api/v1/tools/frontend`;
        
        const response = await fetch(url);
        
        if (!response.ok) {
            throw new Error(`Failed to list tools: ${response.statusText}`);
        }

        return await response.json();
    }

    // Execute a frontend tool
    async executeTool(toolName, toolArguments, agentId, threadId = null, context = null) {
        const response = await fetch(`${this.baseUrl}/api/v1/tools/frontend/execute`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
            },
            body: JSON.stringify({
                tool_name: toolName,
                arguments: toolArguments,
                agent_id: agentId,
                thread_id: threadId,
                context
            })
        });

        if (!response.ok) {
            throw new Error(`Failed to execute tool: ${response.statusText}`);
        }

        return await response.json();
    }

    // Send a message to an agent
    async sendMessage(agentId, message, threadId = null) {
        const response = await fetch(`${this.baseUrl}/api/v1/agents/${agentId}`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
            },
            body: JSON.stringify({
                jsonrpc: '2.0',
                method: 'message/send',
                params: {
                    message: {
                        kind: 'Message',
                        message_id: this.generateId(),
                        role: 'User',
                        parts: [{
                            text: message
                        }],
                        context_id: threadId
                    }
                },
                id: this.generateId()
            })
        });

        if (!response.ok) {
            throw new Error(`Failed to send message: ${response.statusText}`);
        }

        return await response.json();
    }

    generateId() {
        return Math.random().toString(36).substr(2, 9);
    }
}

// Example usage
async function example() {
    const client = new FrontendToolsClient();

    try {
        // 1. Register a notification tool
        const notificationTool = {
            name: 'show_notification',
            description: 'Show a notification to the user',
            input_schema: {
                type: 'object',
                properties: {
                    title: {
                        type: 'string',
                        description: 'The title of the notification'
                    },
                    message: {
                        type: 'string',
                        description: 'The message content'
                    },
                    type: {
                        type: 'string',
                        enum: ['info', 'success', 'warning', 'error'],
                        description: 'The type of notification'
                    }
                },
                required: ['title', 'message']
            },
            frontend_resolved: true,
            metadata: {
                category: 'ui',
                version: '1.0.0'
            }
        };

        console.log('Registering notification tool...');
        const registerResult = await client.registerTool(notificationTool);
        console.log('Tool registered:', registerResult);

        // 2. Register a file upload tool
        const fileUploadTool = {
            name: 'upload_file',
            description: 'Upload a file in the frontend',
            input_schema: {
                type: 'object',
                properties: {
                    file_type: {
                        type: 'string',
                        enum: ['image', 'document', 'video'],
                        description: 'The type of file to upload'
                    },
                    max_size_mb: {
                        type: 'number',
                        description: 'Maximum file size in MB'
                    }
                },
                required: ['file_type']
            },
            frontend_resolved: true,
            metadata: {
                category: 'file',
                version: '1.0.0'
            }
        };

        console.log('Registering file upload tool...');
        const fileUploadResult = await client.registerTool(fileUploadTool, 'assistant');
        console.log('File upload tool registered:', fileUploadResult);

        // 3. List all tools
        console.log('Listing all tools...');
        const allTools = await client.listTools();
        console.log('All tools:', allTools);

        // 4. List tools for specific agent
        console.log('Listing tools for assistant agent...');
        const assistantTools = await client.listTools('assistant');
        console.log('Assistant tools:', assistantTools);

        // 5. Execute a tool (validation)
        console.log('Executing notification tool...');
        const executeResult = await client.executeTool(
            'show_notification',
            {
                title: 'Hello',
                message: 'This is a test notification',
                type: 'info'
            },
            'assistant',
            'test-thread-123'
        );
        console.log('Tool execution result:', executeResult);

        // 6. Send a message to an agent (this would trigger tool usage)
        console.log('Sending message to agent...');
        const messageResult = await client.sendMessage(
            'assistant',
            'Please show me a notification with the message "Hello from the agent!"',
            'test-thread-123'
        );
        console.log('Message result:', messageResult);

    } catch (error) {
        console.error('Error:', error);
    }
}

// Frontend tool execution handlers
class FrontendToolHandlers {
    static showNotification(args) {
        // This would be implemented in your actual frontend
        console.log('Showing notification:', args);
        
        // Example implementation using browser notifications
        if ('Notification' in window && Notification.permission === 'granted') {
            new Notification(args.title, {
                body: args.message,
                icon: '/path/to/icon.png'
            });
        } else {
            // Fallback to alert or custom UI
            alert(`${args.title}: ${args.message}`);
        }
    }

    static uploadFile(args) {
        // This would be implemented in your actual frontend
        console.log('Uploading file:', args);
        
        // Example implementation
        const input = document.createElement('input');
        input.type = 'file';
        input.accept = this.getAcceptTypes(args.file_type);
        
        input.onchange = (event) => {
            const file = event.target.files[0];
            if (file && file.size <= (args.max_size_mb || 10) * 1024 * 1024) {
                console.log('File selected:', file.name);
                // Handle file upload logic here
            } else {
                alert('File too large or invalid type');
            }
        };
        
        input.click();
    }

    static getAcceptTypes(fileType) {
        switch (fileType) {
            case 'image':
                return 'image/*';
            case 'document':
                return '.pdf,.doc,.docx,.txt';
            case 'video':
                return 'video/*';
            default:
                return '*/*';
        }
    }
}

// Message handler that detects and executes frontend tools
function handleAgentMessage(message) {
    console.log('Received message:', message);
    
    // Check if this is a frontend tool response
    if (message.text && message.text.includes('[Frontend Tool:')) {
        // Extract tool information
        const toolMatch = message.text.match(/\[Frontend Tool: ([^\]]+)\]/);
        if (toolMatch) {
            const toolName = toolMatch[1];
            console.log('Frontend tool detected:', toolName);
            
            // Execute the appropriate handler
            switch (toolName) {
                case 'show_notification':
                    // You would extract args from the message or context
                    FrontendToolHandlers.showNotification({
                        title: 'Agent Notification',
                        message: 'This is a notification from the agent',
                        type: 'info'
                    });
                    break;
                case 'upload_file':
                    FrontendToolHandlers.uploadFile({
                        file_type: 'image',
                        max_size_mb: 5
                    });
                    break;
                default:
                    console.log('Unknown frontend tool:', toolName);
            }
        }
    } else {
        // Handle regular message
        console.log('Regular message:', message.text);
    }
}

// Run the example
if (typeof window !== 'undefined') {
    // Browser environment
    window.FrontendToolsClient = FrontendToolsClient;
    window.FrontendToolHandlers = FrontendToolHandlers;
    window.handleAgentMessage = handleAgentMessage;
    window.example = example;
} else {
    // Node.js environment
    module.exports = {
        FrontendToolsClient,
        FrontendToolHandlers,
        handleAgentMessage,
        example
    };
}