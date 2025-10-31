# distri_runtime

TypeScript runtime utilities used by distri plugins. The package mirrors the interfaces exposed by
the `ts-executor` in the main distri repository and is designed to be published on
[deno.land/x](https://deno.land/x).

## Usage

```ts
import {
  callTool,
  callWorkflow,
  registerAgentHandler,
  registerPlugin,
} from "https://distri.dev/base.ts";
```

See `mod.ts` for the full list of exported types and helpers.

## Local Development

```bash
cd runtime
deno task fmt
deno task lint
```

These tasks rely on `deno.json` and keep the module consistent with Deno formatting and linting
rules.

## Publishing to deno.land/x

1. **Pick a version** – update the `version` field in `deno.json` and ensure `mod.ts` reflects any
   changes.
2. **Tag the repository** – from the repo root run `git tag v0.1.0` (or the new version) and
   `git push origin v0.1.0`.
3. **Register/Update the module** – visit [https://deno.land/x](https://deno.land/x) and add or
   update the `distri_runtime` module pointing at this Git repository. The site will fetch the
   tagged release automatically.
4. **Verify** – import the new tag in a scratch Deno script to make sure the CDN serves the package.

Keep plugin imports pinned to an explicit version, e.g.
`https://distri.dev/base.ts`, so workflow code remains deterministic.
