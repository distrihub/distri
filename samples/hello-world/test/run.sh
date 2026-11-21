#!/usr/bin/env bash
# hello-world end to end (docs/gtm/SAMPLES.md in distri-cloud):
# the folder parses, the blueprint applies, its test workflow runs the agent,
# and `distri run` gets a reply. Writes test/out/.
#
#   DISTRI_URL=https://api.distri.dev/v1 DISTRI_API_KEY=… samples/hello-world/test/run.sh
set -euo pipefail
cd "$(dirname "$0")/.."
: "${DISTRI_URL:?set DISTRI_URL (e.g. https://api.distri.dev/v1)}"
: "${DISTRI_API_KEY:?set DISTRI_API_KEY (a workspace API key)}"
export DISTRI_BASE_URL="$DISTRI_URL"
DISTRI=${DISTRI:-distri}
mkdir -p test/out

# Offline: the folder is a valid blueprint.
"$DISTRI" apply . --export > test/out/blueprint.json

# Live: apply, then run the blueprint's test (the say_hello workflow).
"$DISTRI" apply . --set name=Ada --json --test > test/out/apply.json
jq -e '.test.ok == true' test/out/apply.json > /dev/null || { jq '.test.message' test/out/apply.json; exit 1; }
jq -r '.test.summary.steps[] | select(.id == "greet") | .result.output // .output // empty' test/out/apply.json > test/out/greeting.txt || true

# The agent on its own.
"$DISTRI" run --agent hello --task "Say hello to Ada" > test/out/run.txt
test -s test/out/run.txt
echo "hello-world: ok (test/out/)"
