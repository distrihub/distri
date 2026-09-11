#!/usr/bin/env bash
set -euo pipefail

# Build distri-server for a given Rust target triple and emit a release
# tarball + sha256 sidecar in release-out/.
#
# Usage:  scripts/build-server-release.sh <target-triple>
# Example: scripts/build-server-release.sh aarch64-apple-darwin

TARGET="${1:?'target triple required, e.g. aarch64-apple-darwin'}"

case "$TARGET" in
    aarch64-apple-darwin)        PLAT="darwin-arm64" ;;
    x86_64-apple-darwin)         PLAT="darwin-x64" ;;
    x86_64-unknown-linux-gnu)    PLAT="linux-x64" ;;
    aarch64-unknown-linux-gnu)   PLAT="linux-arm64" ;;
    x86_64-pc-windows-msvc)      PLAT="windows-x64" ;;
    *) echo "unknown target $TARGET" >&2; exit 1 ;;
esac

# The server binary lives in the distri-server-cli package.
# Read its [package].version line.
VERSION=$(awk -F'"' '/^version/ { print $2; exit }' server/distri-server-cli/Cargo.toml)
if [[ -z "$VERSION" ]]; then
    echo "could not parse version from server/distri-server-cli/Cargo.toml" >&2
    exit 1
fi

# Linux targets cross-compile from macOS, and that needs three things a plain
# `cargo build --target` does not do:
#
#   cargo-zigbuild       supplies the cross linker
#   the .GLIBC suffix    pins the ABI floor, matching the Makefile
#   the vendored features compile libsqlite3 and OpenSSL rather than looking for
#                        aarch64 copies that are not on a Mac
#
# Without them this stops at "unable to find library" or at openssl-sys saying
# pkg-config will not answer for a foreign target. Native targets need none of
# it and link what is already installed.
GLIBC=2.31
case "$TARGET" in
    *-linux-gnu)
        echo "Building distri-server v${VERSION} for ${TARGET}.${GLIBC} (${PLAT})..."
        cargo zigbuild --release --target "${TARGET}.${GLIBC}" \
            -p distri-server-cli --bin distri-server \
            --features "sqlite_vendored,openssl_vendored"
        ;;
    *)
        echo "Building distri-server v${VERSION} for ${TARGET} (${PLAT})..."
        cargo build --release --target "$TARGET" \
            -p distri-server-cli --bin distri-server --features "sqlite"
        ;;
esac

OUT=release-out
mkdir -p "$OUT"

BIN_NAME="distri-server-${VERSION}-${PLAT}"
SOURCE_BIN="target/${TARGET}/release/distri-server"
case "$TARGET" in *windows*) SOURCE_BIN="${SOURCE_BIN}.exe"; BIN_NAME="${BIN_NAME}.exe" ;; esac

cp "$SOURCE_BIN" "${OUT}/${BIN_NAME}"
chmod +x "${OUT}/${BIN_NAME}"

TAR="${BIN_NAME}.tar.gz"
case "$TAR" in *.exe.tar.gz) TAR="${BIN_NAME%.exe}.tar.gz" ;; esac

tar -czf "${OUT}/${TAR}" -C "${OUT}" "${BIN_NAME}"

# sha256 sidecar (handle both shasum and sha256sum)
if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${OUT}/${TAR}" | awk '{print $1}' > "${OUT}/${TAR}.sha256"
else
    sha256sum "${OUT}/${TAR}" | awk '{print $1}' > "${OUT}/${TAR}.sha256"
fi

echo "Wrote ${OUT}/${TAR}"
echo "SHA256: $(cat ${OUT}/${TAR}.sha256)"
