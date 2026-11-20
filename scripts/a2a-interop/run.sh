#!/usr/bin/env bash
# A2A interop: the official a2a-rs client (`a2acli`) against the OSS distri
# server (sqlite, DISTRI_DURABLE_RUNS=1) with a mock OpenAI-compatible model.
#
#   A2ACLI      path to a2acli (build: in a2aproject/a2a-rs, `cargo build -p a2a-cli --release`)
#   SERVER_BIN  distri-server binary (default: <repo>/target/debug/distri-server;
#               build with `cargo build -p distri-server-cli`)
#   PORT        server port (default 18481); MOCK_LLM_PORT (default 7811)
#   OUT         transcript file (default: scripts/a2a-interop/transcript.txt)
#   EXTRA_CMD   optional command run afterwards against the same server, with
#               DISTRI_BASE_URL and DISTRI_A2A_TEST_AGENT=echo exported, e.g.
#               EXTRA_CMD="cargo test -p distri-cli --test tasks -- --test-threads=1"
#
# Every step must succeed, or fail cleanly per the A2A spec (push configs: the
# card says pushNotifications=false, so a2acli refuses before calling, and the
# server answers -32003 when called directly). Exit status is non-zero on the
# first unexpected result.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
A2ACLI=${A2ACLI:?set A2ACLI to the a2acli binary}
SERVER_BIN=${SERVER_BIN:-$ROOT/target/debug/distri-server}
PORT=${PORT:-18481}
MOCK_LLM_PORT=${MOCK_LLM_PORT:-7811}
OUT=${OUT:-$HERE/transcript.txt}
WORK=$(mktemp -d)
CARD="http://127.0.0.1:$PORT/v1/agents/echo/.well-known/agent-card.json"
BASE="http://127.0.0.1:$PORT/v1"

cleanup() {
  [[ -n ${SERVER_PID:-} ]] && kill "$SERVER_PID" 2>/dev/null
  [[ -n ${MOCK_PID:-} ]] && kill "$MOCK_PID" 2>/dev/null
  wait 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

mkdir -p "$WORK/ws/agents" "$WORK/home"
cat >"$WORK/ws/agents/echo.md" <<AGENT
---
name = "echo"
description = "Answers hello (A2A interop agent)"
version = "1.0.0"
max_iterations = 3
tool_format = "provider"

[model_settings]
model = "mock-1"

[model_settings.provider]
name = "openai_compat"
base_url = "http://127.0.0.1:$MOCK_LLM_PORT/v1"
api_key = "mock"
---
Say hello.
AGENT

MOCK_LLM_PORT=$MOCK_LLM_PORT node "$HERE/mock-llm.mjs" >"$WORK/mock.log" 2>&1 &
MOCK_PID=$!
(
  cd "$WORK/ws"
  HOME="$WORK/home" CURRENT_WORKING_DIR="$WORK/ws" DISTRI_DURABLE_RUNS=1 \
    DISTRI_HOST=127.0.0.1 DISTRI_PORT=$PORT RUST_LOG=${RUST_LOG:-warn} \
    exec "$SERVER_BIN" --headless
) >"$WORK/server.log" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 120); do
  curl -sf -o /dev/null "$CARD" && break
  kill -0 "$SERVER_PID" 2>/dev/null || { cat "$WORK/server.log" >&2; exit 1; }
  sleep 0.5
done
curl -sf -o /dev/null "$CARD" || { echo "server did not come up" >&2; tail -40 "$WORK/server.log" >&2; exit 1; }

: >"$OUT"
FAILED=0
# step <expect: ok|fail> <title> -- command...   (runs a2acli from $WORK: no stray .env)
step() {
  local expect=$1 title=$2; shift 3
  {
    echo "## $title"
    echo "\$ a2acli ${*}"
  } | tee -a "$OUT"
  local output status
  output=$(cd "$WORK" && "$A2ACLI" "$@" 2>&1)
  status=$?
  printf '%s\n[exit %s]\n\n' "$output" "$status" | tee -a "$OUT"
  if [[ $expect == ok && $status -ne 0 ]] || [[ $expect == fail && $status -eq 0 ]]; then
    echo "!! unexpected result for: $title" | tee -a "$OUT"
    FAILED=1
  fi
  LAST_OUTPUT=$output
}
task_id_of() { python3 -c 'import json,sys; d=json.loads(sys.stdin.read()); t=d.get("task", d); print(t.get("id",""))'; }

step ok "card get --validate" -- -a "$CARD" card get --validate
step ok "send (blocking)" -- -a "$CARD" send "hello"
step ok "send --stream" -- -a "$CARD" send "hello again" --stream
step ok "send -o json (task id for the next steps)" -- -a "$CARD" -o json send "and once more"
TASK=$(printf '%s' "$LAST_OUTPUT" | task_id_of)
step ok "task get" -- -a "$CARD" task get "$TASK"
step ok "task list" -- -a "$CARD" task list
step ok "send --async a slow task" -- -a "$CARD" -o json send "[slow] take your time" --async --return-immediately
SLOW=$(printf '%s' "$LAST_OUTPUT" | task_id_of)
step ok "task subscribe (follows the slow task to its end)" -- -a "$CARD" task subscribe "$SLOW"
step ok "send --async another slow task" -- -a "$CARD" -o json send "[slow] cancel me" --async --return-immediately
SLOW2=$(printf '%s' "$LAST_OUTPUT" | task_id_of)
step ok "task cancel (working task)" -- -a "$CARD" task cancel "$SLOW2"
step fail "task cancel (completed task: TaskNotCancelable -32002)" -- -a "$CARD" task cancel "$TASK"
step fail "task push-config list (card: pushNotifications=false)" -- -a "$CARD" task push-config list "$TASK"
step fail "task get (unknown task: TaskNotFound)" -- -a "$CARD" task get does-not-exist

{
  echo "## push-config over the wire (server answer, bypassing a2acli's pre-flight)"
  echo "\$ curl -s $BASE/agents/echo -d '{\"method\":\"ListTaskPushNotificationConfigs\",...}'"
  curl -s "$BASE/agents/echo" -H 'content-type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ListTaskPushNotificationConfigs\",\"params\":{\"taskId\":\"$TASK\"}}"
  echo; echo
} | tee -a "$OUT"

if [[ -n ${EXTRA_CMD:-} ]]; then
  echo "## extra: $EXTRA_CMD" | tee -a "$OUT"
  (cd "$ROOT" && DISTRI_BASE_URL=$BASE DISTRI_A2A_TEST_AGENT=echo DISTRI_A2A_EXPECT_DURABLE=1 \
    bash -c "$EXTRA_CMD") 2>&1 | tail -25 | tee -a "$OUT"
  [[ ${PIPESTATUS[0]} -eq 0 ]] || FAILED=1
fi

if [[ $FAILED -ne 0 ]]; then
  echo "interop: FAILED (see $OUT)" | tee -a "$OUT"
  tail -40 "$WORK/server.log" >&2
  exit 1
fi
echo "interop: OK" | tee -a "$OUT"
