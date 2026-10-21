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
// The Rust half (the `ToolExport` struct and the extractor-line parser) is
// `distri_types::script_tool`.

/** The `export const tool = defineTool({…})` shape (`distri_types::script_tool::ToolExport`). */
export interface ToolExport {
  name: string;
  description?: string;
  /** JSON Schema of the input document. */
  parameters?: Record<string, unknown>;
}

/** Declare the script as a tool agents may call. Returns its argument, typed. */
export function defineTool(tool: ToolExport): ToolExport {
  if (!tool || typeof tool.name !== "string" || tool.name.length === 0) {
    throw new Error("defineTool: `name` is required");
  }
  return tool;
}

/** The prefix of the extractor's result line (`distri_types::script_tool::EXTRACT_SENTINEL`). */
export const EXTRACT_SENTINEL = "__distri_tool__ ";

/** The input document: all of stdin as JSON, `null` when stdin is empty. */
export async function readInput(): Promise<unknown> {
  const text = await new Response(Deno.stdin.readable).text();
  return text.trim() ? JSON.parse(text) : null;
}

/** Print the output document (`undefined` is `null`) on stdout. */
export async function writeOutput(out: unknown): Promise<void> {
  await Deno.stdout.write(new TextEncoder().encode(JSON.stringify(out === undefined ? null : out)));
}

/**
 * Run one imported script module under the contract: when it has a default export, read the
 * input from stdin, call it, print the result. Returns `false` when the module has no default
 * function (the script drove stdin/stdout itself while being imported).
 */
export async function runModule(mod: Record<string, unknown>): Promise<boolean> {
  const main = mod.default;
  if (typeof main !== "function") return false;
  const input = await readInput();
  const out = await main(input);
  await writeOutput(out);
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
