#!/bin/sh
set -eu

REPO="REPO_PLACEHOLDER"
BASE_URL="BASE_URL_PLACEHOLDER"
BINARY="${1:-betcode}"
INSTALL_DIR="${BETCODE_INSTALL_DIR:-/usr/local/bin}"

# Detect OS
OS="$(uname -s)"
case "$OS" in
  Linux*)  OS_SUFFIX="linux" ;;
  Darwin*) OS_SUFFIX="darwin" ;;
  *)       echo "Error: unsupported OS: $OS" >&2; exit 1 ;;
esac

# Detect arch
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64)   ARCH_SUFFIX="amd64" ;;
  aarch64|arm64)   ARCH_SUFFIX="arm64" ;;
  *)               echo "Error: unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

# Server-only binaries
case "$BINARY" in
  betcode-relay|betcode-setup)
    if [ "$OS_SUFFIX" != "linux" ]; then
      echo "Error: $BINARY is only available on Linux" >&2; exit 1
    fi ;;
esac

PLATFORM="${OS_SUFFIX}-${ARCH_SUFFIX}"
URL="https://github.com/${REPO}/releases/latest/download/${BINARY}-${PLATFORM}.tar.gz"

echo "Installing ${BINARY} (${PLATFORM})..."

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

curl -fsSL "$URL" -o "${TMP}/${BINARY}.tar.gz"
tar xzf "${TMP}/${BINARY}.tar.gz" -C "$TMP"

if [ -w "$INSTALL_DIR" ]; then
  mv "${TMP}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
else
  echo "Installing to ${INSTALL_DIR} (requires sudo)..."
  sudo mv "${TMP}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
fi

chmod +x "${INSTALL_DIR}/${BINARY}"
echo "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"
"${INSTALL_DIR}/${BINARY}" --version 2>/dev/null || true
