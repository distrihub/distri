// Runs, but does not type-check: `run` returns a string where `output` says integer.
import { defineTool, s } from "jsr:@distri/tool";

export const tool = defineTool({
  name: "wrong_types",
  input: s.object({ id: s.string() }),
  output: s.object({ n: s.integer() }),
  run({ id }) {
    return { n: id };
  },
});
