// ---
// [tool]
// timeout = 1
// ---
import { defineTool, s } from "jsr:@distri/tool";

export const tool = defineTool({
  name: "sleepy",
  input: s.object({}),
  async run(_input, ctx) {
    await new Promise((r) => setTimeout(r, 10_000));
    return { slept: !ctx.signal.aborted };
  },
});
