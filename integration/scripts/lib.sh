# Shared helpers for integration tests. Source from each test_*.sh:
#   source "$(dirname "$0")/../scripts/lib.sh"

set -euo pipefail

# Resolve repo root regardless of where the test was invoked from.
INT_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
INT_DIR="${INT_REPO_ROOT}/integration"

# Load .env if present. Tests must not fail just because .env is absent —
# they should skip the gated parts instead.
if [[ -f "${INT_DIR}/.env" ]]; then
  set -a; source "${INT_DIR}/.env"; set +a
fi

DISTRI_BIN="${DISTRI_BIN:-${INT_REPO_ROOT}/target/debug/distri}"

# Counters
PASSED=0
FAILED=0
SKIPPED=0
ERRORS=""

run_test() {
  local name="$1"; shift
  echo -n "  ${name}... "
  local output exit_code=0
  output=$("$@" 2>&1) || exit_code=$?
  if [[ ${exit_code} -eq 0 ]]; then
    echo "OK"; PASSED=$((PASSED + 1))
  else
    echo "FAIL (exit ${exit_code})"
    ERRORS="${ERRORS}\n  FAIL: ${name}\n    Command: $*\n    Output: ${output}\n"
    FAILED=$((FAILED + 1))
  fi
}

run_test_contains() {
  local name="$1" expected="$2"; shift 2
  echo -n "  ${name}... "
  local output exit_code=0
  output=$("$@" 2>&1) || exit_code=$?
  if [[ ${exit_code} -ne 0 ]]; then
    echo "FAIL (exit ${exit_code})"
    ERRORS="${ERRORS}\n  FAIL: ${name}\n    Command: $*\n    Output: ${output}\n"
    FAILED=$((FAILED + 1))
  elif echo "${output}" | grep -qiF -- "${expected}"; then
    echo "OK"; PASSED=$((PASSED + 1))
  else
    echo "FAIL (expected '${expected}' not found)"
    ERRORS="${ERRORS}\n  FAIL: ${name}\n    Expected: ${expected}\n    Output: ${output}\n"
    FAILED=$((FAILED + 1))
  fi
}

skip_test() {
  echo "  $1... SKIP ($2)"
  SKIPPED=$((SKIPPED + 1))
}

# Guards: skip the rest of the file silently if a precondition is missing.
require_real_llm() {
  # On cloud, provider keys live server-side in the workspace, so the
  # local env doesn't need them. On opensource we need a key locally.
  if [[ "${SKIP_REAL_LLM:-0}" == "1" ]]; then
    skip_test "$(basename "$0")" "SKIP_REAL_LLM=1"
    summary; exit 0
  fi
  local backend="${DISTRI_BACKEND:-cloud}"
  if [[ "${backend}" == "opensource" \
        && -z "${OPENAI_API_KEY:-}" \
        && -z "${ANTHROPIC_API_KEY:-}" ]]; then
    skip_test "$(basename "$0")" "opensource backend with no provider key in env"
    summary; exit 0
  fi
}

require_cloud() {
  if [[ "${DISTRI_BACKEND:-opensource}" != "cloud" ]]; then
    skip_test "$(basename "$0")" "DISTRI_BACKEND != cloud"
    summary
    exit 0
  fi
}

require_server() {
  local url="${DISTRI_BASE_URL:-http://localhost:1341/v1}"
  local root="${url%/v1}"
  # /health and /healthz both 404 on some cloud configs but the API is
  # still up; treat *any* HTTP response (even 4xx) as "server reachable",
  # only `Connection refused` / DNS failure means down.
  local code
  code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 \
    "${root}/health" 2>/dev/null || echo "000")
  if [[ "${code}" == "000" ]]; then
    code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 \
      "${root}/healthz" 2>/dev/null || echo "000")
  fi
  if [[ "${code}" == "000" ]]; then
    code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 \
      "${url}/agents" 2>/dev/null || echo "000")
  fi
  if [[ "${code}" == "000" ]]; then
    skip_test "$(basename "$0")" "no server at ${url}"
    summary
    exit 0
  fi
}

require_binary() {
  if [[ ! -x "${DISTRI_BIN}" ]]; then
    echo "ERROR: ${DISTRI_BIN} not found. Build with: cargo build -p distri-cli"
    exit 2
  fi
}

push_test_agents() {
  "${DISTRI_BIN}" push "${INT_DIR}/agents" >/dev/null 2>&1 || true
}

summary() {
  local total=$((PASSED + FAILED))
  echo ""
  echo "  ${PASSED}/${total} passed, ${SKIPPED} skipped"
  if [[ ${FAILED} -gt 0 ]]; then
    echo ""
    echo "Failures:"
    echo -e "${ERRORS}"
    return 1
  fi
}
