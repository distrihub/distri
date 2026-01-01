#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

OUTPUT_DIR="${OUTPUT_DIR:-dist}"
PROJECT_NAME="${WRANGLER_PROJECT_NAME:-distri-samples}"
BRANCH="${BRANCH:-main}"
ENV_FILE="${ENV_FILE:-.env.production}"

# Load environment variables from file if it exists
if [[ -f "${ENV_FILE}" ]]; then
  echo "Loading environment variables from ${ENV_FILE}"
  set -a
  # shellcheck disable=SC1090
  source "${ENV_FILE}"
  set +a
fi

# Default production values if not already set
export VITE_DISTRI_API_URL="${VITE_DISTRI_API_URL:-https://api.distri.dev/v1}"
# Default demo project client ID
export VITE_DISTRI_CLIENT_ID="${VITE_DISTRI_CLIENT_ID:-dpc_79t2O3X3zL5mY1pQto84Bfl9bc7xKtdE}"

echo "Building maps-demo..."
pnpm run build

echo "Deploying to Cloudflare Pages (distri-samples-maps)..."
npx wrangler pages deploy "${OUTPUT_DIR}" --project-name="${PROJECT_NAME}" --branch="${BRANCH}"
