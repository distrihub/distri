import { defineTool, EXTRACT_SENTINEL } from "./mod.ts";

const here = new URL(".", import.meta.url).pathname;
const fixture = `${here}testdata/echo.ts`;

async function deno(entry: string, stdin: string): Promise<{ code: number; stdout: string; stderr: string }> {
  const child = new Deno.Command(Deno.execPath(), {
    args: ["run", "--no-prompt", "--quiet", `--allow-read=${here}`, `${here}${entry}`, fixture],
    stdin: "piped",
    stdout: "piped",
    stderr: "piped",
  }).spawn();
  const w = child.stdin.getWriter();
  await w.write(new TextEncoder().encode(stdin));
  await w.close();
  const out = await child.output();
  const dec = new TextDecoder();
  return { code: out.code, stdout: dec.decode(out.stdout), stderr: dec.decode(out.stderr) };
}

function assertEquals(a: unknown, b: unknown) {
  if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(`${JSON.stringify(a)} !== ${JSON.stringify(b)}`);
}

Deno.test("defineTool requires a name", () => {
  assertEquals(defineTool({ name: "x" }).name, "x");
  let threw = false;
  try {
    defineTool({ name: "" });
  } catch {
    threw = true;
  }
  assertEquals(threw, true);
});

Deno.test("run.ts: stdin JSON in, stdout JSON out (after the script's own output)", async () => {
  const r = await deno("run.ts", JSON.stringify({ text: "hi" }));
  assertEquals(r.code, 0);
  const lines = r.stdout.trim().split("\n");
  assertEquals(JSON.parse(lines[lines.length - 1]), { echoed: "hi" });
});

Deno.test("run.ts: a throw is exit 1 with the error on stderr", async () => {
  const r = await deno("run.ts", JSON.stringify({ text: "boom" }));
  assertEquals(r.code, 1);
  if (!r.stderr.includes("Error: boom")) throw new Error(r.stderr);
});

Deno.test("extract.ts: prints the tool export after the sentinel", async () => {
  const child = new Deno.Command(Deno.execPath(), {
    args: ["run", "--no-prompt", "--quiet", `--allow-read=${here}`, `${here}extract.ts`],
    stdin: "piped",
    stdout: "piped",
  }).spawn();
  const w = child.stdin.getWriter();
  await w.write(new TextEncoder().encode(JSON.stringify({ path: fixture })));
  await w.close();
  const out = new TextDecoder().decode((await child.output()).stdout);
  const line = out.split("\n").reverse().find((l) => l.startsWith(EXTRACT_SENTINEL))!;
  assertEquals(JSON.parse(line.slice(EXTRACT_SENTINEL.length)).tool.name, "echo");
});
