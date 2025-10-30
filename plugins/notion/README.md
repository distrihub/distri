# Notion Plugin

Tools for searching and creating content in Notion with the public REST API. Use it to pull knowledge into agents or automate page creation from workflows.

## Authentication

Provide a Notion integration token via context secrets (`NOTION_API_KEY`, `notion`, or `notion:default`) or by passing `api_key` when invoking a tool.

## Tools

| Tool | Description |
| --- | --- |
| `search_pages` | Full-text search across accessible pages and databases |
| `create_page` | Create a page in a database or under another page |

## Usage Example

```ts
import notionPlugin from "./mod.ts";
import { registerPlugin, callTool } from "jsr:@distri/runtime@0.1.0";

registerPlugin(notionPlugin);
const results = await callTool({
  integration: "notion",
  tool_name: "search_pages",
  input: { query: "architecture" },
  context: { secrets: { NOTION_API_KEY: "secret_xyz" } },
});
```
