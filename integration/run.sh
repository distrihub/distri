#!/usr/bin/env bash
# Top-level integration runner. Selects suites by flag.
#
# Usage:
#   ./integration/run.sh                # everything available
#   ./integration/run.sh --mock-only    # skip real-llm + cloud-only
#   ./integration/run.sh --real-only    # skip mock-only
#   ./integration/run.sh --cloud-only   # only cloud/
#   ./integration/run.sh --opensource-only
#   ./integration/run.sh --cli          # cli/ only
#   ./integration/run.sh --api          # api/ only

set -euo pipefail

INT_DIR="$(cd "$(dirname "$0")" && pwd)"

MODE=all
case "${1:-}" in
  --mock-only)        MODE=mock ;;
  --real-only)        MODE=real ;;
  --cloud-only)       MODE=cloud ;;
  --opensource-only)  MODE=opensource ;;
  --cli)              MODE=cli ;;
  --api)              MODE=api ;;
  --help|-h)
    sed -n '2,12p' "$0"
    exit 0
    ;;
esac

run_dir() {
  local dir="$1"
  [[ -d "${dir}" ]] || return 0
  local found=0
  for t in "${dir}"/test_*.sh; do
    [[ -f "${t}" ]] || continue
    found=1
    echo ""
    echo "==> ${t#${INT_DIR}/}"
    bash "${t}" || true
  done
  return 0
}

case "${MODE}" in
  all)
    run_dir "${INT_DIR}/cli"
    run_dir "${INT_DIR}/api"
    run_dir "${INT_DIR}/mock-llm"
    run_dir "${INT_DIR}/real-llm"
    run_dir "${INT_DIR}/${DISTRI_BACKEND:-opensource}"
    ;;
  mock)
    run_dir "${INT_DIR}/cli"
    run_dir "${INT_DIR}/api"
    run_dir "${INT_DIR}/mock-llm"
    ;;
  real)        run_dir "${INT_DIR}/real-llm" ;;
  cloud)       run_dir "${INT_DIR}/cloud" ;;
  opensource)  run_dir "${INT_DIR}/opensource" ;;
  cli)         run_dir "${INT_DIR}/cli" ;;
  api)         run_dir "${INT_DIR}/api" ;;
esac
