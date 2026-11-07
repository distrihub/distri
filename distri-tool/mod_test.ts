import {
  CTX_SENTINEL,
  defineTool,
  EXTRACT_SENTINEL,
  type FromSchema,
  type Infer,
  s,
  type ToolContext,
  toolExport,
} from "./mod.ts";

const here = new URL(".", import.meta.url).pathname;
const echo = `${here}testdata/echo.ts`;
const typed = `${here}testdata/typed.ts`;
const v1Ctx = `${here}testdata/v1_ctx.ts`;

interface Out {
  code: number;
  stdout: string;
  stderr: string;
}

function spawn(entry: string, args: string[]): Deno.ChildProcess {
  return new Deno.Command(Deno.execPath(), {
    args: ["run", "--no-prompt", "--quiet", `--allow-read=${here}`, `${here}${entry}`, ...args],
    stdin: "piped",
    stdout: "piped",
    stderr: "piped",
  }).spawn();
}

async function deno(entry: string, stdin: string, args: string[] = [echo]): Promise<Out> {
  const child = spawn(entry, args);
  const w = child.stdin.getWriter();
  await w.write(new TextEncoder().encode(stdin));
  await w.close();
  const out = await child.output();
  const dec = new TextDecoder();
  return { code: out.code, stdout: dec.decode(out.stdout), stderr: dec.decode(out.stderr) };
}

function assertEquals(a: unknown, b: unknown) {
  if (JSON.stringify(a) !== JSON.stringify(b)) {
    throw new Error(`${JSON.stringify(a)} !== ${JSON.stringify(b)}`);
  }
}

function assertThrows(f: () => unknown, needle: string) {
  try {
    f();
  } catch (e) {
    if (!String(e).includes(needle)) throw new Error(`threw ${e}, expected ${needle}`);
    return;
  }
  throw new Error(`did not throw (${needle})`);
}

function ctxLines(stderr: string): unknown[] {
  return stderr.split("\n").filter((l) => l.startsWith(CTX_SENTINEL)).map((l) =>
    JSON.parse(l.slice(CTX_SENTINEL.length))
  );
}

function lastJson(stdout: string): unknown {
  const lines = stdout.trim().split("\n");
  return JSON.parse(lines[lines.length - 1]);
}

// Type-level checks: these fail `deno test` (which type-checks) when the inference is wrong.
type Equals<A, B> = (<T>() => T extends A ? 1 : 2) extends (<T>() => T extends B ? 1 : 2) ? true : false;
function assertType<_T extends true>() {}

Deno.test("defineTool requires a name", () => {
  assertEquals(defineTool({ name: "x" }).name, "x");
  assertThrows(() => defineTool({ name: "" }), "`name` is required");
});

Deno.test("s.*: each builder is a JSON Schema", () => {
  assertEquals(s.string().schema, { type: "string" });
  assertEquals(
    s.string({ format: "email", enum: ["a@b.c"], description: "who", minLength: 3, maxLength: 9 }).schema,
    { type: "string", description: "who", format: "email", enum: ["a@b.c"], minLength: 3, maxLength: 9 },
  );
  assertEquals(s.number({ min: 0, max: 1, description: "f" }).schema, {
    type: "number",
    description: "f",
    minimum: 0,
    maximum: 1,
  });
  assertEquals(s.integer({ min: 1 }).schema, { type: "integer", minimum: 1 });
  assertEquals(s.boolean().schema, { type: "boolean" });
  assertEquals(s.array(s.string()).schema, { type: "array", items: { type: "string" } });
  assertEquals(s.enum(["a", "b"] as const).schema, { type: "string", enum: ["a", "b"] });
  assertEquals(s.enum([1, "x"] as const).schema, { enum: [1, "x"] });
  assertEquals(s.literal("v1").schema, { type: "string", const: "v1" });
  assertEquals(s.literal(3).schema, { type: "integer", const: 3 });
  assertEquals(s.optional(s.string()).optional, true);
});

Deno.test("s.object: non-optional props are required", () => {
  const o = s.object({ id: s.string(), n: s.optional(s.number()), tags: s.array(s.string()) });
  assertEquals(o.schema, {
    type: "object",
    properties: {
      id: { type: "string" },
      n: { type: "number" },
      tags: { type: "array", items: { type: "string" } },
    },
    required: ["id", "tags"],
  });
  assertEquals(s.object({}, { additionalProperties: false }).schema, {
    type: "object",
    properties: {},
    required: [],
    additionalProperties: false,
  });
  type O = Infer<typeof o>;
  assertType<Equals<O, { id: string; tags: string[]; n?: number }>>();
  const kind = s.enum(["a", "b"] as const);
  assertType<Equals<Infer<typeof kind>, "a" | "b">>();
  const lit = s.literal("v");
  assertType<Equals<Infer<typeof lit>, "v">>();
});

Deno.test("FromSchema: the type of a raw schema written as const", () => {
  const params = {
    type: "object",
    properties: {
      id: { type: "string" },
      count: { type: "integer" },
      ok: { type: "boolean" },
      tags: { type: "array", items: { type: "string" } },
      mode: { enum: ["fast", "slow"] },
    },
    required: ["id", "mode"],
  } as const;
  type P = FromSchema<typeof params>;
  assertType<
    Equals<P, { id: string; mode: "fast" | "slow"; count?: number; ok?: boolean; tags?: string[] }>
  >();

  // A v1 definition with `as const` parameters types `run`'s input.
  const t = defineTool({
    name: "raw",
    parameters: params,
    run(input: FromSchema<typeof params>, ctx: ToolContext) {
      return { id: input.id, mode: input.mode, tool: ctx.tool };
    },
  });
  assertEquals(t.parameters?.required, ["id", "mode"]);
});

Deno.test("defineTool v1.1: input becomes parameters, run is kept", async () => {
  const t = defineTool({
    name: "lookup",
    description: "d",
    input: s.object({ id: s.string() }),
    output: s.object({ status: s.string() }),
    run({ id }) {
      return { status: `shipped ${id}` };
    },
  });
  assertEquals(t.parameters, { type: "object", properties: { id: { type: "string" } }, required: ["id"] });
  const { ctx } = { ctx: {} as ToolContext };
  assertEquals(await t.run({ id: "A" }, ctx), { status: "shipped A" });
  // The wire shape is the v1 ToolExport: no `input`, `output` or `run`.
  assertEquals(toolExport(t), { name: "lookup", description: "d", parameters: t.parameters });

  // Types: `run`'s input comes from `input`; its result is checked against `output`.
  const typed = defineTool({
    name: "typed",
    input: s.object({ id: s.string(), n: s.optional(s.integer()) }),
    output: s.object({ id: s.string(), status: s.enum(["open", "shipped"] as const) }),
    async run({ id, n }) {
      assertType<Equals<typeof n, number | undefined>>();
      return { id, status: "shipped" };
    },
  });
  assertEquals(typed.parameters.required, ["id"]);
  defineTool({
    name: "wrong_output",
    input: s.object({ id: s.string() }),
    output: s.object({ n: s.integer() }),
    // @ts-expect-error: `n` is a string, `output` says integer
    run({ id }) {
      return { n: id };
    },
  });
  defineTool({
    name: "wrong_input",
    input: s.object({ id: s.string() }),
    // @ts-expect-error: `nope` is not in `input`
    run({ nope }) {
      return nope;
    },
  });

  assertThrows(
    () => defineTool({ name: "x", input: s.string() as never, run: () => null }),
    "`input` must be s.object",
  );
  assertThrows(
    // deno-lint-ignore no-explicit-any
    () => defineTool({ name: "x", input: s.object({}), parameters: {}, run: () => null } as any),
    "not both",
  );
});

Deno.test("run.ts: stdin JSON in, stdout JSON out (after the script's own output)", async () => {
  const r = await deno("run.ts", JSON.stringify({ text: "hi" }));
  assertEquals(r.code, 0);
  assertEquals(lastJson(r.stdout), { echoed: "hi" });
});

Deno.test("run.ts: a throw is exit 1 with the error on stderr", async () => {
  const r = await deno("run.ts", JSON.stringify({ text: "boom" }));
  assertEquals(r.code, 1);
  if (!r.stderr.includes("Error: boom")) throw new Error(r.stderr);
});

Deno.test("run.ts: a typed tool's run gets the input and a framed ctx", async () => {
  const r = await deno("run.ts", JSON.stringify({ pages: 2, title: "Doc" }), [typed, "/the/cwd"]);
  assertEquals(r.code, 0);
  assertEquals(lastJson(r.stdout), { done: 2, title: "Doc in typed" });
  assertEquals(ctxLines(r.stderr), [
    { log: "starting { pages: 2 }" },
    { progress: 0.5, message: "page 1/2" },
    { progress: 1, message: "page 2/2" },
    { warn: "careful" },
  ]);
  // Plain console.error stays a plain line.
  if (!r.stderr.split("\n").includes("plain stderr")) throw new Error(r.stderr);
});

Deno.test("run.ts: a v1 default export also gets ctx", async () => {
  const r = await deno("run.ts", "{}", [v1Ctx, "/the/cwd"]);
  assertEquals(r.code, 0);
  assertEquals(lastJson(r.stdout), { signal: true, tool: "v1_ctx", cwd: "/the/cwd" });
  assertEquals(ctxLines(r.stderr), [{ progress: null, message: "no fraction" }]);
});

Deno.test("run.ts: SIGTERM aborts ctx.signal and the run still finishes", async () => {
  const child = spawn("run.ts", [typed]);
  const w = child.stdin.getWriter();
  await w.write(new TextEncoder().encode(JSON.stringify({ pages: 1, hang: true })));
  await w.close();
  // Wait for the run to be inside `run` (the "careful" warning), then cancel it.
  const reader = child.stderr.pipeThrough(new TextDecoderStream()).getReader();
  let stderr = "";
  while (!stderr.includes(`{"warn":"careful"}`)) {
    const { value, done } = await reader.read();
    if (done) throw new Error(`stderr closed early: ${stderr}`);
    stderr += value;
  }
  child.kill("SIGTERM");
  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    stderr += value;
  }
  const out = await new Response(child.stdout).text();
  const status = await child.status;
  assertEquals(status.code, 0);
  if (!stderr.includes(`${CTX_SENTINEL}{"warn":"aborted"}`)) throw new Error(stderr);
  assertEquals(lastJson(out), { done: 1, title: "untitled in typed" });
});

Deno.test("extract.ts: prints the tool export after the sentinel", async () => {
  const extract = async (path: string) => {
    const child = new Deno.Command(Deno.execPath(), {
      args: ["run", "--no-prompt", "--quiet", `--allow-read=${here}`, `${here}extract.ts`],
      stdin: "piped",
      stdout: "piped",
    }).spawn();
    const w = child.stdin.getWriter();
    await w.write(new TextEncoder().encode(JSON.stringify({ path })));
    await w.close();
    const out = new TextDecoder().decode((await child.output()).stdout);
    const line = out.split("\n").reverse().find((l) => l.startsWith(EXTRACT_SENTINEL))!;
    return JSON.parse(line.slice(EXTRACT_SENTINEL.length)).tool;
  };
  assertEquals((await extract(echo)).name, "echo");
  // A typed tool prints the same wire shape: name, description, parameters.
  const t = await extract(typed);
  assertEquals(Object.keys(t), ["name", "description", "parameters"]);
  assertEquals(t.parameters.required, ["pages"]);
  assertEquals(t.parameters.properties.pages, { type: "integer", description: "how many pages", minimum: 1 });
});
