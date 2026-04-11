#!/usr/bin/env bash
# Demonstrate using --overrides instead of --remote.
#
# The --remote flag is shorthand for --overrides '{"remote":true}'.
# This example shows the explicit form.
#
# Usage:
#   source .env && bash testing/remote/examples/overrides.sh

set -euo pipefail

echo "=== Remote via --overrides flag ==="
echo "Server: $DISTRI_BASE_URL"
echo ""

distri run \
  --agent distri_runner \
  --task "say hello" \
  --overrides '{"remote":true}'

echo ""
echo "=== Overrides test passed ==="
