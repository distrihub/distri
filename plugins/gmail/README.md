# Gmail Plugin

Gmail utilities for reading and sending messages from distri workflows. Requests are made using the REST API and require an OAuth access token in the execution context.

## Authentication

Populate `context.auth_session.access_token` with a valid Gmail token that includes the scopes defined below. The included metadata describes the OAuth configuration used by distri.

## Tools

| Tool | Description |
| --- | --- |
| `list_emails` | Search the mailbox with an optional query |
| `send_email` | Send a MIME-formatted email (text body only in this example) |

## Example

```ts
import gmailPlugin from "./mod.ts";
import { registerPlugin, callTool } from "jsr:@distri/runtime@0.1.0";

registerPlugin(gmailPlugin);
await callTool({
  integration: "gmail",
  tool_name: "list_emails",
  input: { query: "from:alerts@example.com" },
  context: { auth_session: { access_token: "ya29..." } },
});
```
