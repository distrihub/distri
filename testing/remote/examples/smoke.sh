#!/usr/bin/env bash
# Minimal remote execution smoke test.
#
# Verifies the remote path works end-to-end.
# Expected: exits in ~2-5 seconds with exit code 0.
#
# Prerequisites:
#   - distri-cloud running with SANDBOX_ENABLED=true
#   - browsr router + orchestrator running
#   - DISTRI_API_KEY set
#
# Usage:
#   source .env && bash testing/remote/examples/smoke.sh

set -euo pipefail

echo "=== Remote smoke test ==="
echo "Server: $DISTRI_BASE_URL"
echo ""

distri run \
  --agent distri_runner \
  --task "say hello" \
  --remote

echo ""
echo "=== Smoke test passed ==="
