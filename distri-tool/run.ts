// The run harness: `deno run <permissions> run.ts <script path or file:// URL> [cwd] [sdk]`.
// Imports the script, then runs it under the contract (`runModule`): stdin JSON in, stdout JSON
// out, stderr for logs, exit 1 on a throw. `tool.run` (or the default export) gets the input and
// a `ToolContext`; `cwd` (the host's working directory) becomes `ctx.cwd`. This is how the
// `distri` CLI runs a script tool; the permission flags on the `deno` command line are the
// sandbox.
//
// A workflow `code` step may also get the script SDK (`@distri/runtime`). The third argument says
// how its calls travel:
// - `relay`: stdin's first line is the input document; each call is written to stderr as
//   `__distri_sdk_call__ {"id", "call"}` and its answer arrives as one stdin line
//   `{"id", "ok", "result" | "error", "code"}`. The host makes the call: the script needs no
//   network access to the host and never sees a token.
// - a URL: stdin's first line is `__distri_sdk_token__ <token>`, then the input document; calls
//   go to `<url>/call` with the token, which stays in a closure inside `runtime.ts`.
import { fail, runModule } from "./mod.ts";
import { type CallAnswer, httpTransport, setTransport, unwrap } from "./runtime.ts";

const TOKEN_LINE = "__distri_sdk_token__ ";
const CALL_LINE = "__distri_sdk_call__ ";
const encoder = new TextEncoder();

/** stdin as lines. */
async function* lines(stream: ReadableStream<Uint8Array>): AsyncGenerator<string> {
  const decoder = new TextDecoder();
  let buf = "";
  for await (const chunk of stream) {
    buf += decoder.decode(chunk, { stream: true });
    let nl: number;
    while ((nl = buf.indexOf("\n")) >= 0) {
      yield buf.slice(0, nl);
      buf = buf.slice(nl + 1);
    }
  }
  buf += decoder.decode();
  if (buf) yield buf;
}

function parseInput(text: string): unknown {
  return text.trim() ? JSON.parse(text) : null;
}

async function relay(): Promise<unknown> {
  const it = lines(Deno.stdin.readable);
  const first = await it.next();
  const input = parseInput(first.done ? "" : first.value);
  const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: unknown) => void; op: string }>();
  let seq = 0;
  (async () => {
    for await (const line of it) {
      if (!line.trim()) continue;
      let answer: CallAnswer & { id?: number };
      try {
        answer = JSON.parse(line);
      } catch {
        continue;
      }
      const p = pending.get(answer.id ?? -1);
      if (!p) continue;
      pending.delete(answer.id!);
      try {
        p.resolve(unwrap(p.op, answer));
      } catch (e) {
        p.reject(e);
      }
    }
    for (const p of pending.values()) p.reject(new Error("@distri/runtime: the host closed the call channel"));
  })();
  setTransport((op, args) =>
    new Promise((resolve, reject) => {
      const id = ++seq;
      pending.set(id, { resolve, reject, op });
      Deno.stderr.writeSync(encoder.encode(`${CALL_LINE}${JSON.stringify({ id, call: { op, ...args } })}\n`));
    })
  );
  return input;
}

async function http(url: string): Promise<unknown> {
  let text = await new Response(Deno.stdin.readable).text();
  let token = "";
  if (text.startsWith(TOKEN_LINE)) {
    const nl = text.indexOf("\n");
    token = (nl < 0 ? text : text.slice(0, nl)).slice(TOKEN_LINE.length).trim();
    text = nl < 0 ? "" : text.slice(nl + 1);
  }
  setTransport(httpTransport(url, token));
  return parseInput(text);
}

const [target, cwd, sdk] = Deno.args;
if (!target) fail(new Error("usage: run.ts <script> [cwd] [relay | sdk url]"));
try {
  const opts: { cwd?: string; input?: unknown } = { cwd };
  if (sdk === "relay") opts.input = await relay();
  else if (sdk) opts.input = await http(sdk);
  await runModule(await import(target.startsWith("file://") ? target : `file://${target}`), opts);
  // The relay keeps stdin open for answers; the run is over once the output is written.
  if (sdk === "relay") Deno.exit(0);
} catch (e) {
  fail(e);
}
