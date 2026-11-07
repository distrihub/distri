// The run harness: `deno run <permissions> run.ts <script path or file:// URL> [cwd]`. Imports
// the script, then runs it under the contract (`runModule`): stdin JSON in, stdout JSON out,
// stderr for logs, exit 1 on a throw. `tool.run` (or the default export) gets the input and a
// `ToolContext`; `cwd` (the host's working directory) becomes `ctx.cwd`. This is how the
// `distri` CLI runs a script tool; the permission flags on the `deno` command line are the
// sandbox.
import { fail, runModule } from "./mod.ts";

const [target, cwd] = Deno.args;
if (!target) fail(new Error("usage: run.ts <script> [cwd]"));
try {
  await runModule(await import(target.startsWith("file://") ? target : `file://${target}`), { cwd });
} catch (e) {
  fail(e);
}
