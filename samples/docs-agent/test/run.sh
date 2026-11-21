#!/usr/bin/env bash
# docs-agent end to end (docs/gtm/SAMPLES.md in distri-cloud): the folder
# parses, the blueprint applies, the portal (app/) starts, and one question is
# answered in the browser: the agent calls search_docs and open_page, which run
# in the page. Screenshots and the transcript go to test/out/.
#
#   DISTRI_URL=https://api.distri.dev/v1 DISTRI_API_KEY=… samples/docs-agent/test/run.sh
set -euo pipefail
cd "$(dirname "$0")/.."
: "${DISTRI_URL:?set DISTRI_URL (e.g. https://api.distri.dev/v1)}"
: "${DISTRI_API_KEY:?set DISTRI_API_KEY (a workspace API key)}"
export DISTRI_BASE_URL="$DISTRI_URL"
DISTRI=${DISTRI:-distri}
PORT=${PORT:-5302}
mkdir -p test/out

"$DISTRI" apply . --export > test/out/blueprint.json
"$DISTRI" apply . --set site_url="http://localhost:$PORT" --json > test/out/apply.json
jq -e '.status == "ready" or .status == "pending"' test/out/apply.json > /dev/null

(cd app && npm install --no-audit --no-fund > ../test/out/npm.log 2>&1)
(cd test && npm install --no-audit --no-fund > out/npm-playwright.log 2>&1 && npx playwright install chromium >> out/npm-playwright.log 2>&1)
(cd app && DISTRI_URL="$DISTRI_URL" DISTRI_API_KEY="$DISTRI_API_KEY" DISTRI_WORKSPACE_ID="${DISTRI_WORKSPACE_ID:-}" PORT=$PORT npx vite > ../test/out/vite.log 2>&1) &
VITE=$!
trap 'kill $VITE 2>/dev/null || true; pkill -P $VITE 2>/dev/null || true' EXIT
for _ in $(seq 1 60); do curl -sf -o /dev/null "http://localhost:$PORT/" && break; sleep 1; done

PORTAL="http://localhost:$PORT" OUT=test/out node test/e2e.mjs > test/out/transcript.txt
echo "docs-agent: ok (test/out/)"
