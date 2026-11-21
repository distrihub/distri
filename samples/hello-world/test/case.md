# hello-world: the use case

1. `distri apply samples/hello-world --test` against a workspace with a model.
2. It prints the agent and the workflow it created, then `✓ Test run finished`.
3. `distri run --agent hello --task "Say hello to Ada"` prints a one-sentence greeting that names Ada.

Nothing here needs a person: `test/run.sh` runs all of it.
