#!/usr/bin/env bash
# Demonstrate using --overrides instead of --remote.
#
# The --remote flag is shorthand for --overrides '{"remote":true}'.
# This example shows the explicit form.
#
# Usage:
#   source .env && bash testing/remote/examples/overrides.sh

set -euo pipefail

BASE_URL="${DISTRI_SMOKE_BASE_URL:-http://localhost:1341}"

echo "=== Remote via --overrides flag ==="
echo "Server: $BASE_URL"
echo ""

distri run \
  --base-url "$BASE_URL" \
  --agent distri_runner \
  --task "say hello" \
  --overrides '{"remote":true}'

echo ""
echo "=== Overrides test passed ==="
