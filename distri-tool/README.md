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
// ---
import { defineTool } from "jsr:@distri/tool";

export const tool = defineTool({
  name: "lookup_order",
  description: "Fetch an order",
  parameters: { type: "object", properties: { id: { type: "string" } }, required: ["id"] },
});

export default async function ({ id }: { id: string }) {
  return { id, status: "shipped" };
}
```

## The contract (frozen)

- Input is one JSON document on stdin; output is one JSON document on stdout; logs go to
  stderr. A non-zero exit is a failed call.
- `export default async function (input)` is the whole script: the host's harness reads stdin,
  calls it and prints the return value. Without a default export, read stdin and write stdout
  yourself.
- `export const tool = defineTool({ name, description, parameters })` makes the script a tool.
  `parameters` is the JSON Schema of the input.

## Files

| File | What |
|---|---|
| `mod.ts` | `defineTool`, `ToolExport`, `runModule` / `runMain` (the stdin/stdout harness), `EXTRACT_SENTINEL` |
| `run.ts` | `deno run <permissions> run.ts <script>`: the harness entry the CLI uses |
| `extract.ts` | reads `{"path"}` on stdin, imports the script, prints `__distri_tool__ {"tool": …}` |
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
