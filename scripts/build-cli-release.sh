#!/usr/bin/env bash
set -euo pipefail

# Build distri-cli for a given Rust target triple and emit a release tarball.
#
# Usage:  scripts/build-cli-release.sh <target-triple>

TARGET="${1:?'target triple required, e.g. aarch64-apple-darwin'}"

case "$TARGET" in
    aarch64-apple-darwin)        PLAT="darwin-arm64" ;;
    x86_64-apple-darwin)         PLAT="darwin-x64" ;;
    x86_64-unknown-linux-gnu)    PLAT="linux-x64" ;;
    aarch64-unknown-linux-gnu)   PLAT="linux-arm64" ;;
    x86_64-pc-windows-msvc)      PLAT="windows-x64" ;;
    *) echo "unknown target $TARGET" >&2; exit 1 ;;
esac

VERSION=$(awk -F'"' '/^version/ { print $2; exit }' distri-cli/Cargo.toml)
if [[ -z "$VERSION" ]]; then
    echo "could not parse version from distri-cli/Cargo.toml" >&2
    exit 1
fi

echo "Building distri-cli v${VERSION} for ${TARGET} (${PLAT})..."
cargo build --release --target "$TARGET" -p distri-cli

OUT=release-out
mkdir -p "$OUT"

# The CLI binary is named `distri` per Cargo.toml [[bin]] section.
SOURCE_BIN="target/${TARGET}/release/distri"
PACKAGE_DIR="distri-cli-${VERSION}-${PLAT}"
PACKAGE_BIN="distri"

case "$TARGET" in *windows*) SOURCE_BIN="${SOURCE_BIN}.exe"; PACKAGE_BIN="distri.exe" ;; esac

# Stage the binary inside a per-version dir so the tarball extracts to
# distri-cli-<ver>-<plat>/distri (avoids polluting cwd).
STAGE="${OUT}/${PACKAGE_DIR}"
mkdir -p "$STAGE"
cp "$SOURCE_BIN" "${STAGE}/${PACKAGE_BIN}"
chmod +x "${STAGE}/${PACKAGE_BIN}"

TAR="${PACKAGE_DIR}.tar.gz"
tar -czf "${OUT}/${TAR}" -C "${OUT}" "${PACKAGE_DIR}"

if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${OUT}/${TAR}" | awk '{print $1}' > "${OUT}/${TAR}.sha256"
else
    sha256sum "${OUT}/${TAR}" | awk '{print $1}' > "${OUT}/${TAR}.sha256"
fi

# Tidy up the stage dir.
rm -rf "$STAGE"

echo "Wrote ${OUT}/${TAR}"
echo "SHA256: $(cat ${OUT}/${TAR}.sha256)"
