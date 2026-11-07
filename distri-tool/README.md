# `@distri/tool`

The script-tool contract: one module that every host running TypeScript tools under Deno
imports. The `distri` CLI runs `.distri/tools/*.ts` and `~/.distri/tools/*.ts` with it;
Distri Builder's `@distri/builder-sdk` re-exports it for `apps/<app>/scripts/*.ts`. The Rust
half (`ToolExport`, the extractor-line parser) is `distri_types::script_tool`.

```ts
// .distri/tools/lookup_order.ts
// ---
// [permissions]
// net = ["localhost:5432"]
// read = ["./fixtures"]
// [tool]
// timeout = 30
// ---
import { defineTool, s } from "jsr:@distri/tool";

export const tool = defineTool({
  name: "lookup_order",
  description: "Fetch an order",
  input: s.object({ id: s.string({ description: "Order id" }), verbose: s.optional(s.boolean()) }),
  output: s.object({ id: s.string(), status: s.enum(["open", "shipped"] as const) }),
  async run({ id }, ctx) {
    ctx.progress(0.5, "querying");
    const res = await fetch(`http://localhost:5432/orders/${id}`, { signal: ctx.signal });
    ctx.log("status", res.status);
    return { id, status: "shipped" };
  },
});
```

## The contract (frozen)

- Input is one JSON document on stdin; output is one JSON document on stdout; logs go to
  stderr. A non-zero exit is a failed call.
- `export const tool = defineTool({ name, description, parameters })` makes the script a tool.
  `parameters` is the JSON Schema of the input; it is what the extractor prints and what the
  hosts read (`distri_types::script_tool::ToolExport`).
- `export default async function (input, ctx)` is the whole script: the host's harness reads
  stdin, calls it and prints the return value. Without a default export (and without
  `tool.run`), read stdin and write stdout yourself.

## v1.1: typed tools

v1 scripts (`parameters` + a default export) keep working unchanged. v1.1 adds:

- **`s`, a schema builder** with no dependencies. Each builder returns `{ schema }` (a JSON
  Schema) that also carries a type, so `Infer<typeof x>` is the TypeScript type of the values it
  accepts:

  | Builder | JSON Schema |
  |---|---|
  | `s.string({ format?, enum?, description?, minLength?, maxLength?, pattern? })` | `{"type": "string", …}` |
  | `s.number({ min?, max?, description? })`, `s.integer(…)` | `{"type": "number" \| "integer", "minimum", "maximum"}` |
  | `s.boolean()` | `{"type": "boolean"}` |
  | `s.array(item)` | `{"type": "array", "items": …}` |
  | `s.object({ …props })` | `{"type": "object", "properties": …, "required": [non-optional props]}` |
  | `s.optional(x)` | `x`, left out of its object's `required` |
  | `s.enum(["a", "b"] as const)` | `{"type": "string", "enum": ["a", "b"]}` |
  | `s.literal(v)` | `{"type": …, "const": v}` |

- **`defineTool({ name, description?, input, output?, run(input, ctx) })`**: `input` must be an
  `s.object(…)`; `defineTool` turns it into `parameters`, so the export the hosts read has the v1
  shape. `run`'s input is typed from `input`, its return value from `output`. The harness calls
  `tool.run` when there is one, else the default export.
- **`FromSchema<typeof schema>`** types a raw JSON Schema written `as const`
  (`object`/`properties`/`required`, `string`, `number`, `integer`, `boolean`, `array`/`items`,
  `enum`, `const`), so a v1 tool can type its input too:
  `defineTool({ name, parameters: {...} as const, run(input: FromSchema<typeof parameters>) {…} })`.
- **`ToolContext`** (`ctx`), the second argument of `run` and of a default export:

  | Field | What |
  |---|---|
  | `signal` | an `AbortSignal` that aborts when the host cancels the call (SIGTERM). The host sends SIGKILL 3s later; return or throw to finish cleanly. |
  | `progress(fraction?, message?)` | `fraction` in 0..=1 when known |
  | `log(...args)`, `warn(...args)` | a log / warning line shown with the call |
  | `tool`, `cwd` | the tool's name; the directory the host runs it in (the project root) |

  `progress`, `log` and `warn` each write one stderr line, `__distri_ctx__ ` + JSON:
  `{"progress":0.62,"message":"page 5/8"}`, `{"log":"text"}`, `{"warn":"text"}`. The CLI shows
  them while the call runs (and so does every other stderr line); they never appear in a failed
  call's error.

## Files

| File | What |
|---|---|
| `mod.ts` | `defineTool`, `s`, `Infer`, `FromSchema`, `ToolContext`, `ToolExport`, `runModule` / `runMain` (the stdin/stdout harness), `EXTRACT_SENTINEL`, `CTX_SENTINEL` |
| `run.ts` | `deno run <permissions> run.ts <script> [cwd]`: the harness entry the CLI uses |
| `extract.ts` | reads `{"path"}` on stdin, imports the script, prints `__distri_tool__ {"tool": …}` (the wire fields only) |
| `mod_test.ts` | `deno test -A` |

Hosts embed these files and map `jsr:@distri/tool` / `@distri/tool` to their copy with an
import map, so a script resolves the module with no network and no package manager.

## Permissions in the CLI

The CLI runs `deno run --no-prompt` with no `--allow-env`, no `--allow-run`, no `--allow-ffi`
and no `--allow-write`. Read is granted to the folder the script lives in. `net` and further
`read` paths come only from the script's frontmatter (approved once per project at the first
call) or from `[tools.<name>]` in the project's `distri.toml` (granted without asking):

```toml
# distri.toml
[tools.lookup_order]
net = ["localhost:5432"]
read = ["./fixtures"]
```

Relative `read` paths resolve against the project root. A script that reaches outside its grant
fails the call with `NotCapable`, naming the permission it lacked.

## Calls in the CLI

- The input is validated against `parameters` before Deno starts. A mismatch fails the call
  with every problem and its JSON pointer:
  `script tool 'lookup_order': input does not match its schema: /id: 17 is not of type "string"`.
- A call may run 120s; `[tool] timeout = N` in the frontmatter or `timeout = N` under
  `[tools.<name>]` in `distri.toml` (which wins) changes that, in seconds.
- A cancelled run sends the script SIGTERM (`ctx.signal` aborts), then SIGKILL 3s later.
- A script that fails to import is reported once in full; while it is unchanged (it and the
  other `*.ts` files in its folder, which it may import), later commands print one line.
- `distri tools check [name]` runs `deno check` (with `@distri/tool` resolved) and checks the
  schema of each script tool: `✓ name  deno check · schema ok` or `✗ name  <first error lines>`,
  exiting non-zero when any fails. `distri tools test <name> '<json>'` validates the input, prints
  `ctx` lines as they arrive and the output document on stdout.
