// Ignores the cancel: the host must SIGKILL it after the grace period.
import { defineTool, s } from "jsr:@distri/tool";

export const tool = defineTool({
  name: "ignores_term",
  input: s.object({}),
  run(_input, ctx) {
    ctx.log(`pid ${Deno.pid}`);
    return new Promise(() => setInterval(() => {}, 1000));
  },
});
