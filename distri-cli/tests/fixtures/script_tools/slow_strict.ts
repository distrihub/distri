// Its run takes 5s to start (the import sleeps when run, not when extracted), so a call that
// returns quickly never reached Deno's run.
import { defineTool } from "jsr:@distri/tool";

if (Deno.args.length > 0) await new Promise((r) => setTimeout(r, 5000));

export const tool = defineTool({
  name: "slow_strict",
  description: "Strict input",
  parameters: {
    type: "object",
    properties: { id: { type: "string" }, n: { type: "integer", minimum: 1 } },
    required: ["id"],
  },
});

export default ({ id }: { id: string }) => ({ id });
