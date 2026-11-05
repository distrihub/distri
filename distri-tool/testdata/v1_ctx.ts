import { defineTool, type ToolContext } from "../mod.ts";

export const tool = defineTool({ name: "v1_ctx", parameters: { type: "object", properties: {} } });

// A v1 default export; the harness passes the context as a second argument.
export default function (_input: unknown, ctx: ToolContext) {
  ctx.progress(undefined, "no fraction");
  return { signal: ctx.signal instanceof AbortSignal, tool: ctx.tool, cwd: ctx.cwd };
}
