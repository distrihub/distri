#!/usr/bin/env bash
# Build CLI release artifacts and create a DRAFT GitHub release.
# Environment-agnostic — runs on dev machine or provisioner VM.
#
# Usage: scripts/publish.sh [version]
#
# Required env:
#   GITHUB_TOKEN   For `gh` and pushing the release branch.
#   GITHUB_REPO    org/repo where the draft release lands (e.g. heyzippy/zippy).
#
# Optional env:
#   SKIP_TESTS=1   Skip unit tests.
#
# Per-repo overrides via scripts/publish.env (sourced if present):
#   VERSION_CRATE, BINARY_NAME, TARBALL_DIR, BUILD_CMD, TEST_CMD

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

# -- Defaults ----------------------------------------------------------------
VERSION_CRATE=""
BINARY_NAME=""
TARBALL_DIR="releases"
BUILD_CMD="make build-all && make release-tarballs"
TEST_CMD="cargo test --workspace"

# -- Load per-repo overrides --------------------------------------------------
if [[ -f "${SCRIPT_DIR}/publish.env" ]]; then
  source "${SCRIPT_DIR}/publish.env"
fi

# -- Validate -----------------------------------------------------------------
if [[ -z "${VERSION_CRATE}" ]]; then
  echo "ERROR: VERSION_CRATE not set (define in scripts/publish.env)" >&2
  exit 1
fi
if [[ -z "${BINARY_NAME}" ]]; then
  echo "ERROR: BINARY_NAME not set (define in scripts/publish.env)" >&2
  exit 1
fi
if [[ -z "${GITHUB_TOKEN:-}" ]]; then
  echo "ERROR: GITHUB_TOKEN env var required" >&2
  exit 1
fi
if [[ -z "${GITHUB_REPO:-}" ]]; then
  echo "ERROR: GITHUB_REPO env var required (e.g. heyzippy/zippy)" >&2
  exit 1
fi
for cmd in git gh make cargo; do
  if ! command -v "${cmd}" >/dev/null 2>&1; then
    echo "ERROR: required tool missing: ${cmd}" >&2
    exit 1
  fi
done

EXPLICIT_VERSION="${1:-}"
EXPLICIT_VERSION="${EXPLICIT_VERSION#v}"
CARGO_TOML="${VERSION_CRATE}/Cargo.toml"
if [[ ! -f "${CARGO_TOML}" ]]; then
  echo "ERROR: ${CARGO_TOML} not found" >&2
  exit 1
fi

# -- Compute version ---------------------------------------------------------
CURRENT_VERSION=$(grep '^version' "${CARGO_TOML}" | head -1 | sed 's/.*"\(.*\)".*/\1/')
echo "[+] Current version: ${CURRENT_VERSION}"

if [[ -n "${EXPLICIT_VERSION}" ]]; then
  NEW_VERSION="${EXPLICIT_VERSION}"
else
  IFS='.' read -r MAJOR MINOR PATCH <<<"${CURRENT_VERSION}"
  NEW_VERSION="${MAJOR}.${MINOR}.$((PATCH + 1))"
fi
echo "[+] New version: ${NEW_VERSION}"

# -- Create release branch ---------------------------------------------------
RELEASE_BRANCH="release/v${NEW_VERSION}"
CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [[ "${CURRENT_BRANCH}" != "${RELEASE_BRANCH}" ]]; then
  git checkout -b "${RELEASE_BRANCH}" 2>/dev/null || git checkout "${RELEASE_BRANCH}"
fi
echo "[OK] On branch ${RELEASE_BRANCH}"

# -- Bump Cargo.toml ---------------------------------------------------------
if [[ "${CURRENT_VERSION}" != "${NEW_VERSION}" ]]; then
  # Portable sed (BSD + GNU): 1,/pat/{s/pat/repl/} — only replaces first occurrence.
  # GNU's 0,/pat/s//repl/ form is not supported on macOS BSD sed.
  sed "1,/^version = \"${CURRENT_VERSION}\"/{s/^version = \"${CURRENT_VERSION}\"/version = \"${NEW_VERSION}\"/;}" \
    "${CARGO_TOML}" > "${CARGO_TOML}.tmp" && mv "${CARGO_TOML}.tmp" "${CARGO_TOML}"
  echo "[OK] Bumped ${CARGO_TOML} to ${NEW_VERSION}"
fi

# -- Build --------------------------------------------------------------------
echo "[+] Cleaning target/"
rm -rf target
echo "[+] Building..."
eval "${BUILD_CMD}"
echo "[OK] Build complete"

# -- Test ---------------------------------------------------------------------
if [[ "${SKIP_TESTS:-0}" != "1" ]]; then
  echo "[+] Running tests: ${TEST_CMD}"
  eval "${TEST_CMD}"
  echo "[OK] Tests passed"
else
  echo "[!] SKIP_TESTS=1 — skipping tests"
fi

# -- Commit + tag + push ------------------------------------------------------
git add "${CARGO_TOML}"
if git diff --cached --quiet; then
  echo "[!] No changes to commit (already on a tagged release branch?)"
else
  git commit -m "chore: release v${NEW_VERSION}"
fi

if ! git tag --list | grep -qx "v${NEW_VERSION}"; then
  git tag "v${NEW_VERSION}"
fi

echo "[+] Pushing ${RELEASE_BRANCH} + v${NEW_VERSION}"
git push origin "${RELEASE_BRANCH}"
git push origin "v${NEW_VERSION}"

# -- Draft release ------------------------------------------------------------
echo "[+] Creating DRAFT release on ${GITHUB_REPO}..."
if ! gh release create "v${NEW_VERSION}" \
    --repo "${GITHUB_REPO}" \
    --title "v${NEW_VERSION}" \
    --generate-notes \
    --draft \
    --target "${RELEASE_BRANCH}" 2>/tmp/gh_release_err.$$; then
  if grep -qE "already exists|422" /tmp/gh_release_err.$$; then
    echo "  Release already exists, uploading assets..."
  else
    cat /tmp/gh_release_err.$$ >&2
    rm -f /tmp/gh_release_err.$$
    exit 1
  fi
fi
rm -f /tmp/gh_release_err.$$

# -- Upload tarballs ----------------------------------------------------------
shopt -s nullglob
uploaded=0
for tarball in "${TARBALL_DIR}/${NEW_VERSION}"/*.tar.gz "${TARBALL_DIR}/${NEW_VERSION}"/*.zip; do
  if [[ -f "${tarball}" ]]; then
    echo "  Uploading $(basename "${tarball}")"
    gh release upload "v${NEW_VERSION}" "${tarball}" --repo "${GITHUB_REPO}" --clobber
    uploaded=$((uploaded + 1))
  fi
done
shopt -u nullglob

if [[ "${uploaded}" -eq 0 ]]; then
  echo "ERROR: no tarballs found in ${TARBALL_DIR}/${NEW_VERSION}/" >&2
  exit 1
fi

echo ""
echo "[OK] DRAFT release v${NEW_VERSION} on ${GITHUB_REPO} (${uploaded} assets)"
echo "PUBLISH_VERSION=${NEW_VERSION}"
