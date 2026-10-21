// ---
// [permissions]
// read = ["./data"]
// ---
import { defineTool } from "jsr:@distri/tool";

export const tool = defineTool({
  name: "read_file",
  description: "Read a text file",
  parameters: { type: "object", properties: { path: { type: "string" } }, required: ["path"] },
});

export default async function ({ path }: { path: string }) {
  return { text: await Deno.readTextFile(path) };
}
