// The extractor. Run it under the same Deno permission set as the script itself. Reads
// `{"path": "<absolute script path>"}` on stdin, imports the module and prints one line
// `__distri_tool__ {"tool": <export const tool | null>}` on stdout. An import that throws exits
// non-zero with the error on stderr; the host turns that into a load error.
//
// Two details keep SDK-less scripts (stdin in, stdout out, no `tool`) registrable:
// - stdin is read with the raw reader, never `Deno.stdin.readable`, so a script that reads
//   `.readable` at import time sees EOF instead of "ReadableStream is locked or disturbed";
// - the result is the last line and carries a sentinel, so whatever the script printed to
//   stdout while being imported is ignored (`distri_types::script_tool::parse_extract_stdout`).
import { EXTRACT_SENTINEL } from "./mod.ts";

const chunks: Uint8Array[] = [];
const buf = new Uint8Array(4096);
while (true) {
  const n = await Deno.stdin.read(buf);
  if (n === null) break;
  chunks.push(buf.slice(0, n));
}
const total = chunks.reduce((n, c) => n + c.length, 0);
const bytes = new Uint8Array(total);
let at = 0;
for (const c of chunks) {
  bytes.set(c, at);
  at += c.length;
}
const { path } = JSON.parse(new TextDecoder().decode(bytes)) as { path: string };
const mod = await import(path.startsWith("file://") ? path : `file://${path}`);
const tool = mod.tool === undefined ? null : mod.tool;
await Deno.stdout.write(new TextEncoder().encode(`\n${EXTRACT_SENTINEL}${JSON.stringify({ tool })}\n`));
