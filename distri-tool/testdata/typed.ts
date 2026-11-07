import { defineTool, s } from "../mod.ts";

export const tool = defineTool({
  name: "typed",
  description: "A v1.1 typed tool",
  input: s.object({
    pages: s.integer({ min: 1, description: "how many pages" }),
    title: s.optional(s.string()),
    hang: s.optional(s.boolean()),
  }),
  output: s.object({ done: s.integer(), title: s.string() }),
  async run({ pages, title, hang }, ctx) {
    ctx.log("starting", { pages });
    for (let i = 1; i <= pages; i++) ctx.progress(i / pages, `page ${i}/${pages}`);
    ctx.warn("careful");
    console.error("plain stderr");
    if (hang) {
      await new Promise<void>((resolve) => ctx.signal.addEventListener("abort", () => resolve()));
      ctx.warn("aborted");
    }
    return { done: pages, title: `${title ?? "untitled"} in ${ctx.tool}` };
  },
});
