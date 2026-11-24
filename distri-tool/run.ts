// The run harness: `deno run <permissions> run.ts <script path or file:// URL> [cwd] [sdk url]`.
// Imports the script, then runs it under the contract (`runModule`): stdin JSON in, stdout JSON
// out, stderr for logs, exit 1 on a throw. `tool.run` (or the default export) gets the input and
// a `ToolContext`; `cwd` (the host's working directory) becomes `ctx.cwd`. This is how the
// `distri` CLI runs a script tool; the permission flags on the `deno` command line are the
// sandbox.
//
// A workflow `code` step may also get the script SDK (`@distri/runtime`): the host passes the SDK
// URL as the third argument and the step's run token as the first line of stdin, before the
// input document (`__distri_sdk_token__ <token>`), so the token is never on a command line.
import { fail, runModule } from "./mod.ts";

const TOKEN_LINE = "__distri_sdk_token__ ";

const [target, cwd, sdkUrl] = Deno.args;
if (!target) fail(new Error("usage: run.ts <script> [cwd] [sdk url]"));
try {
  const opts: { cwd?: string; input?: unknown } = { cwd };
  if (sdkUrl) {
    let text = await new Response(Deno.stdin.readable).text();
    let token = "";
    if (text.startsWith(TOKEN_LINE)) {
      const nl = text.indexOf("\n");
      token = (nl < 0 ? text : text.slice(0, nl)).slice(TOKEN_LINE.length).trim();
      text = nl < 0 ? "" : text.slice(nl + 1);
    }
    (globalThis as Record<string, unknown>).__distri_sdk__ = { url: sdkUrl, token };
    opts.input = text.trim() ? JSON.parse(text) : null;
  }
  await runModule(await import(target.startsWith("file://") ? target : `file://${target}`), opts);
} catch (e) {
  fail(e);
}
