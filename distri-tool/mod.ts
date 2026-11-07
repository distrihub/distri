// @distri/tool — the script-tool contract, shared by every host that runs TypeScript tools
// under Deno: the `distri` CLI (`.distri/tools/*.ts`, `~/.distri/tools/*.ts`) and Distri
// Builder (`apps/<app>/scripts/*.ts`, through `@distri/builder-sdk`). One copy; both import it.
//
// The contract (frozen):
// - input is one JSON document on stdin; output is one JSON document on stdout;
// - logs go to stderr; a non-zero exit is a failed call;
// - `export const tool = defineTool({ name, description, parameters })` makes the script a tool
//   (`parameters` is the JSON Schema of the input document);
// - `export default async function (input) {…}` is the whole script: a host's harness
//   (`run.ts`, or the builder SDK's run hook through `runMain`) reads stdin, calls it and prints
//   its return value. A script without a default export reads stdin and writes stdout itself.
//
// v1.1 adds, without changing the above:
// - `defineTool({ name, input: s.object({…}), run(input, ctx) {…} })`: a typed tool. `input` is
//   built with the `s` schema builder and becomes `parameters`; the harness calls `tool.run`.
// - `ToolContext` (`ctx`): an abort `signal` (SIGTERM, when the host cancels the call), and
//   `progress` / `log` / `warn`, each one stderr line `__distri_ctx__ {json}` the host shows
//   while the call runs.
//
// The Rust half (the `ToolExport` struct and the extractor-line parser) is
// `distri_types::script_tool`.

/** A JSON Schema document. */
export type JsonSchema = { [key: string]: unknown };

/** The `export const tool = defineTool({…})` shape (`distri_types::script_tool::ToolExport`). */
export interface ToolExport {
  name: string;
  description?: string;
  /** JSON Schema of the input document. */
  parameters?: JsonSchema;
}

// ---------------------------------------------------------------------------------------------
// The schema builder: `s.object({ id: s.string() })` is a JSON Schema plus the TypeScript type
// of the documents it accepts (`Infer<typeof x>`).
// ---------------------------------------------------------------------------------------------

declare const phantom: unique symbol;

/** A JSON Schema (`.schema`) that carries the type `T` of the values it accepts. */
export interface Schema<T> {
  readonly schema: JsonSchema;
  readonly optional?: false;
  /** Type-level only; never set. */
  readonly [phantom]?: T;
}

/** A property `s.object` leaves out of `required`. */
export interface OptionalSchema<T> {
  readonly schema: JsonSchema;
  readonly optional: true;
  readonly [phantom]?: T;
}

type AnySchema = Schema<unknown> | OptionalSchema<unknown>;

/** The TypeScript type a schema accepts. */
export type Infer<S> = S extends Schema<infer T> ? T : S extends OptionalSchema<infer T> ? T | undefined
: never;

type Simplify<T> = { [K in keyof T]: T[K] } & {};

type ObjectOf<P extends Record<string, AnySchema>> = Simplify<
  & { [K in keyof P as P[K] extends OptionalSchema<unknown> ? never : K]: Infer<P[K]> }
  & {
    [K in keyof P as P[K] extends OptionalSchema<unknown> ? K : never]?: P[K] extends OptionalSchema<infer T>
      ? T
      : never;
  }
>;

export interface Described {
  description?: string;
}
export interface StringOptions extends Described {
  format?: string;
  enum?: readonly string[];
  minLength?: number;
  maxLength?: number;
  pattern?: string;
}
export interface NumberOptions extends Described {
  min?: number;
  max?: number;
}
export interface ArrayOptions extends Described {
  minItems?: number;
  maxItems?: number;
}
export interface ObjectOptions extends Described {
  /** `false` rejects properties not listed. Default: not set (allowed). */
  additionalProperties?: boolean;
}

type Literal = string | number | boolean | null;

function schema<T>(body: JsonSchema): Schema<T> {
  return { schema: body };
}

/** Drop `undefined` values, so the schema has only what was set. */
function defined(body: Record<string, unknown>): JsonSchema {
  const out: JsonSchema = {};
  for (const [k, v] of Object.entries(body)) if (v !== undefined) out[k] = v;
  return out;
}

function jsonType(v: Literal): string {
  if (v === null) return "null";
  if (typeof v === "number") return Number.isInteger(v) ? "integer" : "number";
  return typeof v;
}

function numberSchema(type: "number" | "integer", o: NumberOptions = {}): Schema<number> {
  return schema(defined({ type, description: o.description, minimum: o.min, maximum: o.max }));
}

/** The schema builder. */
export const s = {
  string(o: StringOptions = {}): Schema<string> {
    return schema(defined({
      type: "string",
      description: o.description,
      format: o.format,
      enum: o.enum ? [...o.enum] : undefined,
      minLength: o.minLength,
      maxLength: o.maxLength,
      pattern: o.pattern,
    }));
  },
  number(o?: NumberOptions): Schema<number> {
    return numberSchema("number", o);
  },
  integer(o?: NumberOptions): Schema<number> {
    return numberSchema("integer", o);
  },
  boolean(o: Described = {}): Schema<boolean> {
    return schema(defined({ type: "boolean", description: o.description }));
  },
  array<I extends Schema<unknown>>(item: I, o: ArrayOptions = {}): Schema<Infer<I>[]> {
    return schema(defined({
      type: "array",
      items: item.schema,
      description: o.description,
      minItems: o.minItems,
      maxItems: o.maxItems,
    }));
  },
  object<P extends Record<string, AnySchema>>(props: P, o: ObjectOptions = {}): Schema<ObjectOf<P>> {
    const properties: Record<string, JsonSchema> = {};
    const required: string[] = [];
    for (const [k, v] of Object.entries(props)) {
      properties[k] = v.schema;
      if (!v.optional) required.push(k);
    }
    return schema(defined({
      type: "object",
      description: o.description,
      properties,
      required,
      additionalProperties: o.additionalProperties,
    }));
  },
  optional<T>(x: Schema<T>): OptionalSchema<T> {
    return { schema: x.schema, optional: true };
  },
  enum<const V extends readonly [Literal, ...Literal[]]>(values: V, o: Described = {}): Schema<V[number]> {
    const types = [...new Set(values.map(jsonType).map((t) => (t === "integer" ? "number" : t)))];
    return schema(defined({
      type: types.length === 1 ? types[0] : undefined,
      enum: [...values],
      description: o.description,
    }));
  },
  literal<const V extends Literal>(value: V, o: Described = {}): Schema<V> {
    return schema(defined({ type: jsonType(value), const: value, description: o.description }));
  },
};

// ---------------------------------------------------------------------------------------------
// `FromSchema<typeof schema>`: the type of a raw JSON Schema written `as const`.
// ---------------------------------------------------------------------------------------------

type RequiredKeys<S> = S extends { required: readonly (infer R)[] } ? R : never;

type FromObject<S> = S extends { properties: infer P } ? Simplify<
    & { -readonly [K in keyof P as K extends RequiredKeys<S> ? K : never]: FromSchema<P[K]> }
    & { -readonly [K in keyof P as K extends RequiredKeys<S> ? never : K]?: FromSchema<P[K]> }
  >
  : Record<string, unknown>;

/** The TypeScript type of the documents a JSON Schema (written `as const`) accepts. */
export type FromSchema<S> = S extends { enum: readonly (infer E)[] } ? E
  : S extends { const: infer C } ? C
  : S extends { type: "object" } ? FromObject<S>
  : S extends { type: "array"; items: infer I } ? FromSchema<I>[]
  : S extends { type: "array" } ? unknown[]
  : S extends { type: "string" } ? string
  : S extends { type: "number" | "integer" } ? number
  : S extends { type: "boolean" } ? boolean
  : S extends { type: "null" } ? null
  : unknown;

// ---------------------------------------------------------------------------------------------
// The tool context.
// ---------------------------------------------------------------------------------------------

/** The prefix of a context line on stderr (`distri_cli::script_tools::runner::CTX_SENTINEL`). */
export const CTX_SENTINEL = "__distri_ctx__ ";

/** What a tool's `run` gets next to its input. */
export interface ToolContext {
  /** Aborts when the host cancels the call (SIGTERM). The host kills the process 3s later. */
  signal: AbortSignal;
  /** Report progress: `fraction` in 0..=1 when known, and/or a message. */
  progress(fraction?: number, message?: string): void;
  /** A log line the host shows with the call. */
  log(...args: unknown[]): void;
  /** A warning line the host shows with the call. */
  warn(...args: unknown[]): void;
  /** The tool's name. */
  tool: string;
  /** The directory the host runs the tool in (the project root for the CLI). */
  cwd: string;
}

/** One context line: `{"progress": 0.62, "message": "page 5/8"}`, `{"log": "…"}`, `{"warn": "…"}`. */
export type CtxLine =
  | { progress: number | null; message?: string }
  | { log: string }
  | { warn: string };

const encoder = new TextEncoder();

function writeErrLine(line: string): void {
  const bytes = encoder.encode(line + "\n");
  let at = 0;
  while (at < bytes.length) at += Deno.stderr.writeSync(bytes.subarray(at));
}

/** Write one context line on stderr. */
export function writeCtxLine(line: CtxLine): void {
  writeErrLine(CTX_SENTINEL + JSON.stringify(line));
}

function format(args: unknown[]): string {
  return args.map((a) => (typeof a === "string" ? a : Deno.inspect(a, { colors: false }))).join(" ");
}

function safeCwd(): string {
  try {
    return Deno.cwd();
  } catch {
    // `Deno.cwd()` needs read access to the cwd, which a sandboxed tool may not have.
    return ".";
  }
}

/** A context whose `signal` aborts on SIGTERM. Call `close()` when the run is over. */
export function createContext(tool: string, cwd?: string): { ctx: ToolContext; close(): void } {
  const controller = new AbortController();
  const onTerm = () => controller.abort(new DOMException("the call was cancelled", "AbortError"));
  let listening = false;
  try {
    Deno.addSignalListener("SIGTERM", onTerm);
    listening = true;
  } catch {
    // No SIGTERM on this platform (Windows): the host kills the process instead.
  }
  const ctx: ToolContext = {
    signal: controller.signal,
    progress(fraction?: number, message?: string) {
      const line: CtxLine = { progress: typeof fraction === "number" ? fraction : null };
      if (message !== undefined) line.message = message;
      writeCtxLine(line);
    },
    log(...args: unknown[]) {
      writeCtxLine({ log: format(args) });
    },
    warn(...args: unknown[]) {
      writeCtxLine({ warn: format(args) });
    },
    tool,
    cwd: cwd ?? safeCwd(),
  };
  return {
    ctx,
    close() {
      if (listening) {
        listening = false;
        try {
          Deno.removeSignalListener("SIGTERM", onTerm);
        } catch {
          // Already gone.
        }
      }
    },
  };
}

// ---------------------------------------------------------------------------------------------
// defineTool.
// ---------------------------------------------------------------------------------------------

type Awaitable<T> = T | Promise<T>;

/** v1: `parameters` is a JSON Schema; `run` (optional) gets the input and the context. */
export interface ToolDefinitionV1<P extends JsonSchema = JsonSchema> extends ToolExport {
  parameters?: P;
  run?: (input: FromSchema<P>, ctx: ToolContext) => Awaitable<unknown>;
}

/** v1.1: `input` (and `output`) are `s` schemas; `run` is typed by them. `O` is the output type. */
export interface TypedToolDefinition<I extends Schema<unknown>, O> {
  name: string;
  description?: string;
  input: I;
  output?: Schema<O>;
  // `O` is inferred from `output` and from what `run` returns: a field of an incompatible type
  // is an error, and a literal `run` returns for an `s.enum` field needs no `as const`.
  run(input: Infer<I>, ctx: ToolContext): Awaitable<O>;
}

/** What `defineTool` returns for a typed definition: the export plus `run`. */
export interface TypedTool<I extends Schema<unknown>, O> extends ToolExport {
  parameters: JsonSchema;
  input: I;
  output?: Schema<O>;
  run(input: Infer<I>, ctx: ToolContext): Awaitable<O>;
}

/** A tool export as a host sees it: the export plus, maybe, `run`. */
export interface RunnableTool extends ToolExport {
  run?: (input: unknown, ctx: ToolContext) => Awaitable<unknown>;
}

/** Declare the script as a tool agents may call. */
export function defineTool<I extends Schema<unknown>, O = unknown>(
  tool: TypedToolDefinition<I, O>,
): TypedTool<I, O>;
export function defineTool<const P extends JsonSchema>(tool: ToolDefinitionV1<P>): ToolDefinitionV1<P>;
export function defineTool(
  tool: ToolDefinitionV1 | TypedToolDefinition<Schema<unknown>, unknown>,
): RunnableTool {
  if (!tool || typeof tool.name !== "string" || tool.name.length === 0) {
    throw new Error("defineTool: `name` is required");
  }
  if ("input" in tool && tool.input !== undefined) {
    if ("parameters" in tool && (tool as ToolDefinitionV1).parameters !== undefined) {
      throw new Error("defineTool: give `input` or `parameters`, not both");
    }
    const input = tool.input as Schema<unknown>;
    if (!input || typeof input.schema !== "object" || input.schema.type !== "object") {
      throw new Error("defineTool: `input` must be s.object({...})");
    }
    if (typeof tool.run !== "function") throw new Error("defineTool: `run` is required with `input`");
    return { ...tool, parameters: input.schema } as RunnableTool;
  }
  return tool as RunnableTool;
}

/** The wire shape of a tool export: what the extractor prints (no `run`, no `input`). */
export function toolExport(tool: unknown): ToolExport | null {
  if (tool === undefined || tool === null) return null;
  const t = tool as ToolExport;
  return defined({
    name: t.name,
    description: t.description,
    parameters: t.parameters,
  }) as unknown as ToolExport;
}

/** The prefix of the extractor's result line (`distri_types::script_tool::EXTRACT_SENTINEL`). */
export const EXTRACT_SENTINEL = "__distri_tool__ ";

// ---------------------------------------------------------------------------------------------
// The harness.
// ---------------------------------------------------------------------------------------------

/** The input document: all of stdin as JSON, `null` when stdin is empty. */
export async function readInput(): Promise<unknown> {
  const text = await new Response(Deno.stdin.readable).text();
  return text.trim() ? JSON.parse(text) : null;
}

/** Print the output document (`undefined` is `null`) on stdout. */
export async function writeOutput(out: unknown): Promise<void> {
  await Deno.stdout.write(encoder.encode(JSON.stringify(out === undefined ? null : out)));
}

/** Where the harness runs a module: the tool's name and the host's cwd, when known. */
export interface RunOptions {
  tool?: string;
  cwd?: string;
}

/**
 * Run one imported script module under the contract: with `tool.run` (or else a default export),
 * read the input from stdin, call it with the input and a `ToolContext`, print the result.
 * Returns `false` when the module has neither (the script drove stdin/stdout itself while being
 * imported).
 */
export async function runModule(mod: Record<string, unknown>, opts: RunOptions = {}): Promise<boolean> {
  const tool = mod.tool as RunnableTool | undefined;
  const main = typeof tool?.run === "function"
    ? (input: unknown, ctx: ToolContext) => tool.run!(input, ctx)
    : mod.default;
  if (typeof main !== "function") return false;
  const input = await readInput();
  const name = opts.tool ?? (typeof tool?.name === "string" ? tool.name : "");
  const { ctx, close } = createContext(name, opts.cwd);
  try {
    const out = await main(input, ctx);
    await writeOutput(out);
  } finally {
    close();
  }
  return true;
}

/** Print an error the way the hosts read it (`<Name>: <message>` first, then the stack) and exit 1. */
export function fail(e: unknown): never {
  console.error(e instanceof Error ? (e.stack ?? `${e.name}: ${e.message}`) : String(e));
  Deno.exit(1);
}

/**
 * The run hook for a script that is itself the main module (the builder's jobs): import
 * `Deno.mainModule` and run it with `runModule`. Call it once the whole module graph has
 * evaluated (e.g. from a `setTimeout(…, 0)`). Errors exit 1 with the error on stderr.
 */
export async function runMain(): Promise<void> {
  try {
    await runModule(await import(Deno.mainModule));
  } catch (e) {
    fail(e);
  }
}
