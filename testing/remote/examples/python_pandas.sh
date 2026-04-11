#!/usr/bin/env bash
# Full integration test: Python + pandas in a remote browsr container.
#
# Exercises:
#   - Container has python3 + pip
#   - Agent can install packages (pandas)
#   - Agent can write and execute a Python script
#   - Agent returns structured text output
#
# Prerequisites:
#   - distri-cloud running with SANDBOX_ENABLED=true
#   - browsr router + orchestrator running
#   - Container image includes python3, pip
#   - DISTRI_API_KEY set
#
# Usage:
#   source .env && bash testing/remote/examples/python_pandas.sh

set -euo pipefail

echo "=== Remote Python + pandas test ==="
echo "Server: $DISTRI_BASE_URL"
echo ""

distri run \
  --agent distri_runner \
  --remote \
  --task "Create a Python script that builds a pandas DataFrame with this sales data: \
Apple=150 units at \$1.20, Banana=230 at \$0.50, Cherry=89 at \$2.50, \
Dragonfruit=310 at \$4.00, Elderberry=175 at \$3.75. Calculate total revenue per \
product (units * price), find the top product by revenue, and print a plain-text bar \
chart of units sold using only dashes (no matplotlib). Return all results as text."

echo ""
echo "=== Python + pandas test passed ==="
