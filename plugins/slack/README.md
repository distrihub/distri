# Slack Plugin

Slack tools for sending messages, managing channels, and uploading files from distri workflows. The implementation uses the official Slack Web API client and expects a bot token with the required scopes for chat and files.

## Features

- Send, update, and delete messages
- List channels and inspect channel or user metadata
- Upload files and run connection diagnostics

## Authentication

Set a Slack bot token via the execution context secrets (e.g. `SLACK_BOT_TOKEN`) or pass `token` in individual tool calls.

## Tools

| Tool | Description |
| --- | --- |
| `send_message` | Post a message to a channel with optional blocks and attachments |
| `list_channels` | List public and private channels available to the bot |
| `get_user_info` | Fetch user profile details |
| `get_channel_info` | Fetch channel metadata |
| `upload_file` | Upload a file to one or more channels |
| `update_message` | Edit an existing message |
| `delete_message` | Delete a message |
| `test_connection` | Run `auth.test` to verify the token |

## Local Testing

Register the plugin with the Deno runtime helper and call tools directly:

```ts
import slackPlugin from "./mod.ts";
import { registerPlugin, callTool } from "jsr:@distri/runtime@0.1.0";

registerPlugin(slackPlugin);
await callTool({
  integration: "slack",
  tool_name: "send_message",
  input: { channel: "#general", text: "Hello team" },
  context: { secrets: { SLACK_BOT_TOKEN: "xoxb-..." } },
});
```
