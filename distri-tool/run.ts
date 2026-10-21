// The run harness: `deno run <permissions> run.ts <script path or file:// URL>`. Imports the
// script, then runs it under the contract (`runModule`): stdin JSON in, stdout JSON out, stderr
// for logs, exit 1 on a throw. This is how the `distri` CLI runs a script tool; the permission
// flags on the `deno` command line are the sandbox.
import { fail, runModule } from "./mod.ts";

const target = Deno.args[0];
if (!target) fail(new Error("usage: run.ts <script>"));
try {
  await runModule(await import(target.startsWith("file://") ? target : `file://${target}`));
} catch (e) {
  fail(e);
}
