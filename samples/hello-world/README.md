# Hello world

> **"Create an agent that says hello."**

The smallest blueprint: one agent (`hello`) and a one-step workflow (`say_hello`) that asks it for a greeting. Apply it to check that your install, your workspace and your model work.

## Run it

```bash
distri login
distri apply samples/hello-world --set name=Ada --test
distri run --agent hello --task "Say hello to Ada"
```

`apply` prints what it created and runs the blueprint's test (this run was against staging, `app-dev.distri.dev`):

```
Hello world: Ready.

  created   agent          hello  https://app-dev.distri.dev/agents/hello
  created   workflow       say_hello  https://app-dev.distri.dev/agents/say_hello

Then: distri apply … --test  →  The run completes and steps.greet.output holds the agent's greeting
Running the test…
✓ Test run finished. Expect: The run completes and steps.greet.output holds the agent's greeting
```

`distri run` streams the agent's reply to your terminal.

## Files

| File | What it is |
|---|---|
| [`blueprint.toml`](blueprint.toml) | Name, the sentence, the `name` param (default `world`), the test |
| [`agents/hello.md`](agents/hello.md) | The agent: TOML front matter, then its instructions |
| [`workflows/say_hello.json`](workflows/say_hello.json) | One `agent_run` step |

## Test

`DISTRI_URL=… DISTRI_API_KEY=… test/run.sh` applies the folder, runs the test workflow and `distri run`, and writes `test/out/`. Set `DISTRI_WORKSPACE_ID` too when your key is not tied to one workspace.
