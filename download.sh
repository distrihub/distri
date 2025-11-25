#!/usr/bin/env sh

set -euf

REPO=${DISTRI_REPO:-distrihub/distri}

log() {
  printf "distri-install: %s\n" "$*" >&2
}

fatal() {
  log "$1"
  exit 1
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    fatal "missing required dependency: $1"
  fi
}

require_cmd curl
require_cmd tar
require_cmd uname
require_cmd mktemp
require_cmd find
require_cmd install

OS=$(uname -s 2>/dev/null || echo unknown)
case "$OS" in
  Darwin) PLATFORM="darwin" ;;
  Linux) PLATFORM="linux" ;;
  *) fatal "unsupported OS: $OS. Only macOS and Linux are supported." ;;
esac

ARCH_RAW=$(uname -m 2>/dev/null || echo unknown)
case "$ARCH_RAW" in
  x86_64 | amd64) ARCH="x86_64" ;;
  arm64 | aarch64) ARCH="arm64" ;;
  *) fatal "unsupported architecture: $ARCH_RAW. Only x86_64 and arm64 are supported." ;;
esac

VERSION=${DISTRI_VERSION:-latest}
ASSET="distri-${PLATFORM}-${ARCH}.tar.gz"

if [ "$VERSION" = "latest" ]; then
  TAG_LABEL="latest"
  DOWNLOAD_URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"
else
  case "$VERSION" in
    v*) TAG="$VERSION" ;;
    *) TAG="v$VERSION" ;;
  esac
  TAG_LABEL="$TAG"
  DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${TAG}/${ASSET}"
fi

choose_install_dir() {
  if [ -n "${DISTRI_INSTALL_DIR:-}" ]; then
    printf "%s\n" "$DISTRI_INSTALL_DIR"
    return
  fi

  if [ "$(id -u 2>/dev/null || echo 1)" -eq 0 ]; then
    printf "/usr/local/bin\n"
    return
  fi

  if [ -d "/usr/local/bin" ] && [ -w "/usr/local/bin" ]; then
    printf "/usr/local/bin\n"
    return
  fi

  if [ ! -d "/usr/local/bin" ]; then
    if mkdir -p "/usr/local/bin" 2>/dev/null; then
      printf "/usr/local/bin\n"
      return
    fi
  fi

  printf "%s\n" "$HOME/.local/bin"
}

INSTALL_DIR=$(choose_install_dir)
mkdir -p "$INSTALL_DIR"

TMPDIR=$(mktemp -d 2>/dev/null || mktemp -d -t distri-install)
trap 'rm -rf "$TMPDIR"' EXIT INT TERM
TARBALL="$TMPDIR/$ASSET"

log "Installing Distri (${TAG_LABEL}) for ${PLATFORM}/${ARCH} into ${INSTALL_DIR}"
log "Downloading ${DOWNLOAD_URL}"

if ! curl -fL "$DOWNLOAD_URL" -o "$TARBALL"; then
  fatal "download failed. If you set DISTRI_VERSION, ensure that release exists for ${PLATFORM}/${ARCH}."
fi

tar -xzf "$TARBALL" -C "$TMPDIR"
BIN_PATH=$(find "$TMPDIR" -type f -name "distri" | head -n 1)

if [ -z "$BIN_PATH" ]; then
  fatal "distri binary not found in downloaded archive."
fi

install -m 0755 "$BIN_PATH" "$INSTALL_DIR/distri"

if ! printf "%s" "$PATH" | tr ":" "\n" | grep -qx "$INSTALL_DIR"; then
  log "added distri to ${INSTALL_DIR}, but that directory is not on your PATH."
  log "Add the following line to your shell profile:"
  log "  export PATH=\"$INSTALL_DIR:\$PATH\""
fi

log "Distri installed successfully. Run 'distri --version' to verify."
