#!/bin/bash
set -euo pipefail
#
# Distri CLI integration tests
# Runs the CLI binary against the live Distri Cloud API.
#
# Required env vars:
#   BINARY_PATH           — path to the distri binary under test
#   DISTRI_API_KEY        — API key for distri cloud
#   DISTRI_BASE_URL       — API base URL (https://api.distri.dev/v1)
#   DISTRI_WORKSPACE_ID   — workspace ID for testing
#
# Optional env vars:
#   SKIP_RUN_TESTS        — set to 1 to skip agent run tests (require LLM keys)
#
# Usage:
#   BINARY_PATH=./distri ./integration_tests.sh
#   # or via release.sh which sets BINARY_PATH automatically

DISTRI="${BINARY_PATH:-./target/debug/distri}"

if [[ ! -f "${DISTRI}" ]]; then
  echo "ERROR: Binary not found at ${DISTRI}"
  echo "Set BINARY_PATH to the distri binary."
  exit 1
fi

# Required env vars
: "${DISTRI_API_KEY:?DISTRI_API_KEY must be set}"
: "${DISTRI_BASE_URL:?DISTRI_BASE_URL must be set}"
: "${DISTRI_WORKSPACE_ID:?DISTRI_WORKSPACE_ID must be set}"

PASSED=0
FAILED=0
SKIPPED=0
ERRORS=""

# -- Test helpers --------------------------------------------------------------
run_test() {
  local name="$1"
  shift
  echo -n "  ${name}... "

  local output
  local exit_code=0
  output=$("$@" 2>&1) || exit_code=$?

  if [[ ${exit_code} -eq 0 ]]; then
    echo "OK"
    PASSED=$((PASSED + 1))
  else
    echo "FAIL (exit ${exit_code})"
    ERRORS="${ERRORS}\n  FAIL: ${name}\n    Command: $*\n    Output: ${output}\n"
    FAILED=$((FAILED + 1))
  fi
}

run_test_contains() {
  local name="$1"
  local expected="$2"
  shift 2
  echo -n "  ${name}... "

  local output
  local exit_code=0
  output=$("$@" 2>&1) || exit_code=$?

  if [[ ${exit_code} -ne 0 ]]; then
    echo "FAIL (exit ${exit_code})"
    ERRORS="${ERRORS}\n  FAIL: ${name}\n    Command: $*\n    Output: ${output}\n"
    FAILED=$((FAILED + 1))
  elif echo "${output}" | grep -qi "${expected}"; then
    echo "OK"
    PASSED=$((PASSED + 1))
  else
    echo "FAIL (expected '${expected}' not found)"
    ERRORS="${ERRORS}\n  FAIL: ${name}\n    Expected: ${expected}\n    Output: ${output}\n"
    FAILED=$((FAILED + 1))
  fi
}

skip_test() {
  local name="$1"
  local reason="$2"
  echo "  ${name}... SKIP (${reason})"
  SKIPPED=$((SKIPPED + 1))
}

echo "=== Distri CLI Integration Tests ==="
echo "Binary:    ${DISTRI}"
echo "API:       ${DISTRI_BASE_URL}"
echo "Workspace: ${DISTRI_WORKSPACE_ID}"
echo ""

# -- Setup: configure the CLI --------------------------------------------------
echo "[Setup] Configuring CLI..."
"${DISTRI}" config set api_key "${DISTRI_API_KEY}" 2>/dev/null || true
"${DISTRI}" config set base_url "${DISTRI_BASE_URL}" 2>/dev/null || true
"${DISTRI}" config set workspace_id "${DISTRI_WORKSPACE_ID}" 2>/dev/null || true
echo ""

# -- Version -------------------------------------------------------------------
echo "[1] Version"
run_test_contains "distri --version prints version" "distri" "${DISTRI}" --version
echo ""

# -- Config --------------------------------------------------------------------
echo "[2] Config"
run_test "config set api_key" "${DISTRI}" config set api_key "${DISTRI_API_KEY}"
run_test "config set base_url" "${DISTRI}" config set base_url "${DISTRI_BASE_URL}"
run_test "config set workspace_id" "${DISTRI}" config set workspace_id "${DISTRI_WORKSPACE_ID}"
run_test_contains "config set confirms update" "Updated" "${DISTRI}" config set api_key "${DISTRI_API_KEY}"
echo ""

# -- Agents --------------------------------------------------------------------
echo "[3] Agents"
run_test "agents list" "${DISTRI}" agents list
echo ""

# -- Tools ---------------------------------------------------------------------
echo "[4] Tools"
run_test "tools list" "${DISTRI}" tools list
echo ""

# -- Prompts -------------------------------------------------------------------
echo "[5] Prompts"
run_test "prompts list" "${DISTRI}" prompts list
echo ""

# -- Skills --------------------------------------------------------------------
echo "[6] Skills"
run_test "skills list" "${DISTRI}" skills list
echo ""

# -- Run agent -----------------------------------------------------------------
echo "[7] Run agent"
if [[ "${SKIP_RUN_TESTS:-0}" == "1" ]]; then
  skip_test "run with task" "SKIP_RUN_TESTS=1 — requires OPENAI_API_KEY in workspace secrets"
else
  run_test "run with task" "${DISTRI}" run --task "Say hello in exactly 3 words"
fi
echo ""

# -- Help / usage --------------------------------------------------------------
echo "[8] Help & usage"
run_test_contains "distri --help shows commands" "Commands" "${DISTRI}" --help
run_test_contains "agents --help" "Agent" "${DISTRI}" agents --help
run_test_contains "tools --help" "Tool" "${DISTRI}" tools --help
run_test_contains "prompts --help" "Prompt" "${DISTRI}" prompts --help
run_test_contains "config --help" "config" "${DISTRI}" config --help
echo ""

# -- Results -------------------------------------------------------------------
echo "========================================"
TOTAL=$((PASSED + FAILED))
echo "  ${PASSED}/${TOTAL} passed, ${SKIPPED} skipped"
if [[ ${FAILED} -gt 0 ]]; then
  echo ""
  echo "Failures:"
  echo -e "${ERRORS}"
  exit 1
fi
echo "========================================"
