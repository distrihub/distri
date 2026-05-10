#!/usr/bin/env bash
# CLI smoke tests — these run against any server (mock or real) and
# only assert on CLI behavior, not LLM output.
source "$(dirname "$0")/../scripts/lib.sh"
require_binary

echo "=== CLI basics ==="
run_test_contains "distri --version" "distri" "${DISTRI_BIN}" --version
run_test_contains "distri --help"    "Commands" "${DISTRI_BIN}" --help
run_test_contains "agents --help"    "Agent" "${DISTRI_BIN}" agents --help
run_test_contains "tools --help"     "Tool"  "${DISTRI_BIN}" tools --help
run_test_contains "config --help"    "config" "${DISTRI_BIN}" config --help

echo ""
echo "=== Config ==="
run_test "config set api_key"      "${DISTRI_BIN}" config set api_key "${DISTRI_API_KEY}"
run_test "config set base_url"     "${DISTRI_BIN}" config set base_url "${DISTRI_BASE_URL}"
run_test "config set workspace_id" "${DISTRI_BIN}" config set workspace_id "${DISTRI_WORKSPACE_ID}"

echo ""
echo "=== Listing endpoints (require server) ==="
require_server
run_test "agents list"  "${DISTRI_BIN}" agents list
run_test "tools list"   "${DISTRI_BIN}" tools list
run_test "prompts list" "${DISTRI_BIN}" prompts list

summary
