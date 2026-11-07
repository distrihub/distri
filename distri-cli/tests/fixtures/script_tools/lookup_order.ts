import { defineTool } from "jsr:@distri/tool";

export const tool = defineTool({
  name: "lookup_order",
  description: "Fetch an order by id",
  parameters: { type: "object", properties: { id: { type: "string" } }, required: ["id"] },
});

export default async function ({ id }: { id: string }) {
  console.error(`looking up ${id}`);
  return { id, status: "shipped" };
}
