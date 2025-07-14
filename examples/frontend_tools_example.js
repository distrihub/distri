/**
 * Frontend Tools Example
 * 
 * This example demonstrates how to use frontend tools with the Distri JavaScript SDK.
 * It shows three scenarios:
 * 1. Registering frontend tools
 * 2. Handling tool calls in the frontend
 * 3. Continuing agent execution with tool responses
 */

import { DistriClient } from '@distri/core';
import { useDistri, useDistriClient } from '@distri/react';
import { useState, useCallback } from 'react';

// Initialize the Distri client
const client = new DistriClient({
  baseUrl: 'http://localhost:8080',
  apiVersion: 'v1',
  debug: true
});

/**
 * Example 1: Register Frontend Tools
 */
async function registerFrontendTools() {
  console.log('Registering frontend tools...');

  // Tool 1: Show notification
  const notificationTool = {
    tool: {
      name: 'show_notification',
      description: 'Show a notification to the user',
      input_schema: {
        type: 'object',
        properties: {
          message: {
            type: 'string',
            description: 'The message to display'
          },
          type: {
            type: 'string',
            enum: ['info', 'warning', 'error', 'success'],
            default: 'info',
            description: 'The type of notification'
          },
          duration: {
            type: 'number',
            default: 5000,
            description: 'Duration in milliseconds'
          }
        },
        required: ['message']
      },
      frontend_resolved: true,
      metadata: {
        category: 'ui',
        requires_user_interaction: false
      }
    }
  };

  // Tool 2: Confirm action
  const confirmTool = {
    tool: {
      name: 'confirm_action',
      description: 'Ask user to confirm an action',
      input_schema: {
        type: 'object',
        properties: {
          question: {
            type: 'string',
            description: 'The question to ask the user'
          },
          options: {
            type: 'array',
            items: { type: 'string' },
            default: ['Yes', 'No'],
            description: 'Available options'
          }
        },
        required: ['question']
      },
      frontend_resolved: true,
      metadata: {
        category: 'ui',
        requires_user_interaction: true
      }
    }
  };

  // Tool 3: Get user input
  const inputTool = {
    tool: {
      name: 'get_user_input',
      description: 'Get input from the user',
      input_schema: {
        type: 'object',
        properties: {
          prompt: {
            type: 'string',
            description: 'The prompt to show the user'
          },
          type: {
            type: 'string',
            enum: ['text', 'number', 'email', 'password'],
            default: 'text',
            description: 'The type of input'
          },
          placeholder: {
            type: 'string',
            description: 'Placeholder text'
          }
        },
        required: ['prompt']
      },
      frontend_resolved: true,
      metadata: {
        category: 'ui',
        requires_user_interaction: true
      }
    }
  };

  try {
    const [notificationResult, confirmResult, inputResult] = await Promise.all([
      client.registerFrontendTool(notificationTool),
      client.registerFrontendTool(confirmTool),
      client.registerFrontendTool(inputTool)
    ]);

    console.log('Tools registered successfully:', {
      notification: notificationResult,
      confirm: confirmResult,
      input: inputResult
    });

    return true;
  } catch (error) {
    console.error('Failed to register tools:', error);
    return false;
  }
}

/**
 * Example 2: List Frontend Tools
 */
async function listFrontendTools() {
  try {
    const tools = await client.getFrontendTools();
    console.log('Available frontend tools:', tools);
    return tools;
  } catch (error) {
    console.error('Failed to list tools:', error);
    return [];
  }
}

/**
 * Example 3: Send Message and Handle Tool Calls
 */
async function sendMessageWithToolHandling(agentId, message) {
  console.log(`Sending message to agent ${agentId}:`, message);

  try {
    // Send streaming message
    const response = await client.sendStreamingMessage(agentId, {
      message: {
        role: 'user',
        parts: [{ type: 'text', text: message }]
      }
    });

    const toolCalls = [];
    let streamingText = '';

    // Handle streaming text
    response.on('text_delta', (event) => {
      streamingText += event.delta;
      console.log('Streaming text:', streamingText);
      // Update UI with streaming text
      updateUI({ type: 'text', content: streamingText });
    });

    // Handle tool call events
    response.on('tool_call_start', (event) => {
      console.log('Tool call started:', event);
      toolCalls.push({
        id: event.tool_call_id,
        name: event.tool_call_name,
        status: 'pending'
      });
      updateUI({ type: 'tool_call_start', toolCall: event });
    });

    response.on('tool_call_args', (event) => {
      console.log('Tool call args:', event);
      const toolCall = toolCalls.find(tc => tc.id === event.tool_call_id);
      if (toolCall) {
        toolCall.args = JSON.parse(event.delta);
        toolCall.status = 'ready';
      }
      updateUI({ type: 'tool_call_args', toolCall: event });
    });

    response.on('tool_call_result', (event) => {
      console.log('Tool call result:', event);
      const toolCall = toolCalls.find(tc => tc.id === event.tool_call_id);
      if (toolCall) {
        toolCall.result = event.result;
        toolCall.status = 'completed';
      }
      updateUI({ type: 'tool_call_result', toolCall: event });
    });

    response.on('task_completed', (event) => {
      console.log('Task completed:', event);
      updateUI({ type: 'task_completed', task: event });
    });

    response.on('task_error', (event) => {
      console.error('Task error:', event);
      updateUI({ type: 'task_error', error: event });
    });

    return {
      response,
      toolCalls,
      threadId: response.thread_id
    };

  } catch (error) {
    console.error('Failed to send message:', error);
    throw error;
  }
}

/**
 * Example 4: Handle Tool Execution in Frontend
 */
function handleToolExecution(toolCall) {
  console.log('Handling tool execution:', toolCall);

  switch (toolCall.name) {
    case 'show_notification':
      return handleShowNotification(toolCall);
    
    case 'confirm_action':
      return handleConfirmAction(toolCall);
    
    case 'get_user_input':
      return handleGetUserInput(toolCall);
    
    default:
      console.warn('Unknown tool:', toolCall.name);
      return Promise.resolve('Tool not implemented');
  }
}

function handleShowNotification(toolCall) {
  const { message, type = 'info', duration = 5000 } = toolCall.args;
  
  // Create notification element
  const notification = document.createElement('div');
  notification.className = `notification notification-${type}`;
  notification.innerHTML = `
    <div class="notification-content">
      <span class="notification-message">${message}</span>
      <button class="notification-close" onclick="this.parentElement.parentElement.remove()">×</button>
    </div>
  `;
  
  // Add to page
  document.body.appendChild(notification);
  
  // Auto-remove after duration
  setTimeout(() => {
    if (notification.parentElement) {
      notification.remove();
    }
  }, duration);
  
  return Promise.resolve('Notification displayed successfully');
}

function handleConfirmAction(toolCall) {
  const { question, options = ['Yes', 'No'] } = toolCall.args;
  
  return new Promise((resolve) => {
    // Create confirmation dialog
    const dialog = document.createElement('div');
    dialog.className = 'confirmation-dialog';
    dialog.innerHTML = `
      <div class="dialog-content">
        <h3>${question}</h3>
        <div class="dialog-buttons">
          ${options.map(option => 
            `<button onclick="handleConfirmResponse('${option}')">${option}</button>`
          ).join('')}
        </div>
      </div>
    `;
    
    // Add to page
    document.body.appendChild(dialog);
    
    // Handle response
    window.handleConfirmResponse = (response) => {
      dialog.remove();
      delete window.handleConfirmResponse;
      resolve(response);
    };
  });
}

function handleGetUserInput(toolCall) {
  const { prompt, type = 'text', placeholder = '' } = toolCall.args;
  
  return new Promise((resolve) => {
    // Create input dialog
    const dialog = document.createElement('div');
    dialog.className = 'input-dialog';
    dialog.innerHTML = `
      <div class="dialog-content">
        <h3>${prompt}</h3>
        <input type="${type}" placeholder="${placeholder}" id="user-input" />
        <div class="dialog-buttons">
          <button onclick="handleInputResponse()">Submit</button>
          <button onclick="handleInputCancel()">Cancel</button>
        </div>
      </div>
    `;
    
    // Add to page
    document.body.appendChild(dialog);
    
    // Focus input
    const input = dialog.querySelector('#user-input');
    input.focus();
    
    // Handle submit
    window.handleInputResponse = () => {
      const value = input.value;
      dialog.remove();
      delete window.handleInputResponse;
      delete window.handleInputCancel;
      resolve(value);
    };
    
    // Handle cancel
    window.handleInputCancel = () => {
      dialog.remove();
      delete window.handleInputResponse;
      delete window.handleInputCancel;
      resolve(null);
    };
    
    // Handle enter key
    input.addEventListener('keypress', (e) => {
      if (e.key === 'Enter') {
        window.handleInputResponse();
      }
    });
  });
}

/**
 * Example 5: Continue Agent Execution with Tool Responses
 */
async function continueWithToolResponses(agentId, threadId, toolResponses) {
  console.log('Continuing with tool responses:', toolResponses);

  try {
    const response = await client.continueWithToolResponses(agentId, {
      agent_id: agentId,
      thread_id: threadId,
      tool_responses: toolResponses
    });

    console.log('Agent continued successfully:', response);
    return response;
  } catch (error) {
    console.error('Failed to continue execution:', error);
    throw error;
  }
}

/**
 * Example 6: Complete Chat Flow with Tools
 */
async function completeChatFlow() {
  const agentId = 'my-agent';
  const message = 'Show me a notification and then ask me to confirm an action';

  try {
    // Step 1: Register tools
    await registerFrontendTools();

    // Step 2: Send message
    const { response, toolCalls, threadId } = await sendMessageWithToolHandling(agentId, message);

    // Step 3: Handle tool calls
    const toolResponses = [];
    
    for (const toolCall of toolCalls) {
      if (toolCall.status === 'ready') {
        console.log(`Executing tool: ${toolCall.name}`);
        
        // Execute tool in frontend
        const result = await handleToolExecution(toolCall);
        
        // Add to responses
        toolResponses.push({
          tool_call_id: toolCall.id,
          result: result,
          metadata: {
            executed_at: new Date().toISOString(),
            tool_name: toolCall.name
          }
        });
      }
    }

    // Step 4: Continue execution if we have responses
    if (toolResponses.length > 0) {
      await continueWithToolResponses(agentId, threadId, toolResponses);
    }

    console.log('Chat flow completed successfully');

  } catch (error) {
    console.error('Chat flow failed:', error);
  }
}

/**
 * React Hook Example
 */
function useFrontendTools() {
  const client = useDistriClient();
  const [pendingToolCalls, setPendingToolCalls] = useState([]);
  const [messages, setMessages] = useState([]);

  const sendMessage = useCallback(async (agentId, text) => {
    try {
      const response = await client.sendStreamingMessage(agentId, {
        message: {
          role: 'user',
          parts: [{ type: 'text', text }]
        }
      });

      let streamingText = '';

      response.on('text_delta', (event) => {
        streamingText += event.delta;
        setMessages(prev => {
          const newMessages = [...prev];
          const lastMessage = newMessages[newMessages.length - 1];
          
          if (lastMessage && lastMessage.type === 'streaming') {
            lastMessage.content = streamingText;
          } else {
            newMessages.push({ type: 'streaming', content: streamingText });
          }
          
          return newMessages;
        });
      });

      response.on('tool_call_start', (event) => {
        setPendingToolCalls(prev => [...prev, {
          id: event.tool_call_id,
          name: event.tool_call_name,
          status: 'pending'
        }]);
      });

      response.on('tool_call_args', (event) => {
        setPendingToolCalls(prev => prev.map(call => 
          call.id === event.tool_call_id 
            ? { ...call, args: JSON.parse(event.delta), status: 'ready' }
            : call
        ));
      });

      return { response, threadId: response.thread_id };

    } catch (error) {
      console.error('Failed to send message:', error);
      throw error;
    }
  }, [client]);

  const handleToolResponse = useCallback(async (agentId, threadId, toolCallId, result) => {
    try {
      await client.continueWithToolResponses(agentId, {
        agent_id: agentId,
        thread_id: threadId,
        tool_responses: [{
          tool_call_id: toolCallId,
          result: result,
          metadata: {
            executed_at: new Date().toISOString()
          }
        }]
      });

      setPendingToolCalls(prev => prev.filter(call => call.id !== toolCallId));

    } catch (error) {
      console.error('Failed to handle tool response:', error);
      throw error;
    }
  }, [client]);

  return {
    sendMessage,
    handleToolResponse,
    pendingToolCalls,
    messages
  };
}

/**
 * UI Update Helper
 */
function updateUI(update) {
  // This would typically update your React state or DOM
  console.log('UI Update:', update);
  
  // Example: Update a message container
  const messageContainer = document.getElementById('message-container');
  if (messageContainer) {
    switch (update.type) {
      case 'text':
        messageContainer.innerHTML += `<div class="message">${update.content}</div>`;
        break;
      case 'tool_call_start':
        messageContainer.innerHTML += `
          <div class="tool-call">
            <strong>Tool Call:</strong> ${update.toolCall.tool_call_name}
          </div>
        `;
        break;
      case 'tool_call_args':
        messageContainer.innerHTML += `
          <div class="tool-args">
            <strong>Arguments:</strong> ${update.toolCall.delta}
          </div>
        `;
        break;
    }
  }
}

// Export for use in other modules
export {
  registerFrontendTools,
  listFrontendTools,
  sendMessageWithToolHandling,
  handleToolExecution,
  continueWithToolResponses,
  completeChatFlow,
  useFrontendTools
};

// Run example if this file is executed directly
if (typeof window !== 'undefined') {
  // Browser environment
  window.FrontendToolsExample = {
    registerFrontendTools,
    listFrontendTools,
    sendMessageWithToolHandling,
    handleToolExecution,
    continueWithToolResponses,
    completeChatFlow
  };
  
  console.log('Frontend Tools Example loaded. Use FrontendToolsExample.completeChatFlow() to run the example.');
}