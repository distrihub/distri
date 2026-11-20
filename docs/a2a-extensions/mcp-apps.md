# mcp-apps

`https://distri.dev/a2a/extensions/mcp-apps/v1` — see the MCP Apps extension
(`io.modelcontextprotocol/ui`).

- **Parts:** `ui://` resources as DataParts with `metadata["distri/part_type"] = "resource_link"`
  (`{uri, mime_type, _meta}`).
- **Endpoints:** `/v1/mcp-apps/resources/read`, `/v1/mcp-apps/tools/call`.
- **params:** `part_types`, `resources_read`, `tools_call`, `see`.
