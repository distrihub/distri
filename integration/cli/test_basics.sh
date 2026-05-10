#!/usr/bin/env bash
# CLI smoke tests — these run against any server (mock or real) and
# only assert on CLI behavior, not LLM output.
source "$(dirname "$0")/../scripts/lib.sh"
require_binary

echo "=== CLI basics ==="
run_test_contains "distri --version"   "distri"   "${DISTRI_BIN}" version
run_test_contains "distri --help"      "Commands" "${DISTRI_BIN}" --help
run_test_contains "agents --help"      "agent"    "${DISTRI_BIN}" agents --help
run_test_contains "tools --help"       "tool"     "${DISTRI_BIN}" tools --help
run_test_contains "profile --help"     "profile"  "${DISTRI_BIN}" profile --help

echo ""
echo "=== Listing endpoints (require server) ==="
require_server
# Auth comes from env vars (DISTRI_API_KEY, DISTRI_BASE_URL,
# DISTRI_WORKSPACE_ID) so callers don't need a populated profile.
run_test "agents list"  "${DISTRI_BIN}" agents list
run_test "tools list"   "${DISTRI_BIN}" tools list
run_test "prompts list" "${DISTRI_BIN}" prompts list

summary
