import { defineTool } from "../mod.ts";

export const tool = defineTool({
  name: "echo",
  description: "Echo the input back",
  parameters: { type: "object", properties: { text: { type: "string" } }, required: ["text"] },
});

console.log("import-time noise");

export default async function ({ text }: { text: string }) {
  if (text === "boom") throw new Error("boom");
  return { echoed: text };
}
