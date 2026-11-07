// A v1.1 typed tool: `input` is an `s` schema, `run` gets the input and the context.
import { defineTool, s } from "jsr:@distri/tool";

export const tool = defineTool({
  name: "typed_pages",
  description: "Pretend to process pages",
  input: s.object({
    pages: s.integer({ min: 1, description: "How many pages" }),
    hang: s.optional(s.boolean({ description: "Wait until the call is cancelled" })),
  }),
  output: s.object({ done: s.integer(), cwd: s.string() }),
  async run({ pages, hang }, ctx) {
    ctx.log(`starting ${ctx.tool}`);
    for (let i = 1; i <= pages; i++) ctx.progress(i / pages, `page ${i}/${pages}`);
    ctx.warn("careful");
    console.error("plain stderr");
    if (hang) {
      await new Promise<void>((resolve) => ctx.signal.addEventListener("abort", () => resolve()));
      ctx.warn("aborted");
    }
    return { done: pages, cwd: ctx.cwd };
  },
});
