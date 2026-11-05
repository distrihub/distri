import { defineTool } from "jsr:@distri/tool";

export const tool = defineTool({ name: "peek_env", description: "Read HOME" });

export default function () {
  return { home: Deno.env.get("HOME") };
}
