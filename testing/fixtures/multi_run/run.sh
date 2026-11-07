#!/usr/bin/env bash
# Multi-run fixture (#170): push the three agents and start a run that forks
# two background children; multi_run_asker then waits on a person.
#
#   DISTRI_BASE_URL=http://localhost:1341/v1 DISTRI_API_KEY=… DISTRI_WORKSPACE_ID=… ./run.sh
#   DISTRI_BASE_URL=http://127.0.0.1:8081/v1 ./run.sh      # local distri-server, no auth
#   distri sessions        # in another terminal: attach, answer, detach
#
# The run is started over A2A (not `distri run`) so the ask_human tool is
# advertised with no client handler anywhere: only a person can answer it,
# which makes the input-required child deterministic.
set -euo pipefail
cd "$(dirname "$0")"
: "${DISTRI_BASE_URL:?set DISTRI_BASE_URL (e.g. http://localhost:1341/v1)}"
DISTRI="${DISTRI_BIN:-distri}"

"$DISTRI" agents push agents --all

thread_id="$(uuidgen | tr 'A-Z' 'a-z')"
message_id="$(uuidgen | tr 'A-Z' 'a-z')"
sed -e "s/__THREAD_ID__/$thread_id/" -e "s/__MESSAGE_ID__/$message_id/" start_request.json >"/tmp/multi_run_$thread_id.json"

echo "thread $thread_id — streaming the parent (its children keep running after it ends)"
curl -sN -X POST "$DISTRI_BASE_URL/agents/multi_run_parent" \
  ${DISTRI_API_KEY:+-H "Authorization: Bearer $DISTRI_API_KEY"} \
  ${DISTRI_WORKSPACE_ID:+-H "X-Workspace-Id: $DISTRI_WORKSPACE_ID"} \
  -H "Content-Type: application/json" \
  -H "Accept: text/event-stream" \
  --data @"/tmp/multi_run_$thread_id.json" | grep -o '"state":"[a-z-]*"' | uniq
echo "parent stream closed — open \`distri sessions\` (or \`distri sessions --thread $thread_id\`)"
