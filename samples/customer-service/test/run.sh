#!/usr/bin/env bash
# customer-service end to end (docs/gtm/SAMPLES.md in distri-cloud): the folder
# parses, the blueprint applies, and its test message runs through triage,
# reply and gist. Telegram delivery is recorded, not required: until a person
# connects Telegram (test/case.md) the notify step fails with "no channel for
# the owner". Writes test/out/.
#
#   DISTRI_URL=https://api.distri.dev/v1 DISTRI_API_KEY=… samples/customer-service/test/run.sh
set -euo pipefail
cd "$(dirname "$0")/.."
: "${DISTRI_URL:?set DISTRI_URL (e.g. https://api.distri.dev/v1)}"
: "${DISTRI_API_KEY:?set DISTRI_API_KEY (a workspace API key)}"
export DISTRI_BASE_URL="$DISTRI_URL"
DISTRI=${DISTRI:-distri}
mkdir -p test/out

"$DISTRI" apply . --export > test/out/blueprint.json

# The notify steps continue on failure, so the run finishes without Telegram;
# the checks below read the JSON rather than the exit code.
"$DISTRI" apply . --set business_name="Acme Coffee" --json --test > test/out/apply.json || true
jq -e '.test.summary.steps' test/out/apply.json > /dev/null || { cat test/out/apply.json; exit 1; }

handle() { jq -r --arg k "$1" '.test.summary.steps[] | select(.id == "handle") | .result[$k] // empty' test/out/apply.json; }
st=$(jq -r '.test.summary.steps[] | select(.id == "handle") | .status' test/out/apply.json)
[ "$st" = "completed" ] || { echo "step handle: $st"; jq '.test.message' test/out/apply.json; exit 1; }
handle decision > test/out/decision.txt
handle reply > test/out/reply.txt
handle gist > test/out/gist.txt
[ "$(cat test/out/decision.txt)" = "escalate" ] || { echo "the refund was not escalated: $(cat test/out/decision.txt)"; exit 1; }
test -s test/out/reply.txt && test -s test/out/gist.txt

# Delivered on Telegram, or why not (no chat connected until a person connects it).
jq -r '.test.summary.steps[] | select(.id == "escalate") | "escalate: \(.status) \(.error // "delivered")"' test/out/apply.json | tee test/out/telegram.txt
echo "customer-service: ok (test/out/)"
