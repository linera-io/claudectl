#!/bin/sh
# claudectl installer — downloads the latest release binary for your platform.
# Usage: curl -fsSL https://raw.githubusercontent.com/mercurialsolo/claudectl/main/install.sh | sh

set -e

REPO="mercurialsolo/claudectl"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

# Detect OS and architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Darwin)  OS_TARGET="apple-darwin" ;;
    Linux)   OS_TARGET="unknown-linux-musl" ;;
    *)       echo "Error: unsupported OS: $OS" >&2; exit 1 ;;
esac

case "$ARCH" in
    x86_64|amd64)   ARCH_TARGET="x86_64" ;;
    aarch64|arm64)   ARCH_TARGET="aarch64" ;;
    *)               echo "Error: unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

TARGET="${ARCH_TARGET}-${OS_TARGET}"
ARCHIVE="claudectl-${TARGET}.tar.gz"

# Get latest release tag
echo "Fetching latest release..."
LATEST=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')

if [ -z "$LATEST" ]; then
    echo "Error: could not determine latest release" >&2
    exit 1
fi

echo "Installing claudectl ${LATEST} for ${TARGET}..."

URL="https://github.com/${REPO}/releases/download/${LATEST}/${ARCHIVE}"
CHECKSUM_URL="${URL}.sha256"

# Download to temp directory
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

curl -fsSL -o "${TMP_DIR}/${ARCHIVE}" "$URL"

# Verify checksum if available
if curl -fsSL -o "${TMP_DIR}/checksum.sha256" "$CHECKSUM_URL" 2>/dev/null; then
    cd "$TMP_DIR"
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -c checksum.sha256
    elif command -v sha256sum >/dev/null 2>&1; then
        sha256sum -c checksum.sha256
    fi
    cd - >/dev/null
fi

# Extract and install
tar xzf "${TMP_DIR}/${ARCHIVE}" -C "$TMP_DIR"

if [ -w "$INSTALL_DIR" ]; then
    mv "${TMP_DIR}/claudectl" "${INSTALL_DIR}/claudectl"
else
    echo "Installing to ${INSTALL_DIR} (requires sudo)..."
    sudo mv "${TMP_DIR}/claudectl" "${INSTALL_DIR}/claudectl"
fi

chmod +x "${INSTALL_DIR}/claudectl"

echo "claudectl ${LATEST} installed to ${INSTALL_DIR}/claudectl"
echo "Run 'claudectl --help' to get started."
